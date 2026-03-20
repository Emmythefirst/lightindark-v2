use anchor_lang::prelude::*;
use anchor_spl::token::{self, Burn, Token, TokenAccount, Transfer};
use ephemeral_rollups_sdk::cpi::{delegate_account, DelegateAccounts, DelegateConfig};
use ephemeral_rollups_sdk::ephem::commit_and_undelegate_accounts;

declare_id!("6CvCAte9SsfB34yWcpshY3Do2d7VqkLfHCRbHBsv6zar");

// ============================================================
//  CONSTANTS
// ============================================================

const SEASON_SEED: &[u8] = b"season";
const ENTRY_SEED: &[u8] = b"entry";
const RUN_SEED: &[u8] = b"run";
const VAULT_SEED: &[u8] = b"vault";

// Reward splits (out of 100)
const WINNER_1_PCT: u64 = 40;
const WINNER_2_PCT: u64 = 20;
const WINNER_3_PCT: u64 = 10;
const ROLLOVER_PCT: u64 = 15;
const CREATOR_PCT: u64 = 5;
const BURN_PCT: u64 = 10;

// ============================================================
//  PROGRAM
// ============================================================

#[program]
pub mod lightindark_v2 {
    use super::*;

    // --------------------------------------------------------
    //  1. ADMIN: Initialize a new season
    // --------------------------------------------------------
    pub fn initialize_season(
        ctx: Context<InitializeSeason>,
        season_id: u32,
        stake_amount: u64,
        registration_start: i64,
        registration_end: i64,
        season_end: i64,
    ) -> Result<()> {
        let season = &mut ctx.accounts.season_config;
        season.season_id = season_id;
        season.authority = ctx.accounts.authority.key();
        season.creator = ctx.accounts.authority.key(); // creator = deployer by default
        season.stake_amount = stake_amount;
        season.registration_start = registration_start;
        season.registration_end = registration_end;
        season.season_end = season_end;
        season.prize_pool = 0;
        season.player_count = 0;
        season.status = SeasonStatus::Registration;
        // Top 3 slots — empty until runs are committed
        season.top_players = [Pubkey::default(); 3];
        season.top_times = [u64::MAX; 3];
        season.top_deaths = [u32::MAX; 3];
        season.bump = ctx.bumps.season_config;
        msg!("Season {} initialized", season_id);
        Ok(())
    }

    // --------------------------------------------------------
    //  2. PLAYER: Stake tokens to enter a season
    // --------------------------------------------------------
    pub fn stake_for_season(
        ctx: Context<StakeForSeason>,
        season_id: u32,
    ) -> Result<()> {
        let season = &mut ctx.accounts.season_config;
        let clock = Clock::get()?;

        require!(
            season.status == SeasonStatus::Registration,
            LightInDarkError::RegistrationClosed
        );
        require!(
            clock.unix_timestamp >= season.registration_start
                && clock.unix_timestamp <= season.registration_end,
            LightInDarkError::OutsideRegistrationWindow
        );

        let stake_amount = season.stake_amount;

        // Transfer stake from player ATA to vault
        let cpi_accounts = Transfer {
            from: ctx.accounts.player_token_account.to_account_info(),
            to: ctx.accounts.vault.to_account_info(),
            authority: ctx.accounts.player.to_account_info(),
        };
        token::transfer(
            CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts),
            stake_amount,
        )?;

        // Create player entry
        let entry = &mut ctx.accounts.player_entry;
        entry.player = ctx.accounts.player.key();
        entry.season_id = season_id;
        entry.staked_amount = stake_amount;
        entry.is_eligible = true;
        entry.best_time_ms = u64::MAX;
        entry.best_death_count = u32::MAX;
        entry.run_count = 0;
        entry.bump = ctx.bumps.player_entry;

        season.prize_pool = season.prize_pool.checked_add(stake_amount).unwrap();
        season.player_count = season.player_count.checked_add(1).unwrap();

        msg!(
            "Player {} staked {} tokens for season {}",
            ctx.accounts.player.key(),
            stake_amount,
            season_id
        );
        Ok(())
    }

    // --------------------------------------------------------
    //  3. PLAYER: Check eligibility (read-only helper)
    // --------------------------------------------------------
    pub fn check_eligibility(
        ctx: Context<CheckEligibility>,
        _season_id: u32,
    ) -> Result<bool> {
        let entry = &ctx.accounts.player_entry;
        msg!("Player eligible: {}", entry.is_eligible);
        Ok(entry.is_eligible)
    }

    // --------------------------------------------------------
    //  4. PLAYER: Start a competitive run — delegates ActiveRun to ER
    // --------------------------------------------------------
    pub fn start_competitive_run(
        ctx: Context<StartCompetitiveRun>,
        season_id: u32,
        level_id: u8,
    ) -> Result<()> {
        let entry = &ctx.accounts.player_entry;
        require!(entry.is_eligible, LightInDarkError::NotEligible);

        let season = &ctx.accounts.season_config;
        require!(
            season.status == SeasonStatus::Active,
            LightInDarkError::SeasonNotActive
        );

        let run = &mut ctx.accounts.active_run;
        run.player = ctx.accounts.player.key();
        run.season_id = season_id;
        run.level_id = level_id;
        run.start_time = Clock::get()?.unix_timestamp;
        run.elapsed_ms = 0;
        run.death_count = 0;
        run.is_finished = false;
        run.bump = ctx.bumps.active_run;

        // Delegate ActiveRun PDA to Ephemeral Rollup
        let player_key = ctx.accounts.player.key();
        let pda_seeds: &[&[u8]] = &[RUN_SEED, player_key.as_ref(), &[run.bump]];

        delegate_account(
            DelegateAccounts {
                payer: &ctx.accounts.player.to_account_info(),
                pda: &ctx.accounts.active_run.to_account_info(),
                owner_program: &ctx.accounts.owner_program.to_account_info(),
                buffer: &ctx.accounts.buffer.to_account_info(),
                delegation_record: &ctx.accounts.delegation_record.to_account_info(),
                delegation_metadata: &ctx.accounts.delegation_metadata.to_account_info(),
                delegation_program: &ctx.accounts.delegation_program.to_account_info(),
                system_program: &ctx.accounts.system_program.to_account_info(),
            },
            pda_seeds,
            DelegateConfig {
                commit_frequency_ms: 3_000,
                validator: None,
            },
        )?;

        msg!(
            "Competitive run started: player={} season={} level={}",
            player_key,
            season_id,
            level_id
        );
        Ok(())
    }

    // --------------------------------------------------------
    //  5. ER: Update run state (called from ER layer, gasless)
    // --------------------------------------------------------
    pub fn update_run(
        ctx: Context<UpdateRun>,
        elapsed_ms: u64,
        death_count: u32,
    ) -> Result<()> {
        let run = &mut ctx.accounts.active_run;
        require!(!run.is_finished, LightInDarkError::RunAlreadyFinished);
        require!(
            run.player == ctx.accounts.player.key(),
            LightInDarkError::Unauthorized
        );
        run.elapsed_ms = elapsed_ms;
        run.death_count = death_count;
        Ok(())
    }

    // --------------------------------------------------------
    //  6. PLAYER: Commit final run — undelegates from ER, updates top 3
    // --------------------------------------------------------
    pub fn commit_run(
        ctx: Context<CommitRun>,
        final_time_ms: u64,
        final_death_count: u32,
    ) -> Result<()> {
        let run = &mut ctx.accounts.active_run;
        require!(!run.is_finished, LightInDarkError::RunAlreadyFinished);
        require!(
            run.player == ctx.accounts.player.key(),
            LightInDarkError::Unauthorized
        );

        run.elapsed_ms = final_time_ms;
        run.death_count = final_death_count;
        run.is_finished = true;

        // Update player personal best
        let entry = &mut ctx.accounts.player_entry;
        let is_better = final_time_ms < entry.best_time_ms
            || (final_time_ms == entry.best_time_ms
                && final_death_count < entry.best_death_count);
        if is_better {
            entry.best_time_ms = final_time_ms;
            entry.best_death_count = final_death_count;
        }
        entry.run_count = entry.run_count.checked_add(1).unwrap();

        // ── Update season top 3 on-chain ─────────────────────
        // This is what makes distribute_season_rewards permissionless.
        // The program always knows the live top 3 — no admin input needed.
        if is_better {
            let player_key = ctx.accounts.player.key();
            let season = &mut ctx.accounts.season_config;

            // Check if player is already in top 3 — update their slot
            let mut existing_slot: Option<usize> = None;
            for i in 0..3 {
                if season.top_players[i] == player_key {
                    existing_slot = Some(i);
                    break;
                }
            }

            if let Some(slot) = existing_slot {
                season.top_times[slot] = final_time_ms;
                season.top_deaths[slot] = final_death_count;
            } else {
                // Find the worst current entry in top 3
                let mut worst_idx = 0;
                for i in 1..3 {
                    let current_worse = season.top_times[i] > season.top_times[worst_idx]
                        || (season.top_times[i] == season.top_times[worst_idx]
                            && season.top_deaths[i] > season.top_deaths[worst_idx]);
                    if current_worse {
                        worst_idx = i;
                    }
                }
                // Replace if new run beats the worst slot
                let beats_worst = final_time_ms < season.top_times[worst_idx]
                    || (final_time_ms == season.top_times[worst_idx]
                        && final_death_count < season.top_deaths[worst_idx]);
                if beats_worst {
                    season.top_players[worst_idx] = player_key;
                    season.top_times[worst_idx] = final_time_ms;
                    season.top_deaths[worst_idx] = final_death_count;
                }
            }

            // Re-sort top 3: fastest time first, death count as tiebreaker
            for i in 0..2 {
                for j in (i + 1)..3 {
                    let should_swap = season.top_times[i] > season.top_times[j]
                        || (season.top_times[i] == season.top_times[j]
                            && season.top_deaths[i] > season.top_deaths[j]);
                    if should_swap {
                        season.top_players.swap(i, j);
                        season.top_times.swap(i, j);
                        season.top_deaths.swap(i, j);
                    }
                }
            }
        }

        // Undelegate ActiveRun from ER — commits final state to L1
        commit_and_undelegate_accounts(
            &ctx.accounts.player.to_account_info(),
            vec![&ctx.accounts.active_run.to_account_info()],
            &ctx.accounts.magic_context.to_account_info(),
            &ctx.accounts.magic_program.to_account_info(),
        )?;

        msg!(
            "Run committed: player={} time={}ms deaths={} personal_best={}",
            ctx.accounts.player.key(),
            final_time_ms,
            final_death_count,
            is_better
        );
        Ok(())
    }

    // --------------------------------------------------------
    //  7. ADMIN: Activate season (open runs)
    // --------------------------------------------------------
    pub fn activate_season(ctx: Context<AdminAction>, _season_id: u32) -> Result<()> {
        let season = &mut ctx.accounts.season_config;
        require!(
            season.authority == ctx.accounts.authority.key(),
            LightInDarkError::Unauthorized
        );
        season.status = SeasonStatus::Active;
        msg!("Season {} activated", season.season_id);
        Ok(())
    }



    // --------------------------------------------------------
    //  ADMIN: Force-close a season with incompatible struct layout
    //  Used when the account cannot be deserialized (old struct).
    //  Returns lamports to authority.
    // --------------------------------------------------------
    pub fn force_close_season(ctx: Context<ForceCloseSeason>, _season_id: u32) -> Result<()> {
        msg!("Force closing season account");
        Ok(())
    }

    // --------------------------------------------------------
    //  ADMIN: Close a season (devnet utility / emergency reset)
    //  Closes both SeasonConfig and vault token account.
    //  Returns lamports to authority.
    // --------------------------------------------------------
    pub fn close_season(ctx: Context<CloseSeason>, season_id: u32) -> Result<()> {
        require!(
            ctx.accounts.season_config.authority == ctx.accounts.authority.key(),
            LightInDarkError::Unauthorized
        );

        // Close vault token account — transfer any remaining tokens back and reclaim rent
        let season_id_bytes = season_id.to_le_bytes();
        let vault_bump = ctx.bumps.vault;
        let seeds = &[VAULT_SEED, season_id_bytes.as_ref(), &[vault_bump]];
        let signer_seeds = &[&seeds[..]];

        // Transfer any remaining tokens back to authority token account
        let vault_balance = ctx.accounts.vault.amount;
        if vault_balance > 0 {
            token::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.vault.to_account_info(),
                        to: ctx.accounts.authority_token_account.to_account_info(),
                        authority: ctx.accounts.vault.to_account_info(),
                    },
                    signer_seeds,
                ),
                vault_balance,
            )?;
        }

        // Close the vault token account
        token::close_account(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                token::CloseAccount {
                    account: ctx.accounts.vault.to_account_info(),
                    destination: ctx.accounts.authority.to_account_info(),
                    authority: ctx.accounts.vault.to_account_info(),
                },
                signer_seeds,
            ),
        )?;

        msg!("Season {} closed", ctx.accounts.season_config.season_id);
        Ok(())
    }


    // --------------------------------------------------------
    //  ADMIN: Close vault token account (devnet utility)
    //  Used when SeasonConfig is already closed but vault remains.
    // --------------------------------------------------------
    pub fn close_vault(ctx: Context<CloseVault>, season_id: u32) -> Result<()> {
        let season_id_bytes = season_id.to_le_bytes();
        let vault_bump = ctx.bumps.vault;
        let seeds = &[VAULT_SEED, season_id_bytes.as_ref(), &[vault_bump]];
        let signer_seeds = &[&seeds[..]];

        // Transfer any remaining tokens to authority
        let vault_balance = ctx.accounts.vault.amount;
        if vault_balance > 0 {
            token::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.vault.to_account_info(),
                        to: ctx.accounts.authority_token_account.to_account_info(),
                        authority: ctx.accounts.vault.to_account_info(),
                    },
                    signer_seeds,
                ),
                vault_balance,
            )?;
        }

        // Close vault and return rent to authority
        token::close_account(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                token::CloseAccount {
                    account: ctx.accounts.vault.to_account_info(),
                    destination: ctx.accounts.authority.to_account_info(),
                    authority: ctx.accounts.vault.to_account_info(),
                },
                signer_seeds,
            ),
        )?;

        msg!("Vault for season {} closed", season_id);
        Ok(())
    }

    // --------------------------------------------------------
    //  8. PERMISSIONLESS: Distribute season rewards
    //
    //  Anyone can call this after season_end timestamp has passed.
    //  No authority required. Winners are read from SeasonConfig.top_players
    //  which is maintained live by commit_run.
    //
    //  Splits:
    //    1st = 40% | 2nd = 20% | 3rd = 10%
    //    Rollover = 15% | Creator = 5% | Burn = 10%
    // --------------------------------------------------------
    pub fn distribute_season_rewards(
        ctx: Context<DistributeRewards>,
        season_id: u32,
    ) -> Result<()> {
        let season = &ctx.accounts.season_config;

        // Season must be over
        let clock = Clock::get()?;
        require!(
            clock.unix_timestamp > season.season_end,
            LightInDarkError::SeasonNotEnded
        );
        // Can only distribute once
        require!(
            season.status == SeasonStatus::Active,
            LightInDarkError::SeasonAlreadyDistributed
        );

        let pool = season.prize_pool;
        require!(pool > 0, LightInDarkError::EmptyPrizePool);

        // Compute splits
        let w1 = pool.checked_mul(WINNER_1_PCT).unwrap().checked_div(100).unwrap();
        let w2 = pool.checked_mul(WINNER_2_PCT).unwrap().checked_div(100).unwrap();
        let w3 = pool.checked_mul(WINNER_3_PCT).unwrap().checked_div(100).unwrap();
        let creator_cut = pool.checked_mul(CREATOR_PCT).unwrap().checked_div(100).unwrap();
        let burn_amt = pool.checked_mul(BURN_PCT).unwrap().checked_div(100).unwrap();
        let rollover = pool.checked_mul(ROLLOVER_PCT).unwrap().checked_div(100).unwrap();

        // Vault PDA signer seeds
        let season_id_bytes = season_id.to_le_bytes();
        let vault_bump = ctx.bumps.vault;
        let seeds = &[VAULT_SEED, season_id_bytes.as_ref(), &[vault_bump]];
        let signer_seeds = &[&seeds[..]];

        // Transfer to winner 1
        if season.top_players[0] != Pubkey::default() {
            token::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.vault.to_account_info(),
                        to: ctx.accounts.winner1_token_account.to_account_info(),
                        authority: ctx.accounts.vault.to_account_info(),
                    },
                    signer_seeds,
                ),
                w1,
            )?;
            msg!("Winner 1 ({}): {} tokens", season.top_players[0], w1);
        }

        // Transfer to winner 2
        if season.top_players[1] != Pubkey::default() {
            token::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.vault.to_account_info(),
                        to: ctx.accounts.winner2_token_account.to_account_info(),
                        authority: ctx.accounts.vault.to_account_info(),
                    },
                    signer_seeds,
                ),
                w2,
            )?;
            msg!("Winner 2 ({}): {} tokens", season.top_players[1], w2);
        }

        // Transfer to winner 3
        if season.top_players[2] != Pubkey::default() {
            token::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.vault.to_account_info(),
                        to: ctx.accounts.winner3_token_account.to_account_info(),
                        authority: ctx.accounts.vault.to_account_info(),
                    },
                    signer_seeds,
                ),
                w3,
            )?;
            msg!("Winner 3 ({}): {} tokens", season.top_players[2], w3);
        }

        // Transfer to creator
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault.to_account_info(),
                    to: ctx.accounts.creator_token_account.to_account_info(),
                    authority: ctx.accounts.vault.to_account_info(),
                },
                signer_seeds,
            ),
            creator_cut,
        )?;
        msg!("Creator: {} tokens", creator_cut);

        // Burn
        token::burn(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Burn {
                    mint: ctx.accounts.token_mint.to_account_info(),
                    from: ctx.accounts.vault.to_account_info(),
                    authority: ctx.accounts.vault.to_account_info(),
                },
                signer_seeds,
            ),
            burn_amt,
        )?;
        msg!("Burned: {} tokens", burn_amt);

        // Mark season ended, keep rollover in prize_pool for next season init
        let season = &mut ctx.accounts.season_config;
        season.status = SeasonStatus::Ended;
        season.prize_pool = rollover;

        msg!(
            "Season {} complete. Rollover to next season: {} tokens",
            season_id,
            rollover
        );
        Ok(())
    }
}

// ============================================================
//  ACCOUNT STRUCTS
// ============================================================

#[account]
pub struct SeasonConfig {
    pub season_id: u32,           // 4
    pub authority: Pubkey,        // 32
    pub creator: Pubkey,          // 32  — receives 5% cut at season end
    pub stake_amount: u64,        // 8
    pub registration_start: i64,  // 8
    pub registration_end: i64,    // 8
    pub season_end: i64,          // 8
    pub prize_pool: u64,          // 8
    pub player_count: u32,        // 4
    pub status: SeasonStatus,     // 1
    // Live top 3 — maintained by commit_run, read by distribute_season_rewards
    pub top_players: [Pubkey; 3], // 96
    pub top_times: [u64; 3],      // 24
    pub top_deaths: [u32; 3],     // 12
    pub bump: u8,                 // 1
}

impl SeasonConfig {
    pub const LEN: usize = 8   // discriminator
        + 4                    // season_id
        + 32                   // authority
        + 32                   // creator
        + 8                    // stake_amount
        + 8                    // registration_start
        + 8                    // registration_end
        + 8                    // season_end
        + 8                    // prize_pool
        + 4                    // player_count
        + 1                    // status
        + (32 * 3)             // top_players
        + (8 * 3)              // top_times
        + (4 * 3)              // top_deaths
        + 1;                   // bump
}

#[account]
pub struct PlayerEntry {
    pub player: Pubkey,          // 32
    pub season_id: u32,          // 4
    pub staked_amount: u64,      // 8
    pub is_eligible: bool,       // 1
    pub best_time_ms: u64,       // 8
    pub best_death_count: u32,   // 4
    pub run_count: u32,          // 4
    pub bump: u8,                // 1
}

impl PlayerEntry {
    pub const LEN: usize = 8 + 32 + 4 + 8 + 1 + 8 + 4 + 4 + 1;
}

#[account]
pub struct ActiveRun {
    pub player: Pubkey,    // 32
    pub season_id: u32,    // 4
    pub level_id: u8,      // 1
    pub start_time: i64,   // 8
    pub elapsed_ms: u64,   // 8
    pub death_count: u32,  // 4
    pub is_finished: bool, // 1
    pub bump: u8,          // 1
}

impl ActiveRun {
    pub const LEN: usize = 8 + 32 + 4 + 1 + 8 + 8 + 4 + 1 + 1;
}

// ============================================================
//  ENUMS
// ============================================================

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum SeasonStatus {
    Registration,
    Active,
    Ended,
}

// ============================================================
//  CONTEXTS
// ============================================================

#[derive(Accounts)]
#[instruction(season_id: u32)]
pub struct InitializeSeason<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = SeasonConfig::LEN,
        seeds = [SEASON_SEED, &season_id.to_le_bytes()],
        bump
    )]
    pub season_config: Account<'info, SeasonConfig>,

    // Vault token account initialized here so players can stake immediately
    #[account(
        init,
        payer = authority,
        token::mint = token_mint,
        token::authority = vault,
        seeds = [VAULT_SEED, &season_id.to_le_bytes()],
        bump
    )]
    pub vault: Account<'info, TokenAccount>,

    pub token_mint: Account<'info, anchor_spl::token::Mint>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(season_id: u32)]
pub struct StakeForSeason<'info> {
    #[account(mut)]
    pub player: Signer<'info>,

    #[account(
        mut,
        seeds = [SEASON_SEED, &season_id.to_le_bytes()],
        bump = season_config.bump
    )]
    pub season_config: Account<'info, SeasonConfig>,

    #[account(
        init,
        payer = player,
        space = PlayerEntry::LEN,
        seeds = [ENTRY_SEED, &season_id.to_le_bytes(), player.key().as_ref()],
        bump
    )]
    pub player_entry: Account<'info, PlayerEntry>,

    #[account(mut)]
    pub player_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [VAULT_SEED, &season_id.to_le_bytes()],
        bump
    )]
    pub vault: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(season_id: u32)]
pub struct CheckEligibility<'info> {
    pub player: Signer<'info>,

    #[account(
        seeds = [ENTRY_SEED, &season_id.to_le_bytes(), player.key().as_ref()],
        bump = player_entry.bump
    )]
    pub player_entry: Account<'info, PlayerEntry>,
}

#[derive(Accounts)]
#[instruction(season_id: u32)]
pub struct StartCompetitiveRun<'info> {
    #[account(mut)]
    pub player: Signer<'info>,

    #[account(
        seeds = [SEASON_SEED, &season_id.to_le_bytes()],
        bump = season_config.bump
    )]
    pub season_config: Account<'info, SeasonConfig>,

    #[account(
        seeds = [ENTRY_SEED, &season_id.to_le_bytes(), player.key().as_ref()],
        bump = player_entry.bump
    )]
    pub player_entry: Account<'info, PlayerEntry>,

    #[account(
        init,
        payer = player,
        space = ActiveRun::LEN,
        seeds = [RUN_SEED, player.key().as_ref()],
        bump
    )]
    pub active_run: Account<'info, ActiveRun>,

    /// CHECK: owner program for delegation
    pub owner_program: UncheckedAccount<'info>,
    /// CHECK: buffer PDA
    #[account(mut)]
    pub buffer: UncheckedAccount<'info>,
    /// CHECK: delegation record PDA
    #[account(mut)]
    pub delegation_record: UncheckedAccount<'info>,
    /// CHECK: delegation metadata PDA
    #[account(mut)]
    pub delegation_metadata: UncheckedAccount<'info>,
    /// CHECK: delegation program
    pub delegation_program: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdateRun<'info> {
    pub player: Signer<'info>,

    #[account(
        mut,
        seeds = [RUN_SEED, player.key().as_ref()],
        bump = active_run.bump
    )]
    pub active_run: Account<'info, ActiveRun>,
}

#[derive(Accounts)]
#[instruction(season_id: u32)]
pub struct CommitRun<'info> {
    #[account(mut)]
    pub player: Signer<'info>,

    #[account(
        mut,
        seeds = [RUN_SEED, player.key().as_ref()],
        bump = active_run.bump
    )]
    pub active_run: Account<'info, ActiveRun>,

    #[account(
        mut,
        seeds = [ENTRY_SEED, &season_id.to_le_bytes(), player.key().as_ref()],
        bump = player_entry.bump
    )]
    pub player_entry: Account<'info, PlayerEntry>,

    // Needed to update live top 3
    #[account(
        mut,
        seeds = [SEASON_SEED, &season_id.to_le_bytes()],
        bump = season_config.bump
    )]
    pub season_config: Account<'info, SeasonConfig>,

    /// CHECK: magic context for ER undelegation
    #[account(mut)]
    pub magic_context: UncheckedAccount<'info>,
    /// CHECK: magic program
    pub magic_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
#[instruction(season_id: u32)]
pub struct AdminAction<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [SEASON_SEED, &season_id.to_le_bytes()],
        bump = season_config.bump
    )]
    pub season_config: Account<'info, SeasonConfig>,
}

#[derive(Accounts)]
#[instruction(season_id: u32)]
pub struct ForceCloseSeason<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    /// CHECK: skip deserialization — account may have incompatible layout
    #[account(
        mut,
        seeds = [SEASON_SEED, &season_id.to_le_bytes()],
        bump,
        owner = crate::ID,
    )]
    pub season_config: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(season_id: u32)]
pub struct CloseSeason<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        close = authority,
        seeds = [SEASON_SEED, &season_id.to_le_bytes()],
        bump = season_config.bump
    )]
    pub season_config: Account<'info, SeasonConfig>,

    #[account(
        mut,
        seeds = [VAULT_SEED, &season_id.to_le_bytes()],
        bump
    )]
    pub vault: Account<'info, TokenAccount>,

    #[account(mut)]
    pub authority_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(season_id: u32)]
pub struct CloseVault<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [VAULT_SEED, &season_id.to_le_bytes()],
        bump
    )]
    pub vault: Account<'info, TokenAccount>,

    #[account(mut)]
    pub authority_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
#[instruction(season_id: u32)]
pub struct DistributeRewards<'info> {
    // No authority required — anyone can call after season_end
    #[account(mut)]
    pub caller: Signer<'info>,

    #[account(
        mut,
        seeds = [SEASON_SEED, &season_id.to_le_bytes()],
        bump = season_config.bump
    )]
    pub season_config: Box<Account<'info, SeasonConfig>>,

    #[account(
        mut,
        seeds = [VAULT_SEED, &season_id.to_le_bytes()],
        bump
    )]
    pub vault: Box<Account<'info, TokenAccount>>,

    /// CHECK: token mint for burn instruction
    #[account(mut)]
    pub token_mint: UncheckedAccount<'info>,

    // Winner token accounts — constraint validates against top_players stored on-chain
    #[account(
        mut,
        constraint = winner1_token_account.owner == season_config.top_players[0]
            @ LightInDarkError::WrongWinnerAccount
    )]
    pub winner1_token_account: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = winner2_token_account.owner == season_config.top_players[1]
            @ LightInDarkError::WrongWinnerAccount
    )]
    pub winner2_token_account: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = winner3_token_account.owner == season_config.top_players[2]
            @ LightInDarkError::WrongWinnerAccount
    )]
    pub winner3_token_account: Box<Account<'info, TokenAccount>>,

    // Creator token account — validated against season_config.creator
    #[account(
        mut,
        constraint = creator_token_account.owner == season_config.creator
            @ LightInDarkError::WrongCreatorAccount
    )]
    pub creator_token_account: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
}

// ============================================================
//  ERRORS
// ============================================================

#[error_code]
pub enum LightInDarkError {
    #[msg("Registration is closed")]
    RegistrationClosed,
    #[msg("Outside registration window")]
    OutsideRegistrationWindow,
    #[msg("Player is not eligible for this season")]
    NotEligible,
    #[msg("Season is not active")]
    SeasonNotActive,
    #[msg("Run is already finished")]
    RunAlreadyFinished,
    #[msg("Unauthorized")]
    Unauthorized,
    #[msg("Too many winners provided")]
    TooManyWinners,
    #[msg("Season has not ended yet — wait until season_end timestamp")]
    SeasonNotEnded,
    #[msg("Season rewards have already been distributed")]
    SeasonAlreadyDistributed,
    #[msg("Prize pool is empty")]
    EmptyPrizePool,
    #[msg("Wrong winner token account — must match on-chain top_players")]
    WrongWinnerAccount,
    #[msg("Wrong creator token account")]
    WrongCreatorAccount,
}