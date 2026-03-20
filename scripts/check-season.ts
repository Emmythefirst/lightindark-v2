process.stdout.write("SCRIPT START\n");
import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { LightindarkV2 } from "../target/types/lightindark_v2";
import { PublicKey } from "@solana/web3.js";
import { getAssociatedTokenAddress, getAccount } from "@solana/spl-token";

const LID_MINT = new PublicKey("3TX7tdXJLnJ51aBRR3TkVocnyFaiyNhETK3CQFp3E6bf");

// ============================================================
//  check-season.ts
//
//  Reads full season state from chain:
//    - SeasonConfig (status, pool, top 3, timestamps)
//    - Vault LID balance
//    - Player LID balance (your wallet)
//    - Player eligibility (PlayerEntry PDA exists = staked)
//
//  Run with: npx ts-node scripts/check-season.ts
//  To check a specific player, set PLAYER_TO_CHECK below.
// ============================================================

const SEASON_ID = 0;

// Set to a specific wallet to check eligibility for that player.
// Defaults to your authority wallet if left empty.
const PLAYER_TO_CHECK = "";

async function main() {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.LightindarkV2 as Program<LightindarkV2>;

  const seasonIdBuffer = Buffer.alloc(4);
  seasonIdBuffer.writeUInt32LE(SEASON_ID);

  const [seasonPDA] = PublicKey.findProgramAddressSync(
    [Buffer.from("season"), seasonIdBuffer],
    program.programId
  );

  const [vaultPDA] = PublicKey.findProgramAddressSync(
    [Buffer.from("vault"), seasonIdBuffer],
    program.programId
  );

  // ── Season Config ─────────────────────────────────────────
  let season: any;
  try {
    season = await program.account.seasonConfig.fetch(seasonPDA);
  } catch {
    console.error(`Season ${SEASON_ID} not found on chain. Has initialize-season.ts been run?`);
    process.exit(1);
  }

  console.log("\n=== Season", SEASON_ID, "===");
  console.log("PDA:                ", seasonPDA.toBase58());
  console.log("Authority:          ", season.authority.toBase58());
  console.log("Creator:            ", season.creator.toBase58());
  console.log("Stake amount:       ", season.stakeAmount.toString(), "raw (=", season.stakeAmount.toNumber() / 1_000_000, "LID)");
  console.log("Prize pool:         ", season.prizePool.toString(), "raw (=", season.prizePool.toNumber() / 1_000_000, "LID)");
  console.log("Player count:       ", season.playerCount.toString());
  console.log("Status:             ", JSON.stringify(season.status));
  console.log("Registration start: ", new Date(season.registrationStart.toNumber() * 1000).toISOString());
  console.log("Registration end:   ", new Date(season.registrationEnd.toNumber() * 1000).toISOString());
  console.log("Season end:         ", new Date(season.seasonEnd.toNumber() * 1000).toISOString());

  // ── Top 3 ─────────────────────────────────────────────────
  console.log("\n--- Top 3 (live on-chain) ---");
  const defaultKey = new PublicKey("11111111111111111111111111111111");
  season.topPlayers.forEach((p: PublicKey, i: number) => {
    if (p.toBase58() === defaultKey.toBase58()) {
      console.log(` ${i + 1}. (empty)`);
    } else {
      const timeMs = season.topTimes[i].toNumber();
      const deaths = season.topDeaths[i];
      const mins = Math.floor(timeMs / 60000);
      const secs = Math.floor((timeMs % 60000) / 1000);
      const ms = timeMs % 1000;
      console.log(` ${i + 1}. ${p.toBase58()} — ${String(mins).padStart(2,"0")}:${String(secs).padStart(2,"0")}.${String(ms).padStart(3,"0")} | deaths: ${deaths}`);
    }
  });

  // ── Vault balance ─────────────────────────────────────────
  console.log("\n--- Vault ---");
  try {
    const vaultAccount = await getAccount(provider.connection, vaultPDA);
    console.log(" Vault PDA:     ", vaultPDA.toBase58());
    console.log(" Vault balance: ", vaultAccount.amount.toString(), "raw (=", Number(vaultAccount.amount) / 1_000_000, "LID)");
  } catch {
    console.log(" Vault PDA:     ", vaultPDA.toBase58());
    console.log(" Vault balance:  NOT INITIALIZED — run initialize-season.ts first");
  }

  // ── Player check ──────────────────────────────────────────
  const playerKey = PLAYER_TO_CHECK
    ? new PublicKey(PLAYER_TO_CHECK)
    : provider.wallet.publicKey;

  console.log("\n--- Player Check:", playerKey.toBase58(), "---");

  // 1. LID token balance
  try {
    const playerATA = await getAssociatedTokenAddress(LID_MINT, playerKey);
    const tokenAccount = await getAccount(provider.connection, playerATA);
    const balance = Number(tokenAccount.amount);
    const stakeRequired = season.stakeAmount.toNumber();
    console.log(" LID balance:    ", balance, "raw (=", balance / 1_000_000, "LID)");
    console.log(" Stake required: ", stakeRequired, "raw (=", stakeRequired / 1_000_000, "LID)");
    console.log(" Can stake:      ", balance >= stakeRequired ? "YES ✓" : "NO — insufficient LID");
  } catch {
    console.log(" LID balance:     No LID token account found for this wallet");
    console.log(" Can stake:       NO — needs LID tokens first");
  }

  // 2. PlayerEntry PDA — does it exist? = already staked = eligible
  const [playerEntryPDA] = PublicKey.findProgramAddressSync(
    [Buffer.from("entry"), seasonIdBuffer, playerKey.toBytes()],
    program.programId
  );

  try {
    const entry = await program.account.playerEntry.fetch(playerEntryPDA);
    console.log(" Already staked: YES ✓ — eligible for season", SEASON_ID);
    console.log(" Staked amount:  ", entry.stakedAmount.toString(), "raw");
    console.log(" Best time:      ", entry.bestTimeMs.toString() === "18446744073709551615"
      ? "no runs yet"
      : entry.bestTimeMs.toString() + "ms");
    console.log(" Run count:      ", entry.runCount.toString());
  } catch {
    console.log(" Already staked: NO — PlayerEntry PDA does not exist");
    console.log(" Eligible:       NO — must stake to enter season", SEASON_ID);
  }
}

main().then(() => process.exit(0)).catch((err) => {
  console.error("FAILED:", err);
  process.exit(1);
});