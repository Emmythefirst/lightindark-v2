process.stdout.write("SCRIPT START\n");
import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { LightindarkV2 } from "../target/types/lightindark_v2";
import { PublicKey } from "@solana/web3.js";

const LID_MINT = new PublicKey("3TX7tdXJLnJ51aBRR3TkVocnyFaiyNhETK3CQFp3E6bf");

async function main() {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.LightindarkV2 as Program<LightindarkV2>;
  const authority = provider.wallet.publicKey;

  const SEASON_ID = 2;
  const seasonIdBuffer = Buffer.alloc(4);
  seasonIdBuffer.writeUInt32LE(SEASON_ID);

  const [seasonPDA] = PublicKey.findProgramAddressSync(
    [Buffer.from("season"), seasonIdBuffer],
    program.programId
  );

  // Registration: next 7 days
  // Season active: 7-37 days from now
  const now = Math.floor(Date.now() / 1000);
  const registrationStart = now;
  const registrationEnd = now + 7 * 24 * 60 * 60;
  const seasonEnd = now + 37 * 24 * 60 * 60;

  // Stake amount: 100 LID tokens (6 decimals)
  const stakeAmount = new anchor.BN(100 * 1_000_000);

  console.log("Initializing season", SEASON_ID);
  console.log("Season PDA:", seasonPDA.toBase58());
  console.log("Stake amount: 100 LID");

  const tx = await program.methods
    .initializeSeason(
      SEASON_ID,
      stakeAmount,
      new anchor.BN(registrationStart),
      new anchor.BN(registrationEnd),
      new anchor.BN(seasonEnd)
    )
    .accounts({})
    .rpc();

  console.log("Season initialized! Tx:", tx);
  console.log("Season PDA:", seasonPDA.toBase58());
}

main().catch(console.error);