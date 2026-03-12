process.stdout.write("SCRIPT START\n");
import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { LightindarkV2 } from "../target/types/lightindark_v2";
import { PublicKey } from "@solana/web3.js";

// Fill in top 3 winner wallets before running
const WINNERS = [
  new PublicKey("WINNER_1_PUBKEY_HERE"),
  new PublicKey("WINNER_2_PUBKEY_HERE"),
  new PublicKey("WINNER_3_PUBKEY_HERE"),
];

async function main() {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.LightindarkV2 as Program<LightindarkV2>;
  const authority = provider.wallet.publicKey;

  const SEASON_ID = 1;

  console.log("Distributing rewards for season", SEASON_ID);
  console.log("Winners:");
  WINNERS.forEach((w, i) => console.log(` ${i + 1}. ${w.toBase58()}`));

  const tx = await program.methods
    .distributeSeasonRewards(SEASON_ID, WINNERS)
    .accounts({})
    .rpc();

  console.log("Rewards distributed! Tx:", tx);
}

main().then(() => process.exit(0)).catch((err) => { console.error("FAILED:", err); process.exit(1); });
