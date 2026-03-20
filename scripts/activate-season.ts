process.stdout.write("SCRIPT START\n");
import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { LightindarkV2 } from "../target/types/lightindark_v2";

async function main() {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.LightindarkV2 as Program<LightindarkV2>;

  const SEASON_ID = 0;
  const seasonIdBuffer = Buffer.alloc(4);
  seasonIdBuffer.writeUInt32LE(SEASON_ID);

  console.log("Activating season", SEASON_ID);

  const tx = await program.methods
    .activateSeason(SEASON_ID)
    .accounts({})
    .rpc();

  console.log("Season activated! Tx:", tx);
}

main().then(() => process.exit(0)).catch((err) => { console.error("FAILED:", err); process.exit(1); });
