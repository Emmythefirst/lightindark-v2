process.stdout.write("SCRIPT START\n");
import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { LightindarkV2 } from "../target/types/lightindark_v2";
import { PublicKey } from "@solana/web3.js";
import { getAssociatedTokenAddress } from "@solana/spl-token";

const SEASON_ID_TO_CLOSE = 0;
const LID_MINT = new PublicKey("3TX7tdXJLnJ51aBRR3TkVocnyFaiyNhETK3CQFp3E6bf");

async function main() {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.LightindarkV2 as Program<LightindarkV2>;
  const authority = provider.wallet.publicKey;

  const seasonIdBuffer = Buffer.alloc(4);
  seasonIdBuffer.writeUInt32LE(SEASON_ID_TO_CLOSE);

  const [seasonPDA] = PublicKey.findProgramAddressSync(
    [Buffer.from("season"), seasonIdBuffer],
    program.programId
  );
  const [vaultPDA] = PublicKey.findProgramAddressSync(
    [Buffer.from("vault"), seasonIdBuffer],
    program.programId
  );

  console.log("Closing season", SEASON_ID_TO_CLOSE);
  console.log("Season PDA:", seasonPDA.toBase58());
  console.log("Vault PDA: ", vaultPDA.toBase58());

  const seasonInfo = await provider.connection.getAccountInfo(seasonPDA);
  const vaultInfo  = await provider.connection.getAccountInfo(vaultPDA);

  console.log("Season account exists:", !!seasonInfo);
  console.log("Vault account exists: ", !!vaultInfo);

  if (!seasonInfo && !vaultInfo) {
    console.log("Nothing to close — both accounts already gone.");
    return;
  }

  const authorityATA = await getAssociatedTokenAddress(LID_MINT, authority);

  // Close season config if it exists
  if (seasonInfo) {
    try {
      const tx = await program.methods
        .closeSeason(SEASON_ID_TO_CLOSE)
        .accounts({ authorityTokenAccount: authorityATA })
        .rpc();
      console.log("Season + vault closed! Tx:", tx);
      return;
    } catch (e: any) {
      if (e.error?.errorCode?.code === "AccountDidNotDeserialize") {
        console.log("Struct mismatch — force closing season config...");
        const tx = await program.methods
          .forceCloseSeason(SEASON_ID_TO_CLOSE)
          .accounts({})
          .rpc();
        console.log("Season force-closed! Tx:", tx);
      } else {
        throw e;
      }
    }
  }

  // Close vault separately if it still exists
  const vaultStillExists = await provider.connection.getAccountInfo(vaultPDA);
  if (vaultStillExists) {
    console.log("Closing vault...");
    const tx = await program.methods
      .closeVault(SEASON_ID_TO_CLOSE)
      .accounts({ authorityTokenAccount: authorityATA })
      .rpc();
    console.log("Vault closed! Tx:", tx);
  }

  console.log("Done. You can now reinitialize season", SEASON_ID_TO_CLOSE);
}

main().then(() => process.exit(0)).catch((err) => {
  console.error("FAILED:", err);
  process.exit(1);
});