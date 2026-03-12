process.stdout.write("SCRIPT START\n");
import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { LightindarkV2 } from "../target/types/lightindark_v2";
import { PublicKey } from "@solana/web3.js";
import {
  getAssociatedTokenAddress,
} from "@solana/spl-token";

// ============================================================
//  distribute-rewards.ts
//
//  Permissionless — anyone can call this after season_end.
//  Winners are read from SeasonConfig.top_players on-chain.
//  No need to manually specify winners.
//
//  Run with:
//    npx ts-node scripts/distribute-rewards.ts
// ============================================================

const SEASON_ID = 1;
const LID_TOKEN_MINT = new PublicKey("3TX7tdXJLnJ51aBRR3TkVocnyFaiyNhETK3CQFp3E6bf");

async function main() {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.LightindarkV2 as Program<LightindarkV2>;

  // ── Derive PDAs ──────────────────────────────────────────
  const seasonIdBytes = Buffer.alloc(4);
  seasonIdBytes.writeUInt32LE(SEASON_ID);

  const [seasonConfigPDA] = PublicKey.findProgramAddressSync(
    [Buffer.from("season"), seasonIdBytes],
    program.programId
  );

  const [vaultPDA] = PublicKey.findProgramAddressSync(
    [Buffer.from("vault"), seasonIdBytes],
    program.programId
  );

  // ── Read current top 3 from chain ────────────────────────
  console.log("Reading SeasonConfig from chain...");
  const seasonAccount = await program.account.seasonConfig.fetch(seasonConfigPDA);

  console.log("\nSeason", SEASON_ID, "status:", seasonAccount.status);
  console.log("Prize pool:", seasonAccount.prizePool.toString(), "raw tokens");
  console.log("\nTop 3 players on-chain:");
  seasonAccount.topPlayers.forEach((p: PublicKey, i: number) => {
    console.log(
      ` ${i + 1}. ${p.toBase58()} — time: ${seasonAccount.topTimes[i].toString()}ms, deaths: ${seasonAccount.topDeaths[i]}`
    );
  });

  const [w1, w2, w3] = seasonAccount.topPlayers as PublicKey[];
  const creator = seasonAccount.creator as PublicKey;

  // ── Derive winner ATAs ───────────────────────────────────
  const winner1ATA = await getAssociatedTokenAddress(LID_TOKEN_MINT, w1);
  const winner2ATA = await getAssociatedTokenAddress(LID_TOKEN_MINT, w2);
  const winner3ATA = await getAssociatedTokenAddress(LID_TOKEN_MINT, w3);
  const creatorATA = await getAssociatedTokenAddress(LID_TOKEN_MINT, creator);

  console.log("\nDistributing rewards for season", SEASON_ID, "...");

  const tx = await program.methods
    .distributeSeasonRewards(SEASON_ID)
    .accounts({
      caller: provider.wallet.publicKey,
      tokenMint: LID_TOKEN_MINT,
      winner1TokenAccount: winner1ATA,
      winner2TokenAccount: winner2ATA,
      winner3TokenAccount: winner3ATA,
      creatorTokenAccount: creatorATA,
    })
    .rpc();

  console.log("\nRewards distributed! Tx:", tx);
  console.log("\nSplit summary:");
  const pool = Number(seasonAccount.prizePool);
  console.log(` Winner 1: ${Math.floor(pool * 0.40)} tokens (40%)`);
  console.log(` Winner 2: ${Math.floor(pool * 0.20)} tokens (20%)`);
  console.log(` Winner 3: ${Math.floor(pool * 0.10)} tokens (10%)`);
  console.log(` Creator:  ${Math.floor(pool * 0.05)} tokens (5%)`);
  console.log(` Burned:   ${Math.floor(pool * 0.10)} tokens (10%)`);
  console.log(` Rollover: ${Math.floor(pool * 0.15)} tokens (15%) — stays in vault for next season`);
}

main()
  .then(() => process.exit(0))
  .catch((err) => {
    console.error("FAILED:", err);
    process.exit(1);
  });