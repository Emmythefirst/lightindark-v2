use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
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

// Reward splits (basis points out of 100)
const WINNER_1_PCT: u64 = 40; // 40% of prize pool
const WINNER_2_PCT: u64 = 20; // 20% of prize pool
const WINNER_3_PCT: u64 = 10; // 10% of prize pool
const ROLLOVER_PCT: u64 = 15; // 15% rolled to next season
const CREATOR_PCT: u64 = 5;   // 5% to creator
const BURN_PCT: u64 = 10;     // 10% burned

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
        season.stake_amount = stake_amount;
        season.registration_start = registration_start;
        season.registration_end = registration_end;
        season_end_field(season, season_end);
        season.prize_pool = 0;
        season.player_count = 0;
        season.status = SeasonStatus::Registration;
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

        // Transfer stake from player to vault
        let stake_amount = season.stake_amount;
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

        msg!("Player {} staked {} tokens for season {}", ctx.accounts.player.key(), stake_amount, season_id);
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

        // Initialize the active run account
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
        let pda_seeds: &[&[u8]] = &[
            RUN_SEED,
            player_key.as_ref(),
            &[run.bump],
        ];
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

        msg!("Competitive run started for player {} season {} level {}", player_key, season_id, level_id);
        Ok(())
    }

    // --------------------------------------------------------
    //  5. ER: Update run state (called from ER, gasless)
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
    //  6. PLAYER: Commit final run result — undelegates from ER
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

        // Update player entry with best result
        let entry = &mut ctx.accounts.player_entry;
        let is_better = final_time_ms < entry.best_time_ms
            || (final_time_ms == entry.best_time_ms && final_death_count < entry.best_death_count);
        if is_better {
            entry.best_time_ms = final_time_ms;
            entry.best_death_count = final_death_count;
        }
        entry.run_count = entry.run_count.checked_add(1).unwrap();

        // Undelegate ActiveRun from ER — commits final state to L1
        commit_and_undelegate_accounts(
            &ctx.accounts.player.to_account_info(),
            vec![&ctx.accounts.active_run.to_account_info()],
            &ctx.accounts.magic_context.to_account_info(),
            &ctx.accounts.magic_program.to_account_info(),
        )?;

        msg!(
            "Run committed: player={} time={}ms deaths={} best={}",
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
    //  8. ADMIN: Distribute season rewards
    // --------------------------------------------------------
    pub fn distribute_season_rewards(
        ctx: Context<DistributeRewards>,
        _season_id: u32,
        winners: Vec<Pubkey>,
    ) -> Result<()> {
        let season = &mut ctx.accounts.season_config;
        require!(
            season.authority == ctx.accounts.authority.key(),
            LightInDarkError::Unauthorized
        );
        require!(
            season.status == SeasonStatus::Active,
            LightInDarkError::SeasonNotActive
        );
        require!(winners.len() <= 3, LightInDarkError::TooManyWinners);

        let pool = season.prize_pool;
        let splits = [WINNER_1_PCT, WINNER_2_PCT, WINNER_3_PCT];

        // Pay winners from vault
        let season_id_bytes = season.season_id.to_le_bytes();
        let seeds = &[
            VAULT_SEED,
            season_id_bytes.as_ref(),
            &[ctx.bumps.vault],
        ];
        let signer_seeds = &[&seeds[..]];

        for (i, winner) in winners.iter().enumerate() {
            let amount = pool.checked_mul(splits[i]).unwrap().checked_div(100).unwrap();
            // Transfer handled via remaining_accounts in practice
            // Simplified: emit event for off-chain distribution trigger
            msg!("Winner {}: {} gets {} tokens", i + 1, winner, amount);
        }

        let rollover = pool.checked_mul(ROLLOVER_PCT).unwrap().checked_div(100).unwrap();
        let creator_cut = pool.checked_mul(CREATOR_PCT).unwrap().checked_div(100).unwrap();
        let burn_amount = pool.checked_mul(BURN_PCT).unwrap().checked_div(100).unwrap();

        msg!("Rollover: {} | Creator: {} | Burn: {}", rollover, creator_cut, burn_amount);

        season.status = SeasonStatus::Ended;
        season.prize_pool = rollover; // keep rollover in pool for next season
        Ok(())
    }
}

// ============================================================
//  HELPER
// ============================================================

fn season_end_field(season: &mut Account<SeasonConfig>, season_end: i64) {
    season.season_end = season_end;
}

// ============================================================
//  ACCOUNT STRUCTS
// ============================================================

#[account]
pub struct SeasonConfig {
    pub season_id: u32,          // 4
    pub authority: Pubkey,       // 32
    pub stake_amount: u64,       // 8
    pub registration_start: i64, // 8
    pub registration_end: i64,   // 8
    pub season_end: i64,         // 8
    pub prize_pool: u64,         // 8
    pub player_count: u32,       // 4
    pub status: SeasonStatus,    // 1
    pub bump: u8,                // 1
}

impl SeasonConfig {
    pub const LEN: usize = 8 + 4 + 32 + 8 + 8 + 8 + 8 + 8 + 4 + 1 + 1;
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

    /// CHECK: delegation program accounts
    pub owner_program: UncheckedAccount<'info>,
    /// CHECK: buffer
    #[account(mut)]
    pub buffer: UncheckedAccount<'info>,
    /// CHECK: delegation record
    #[account(mut)]
    pub delegation_record: UncheckedAccount<'info>,
    /// CHECK: delegation metadata
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
pub struct DistributeRewards<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
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
}
