process.stdout.write("SCRIPT START\n");
import * as anchor from "@coral-xyz/anchor";
import { PublicKey } from "@solana/web3.js";
import { getOrCreateAssociatedTokenAccount, transfer } from "@solana/spl-token";

const LID_MINT = new PublicKey("3TX7tdXJLnJ51aBRR3TkVocnyFaiyNhETK3CQFp3E6bf");
const YOUR_LID_ACCOUNT = new PublicKey("5apeju9mVF1R6wZQwv2Kw1F3MFWBqwwue4qBaQahDzM8");

// Add test player wallets here
const TEST_PLAYERS: string[] = [
   "Ch3McJzQRuWCxPxjENz7vjLE5bDozfMy7ZC3KoeuvYCv",
];

const AIRDROP_AMOUNT = 200 * 1_000_000; // 200 LID per player

async function main() {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  if (TEST_PLAYERS.length === 0) {
    console.log("No players listed. Add wallet addresses to TEST_PLAYERS array.");
    return;
  }

  const connection = provider.connection;
  const payer = (provider.wallet as anchor.Wallet).payer;

  for (const playerWallet of TEST_PLAYERS) {
    const player = new PublicKey(playerWallet);
    const playerATA = await getOrCreateAssociatedTokenAccount(
      connection, payer, LID_MINT, player
    );

    await transfer(
      connection,
      payer,
      YOUR_LID_ACCOUNT,
      playerATA.address,
      payer,
      AIRDROP_AMOUNT
    );

    console.log(`Sent 200 LID to ${playerWallet} (ATA: ${playerATA.address.toBase58()})`);
  }
}

main().then(() => process.exit(0)).catch((err) => { console.error("FAILED:", err); process.exit(1); });
