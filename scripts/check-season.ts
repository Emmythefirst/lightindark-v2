process.stdout.write("SCRIPT START\n");
import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { LightindarkV2 } from "../target/types/lightindark_v2";
import { PublicKey } from "@solana/web3.js";

async function main() {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.LightindarkV2 as Program<LightindarkV2>;

  const SEASON_ID = 1;
  const seasonIdBuffer = Buffer.alloc(4);
  seasonIdBuffer.writeUInt32LE(SEASON_ID);

  const [seasonPDA] = PublicKey.findProgramAddressSync(
    [Buffer.from("season"), seasonIdBuffer],
    program.programId
  );

  const season = await program.account.seasonConfig.fetch(seasonPDA);

  console.log("\n=== Season", SEASON_ID, "===");
  console.log("PDA:               ", seasonPDA.toBase58());
  console.log("Authority:         ", season.authority.toBase58());
  console.log("Stake amount:      ", season.stakeAmount.toString(), "LID (raw)");
  console.log("Prize pool:        ", season.prizePool.toString());
  console.log("Player count:      ", season.playerCount);
  console.log("Status:            ", JSON.stringify(season.status));
  console.log("Registration start:", new Date(season.registrationStart.toNumber() * 1000).toISOString());
  console.log("Registration end:  ", new Date(season.registrationEnd.toNumber() * 1000).toISOString());
  console.log("Season end:        ", new Date(season.seasonEnd.toNumber() * 1000).toISOString());
}

main().then(() => process.exit(0)).catch((err) => { console.error("FAILED:", err); process.exit(1); });
