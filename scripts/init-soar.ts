process.stdout.write("SCRIPT START\n");
import * as anchor from "@coral-xyz/anchor";
import { SoarProgram, GameType, Genre } from "@magicblock-labs/soar-sdk";
import { Keypair } from "@solana/web3.js";

async function main() {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const authority = (provider.wallet as anchor.Wallet).payer;

  // Use SoarProgram.get() with full AnchorProvider
  const client = SoarProgram.get(provider);

  // Generate a new game keypair
  const gameKeypair = Keypair.generate();
  console.log("Game keypair:", gameKeypair.publicKey.toBase58());

  // Initialize game
  const { newGame, transaction: gameTx } = await client.initializeNewGame(
    gameKeypair.publicKey,
    "LightInDark",
    "A 1-bit mobile speedrun platformer with on-chain competitive seasons",
    Genre.Action,
    GameType.Mobile,
    gameKeypair.publicKey,
    [authority.publicKey]
  );

  const gameSig = await client.sendAndConfirmTransaction(gameTx, [authority, gameKeypair]);
  console.log("Game registered! Tx:", gameSig);
  console.log("SOAR Game Key:", newGame.toBase58());

  // Add Season 1 leaderboard (isAscending=true: lower time = better rank)
  const { transaction: lbTx } = await client.addNewGameLeaderBoard(
    newGame,
    authority.publicKey,
    "Season 1 Speedrun",
    gameKeypair.publicKey,
    100,
    true
  );

  const lbSig = await client.sendAndConfirmTransaction(lbTx, [authority]);
  console.log("Leaderboard created! Tx:", lbSig);

  // Fetch game to get leaderboard count
  const gameAccount = await client.fetchGameAccount(newGame);
  console.log("Leaderboard count:", gameAccount.leaderboardCount.toString());

  // Fetch all leaderboards for this game to get the address
  const leaderboards = await client.fetchAllLeaderboardAccounts();
  const ourLb = leaderboards.find(lb => lb.game.toBase58() === newGame.toBase58());
  
  console.log("\n=== SAVE THESE ===");
  console.log("SOARGameKey:        ", newGame.toBase58());
  if (ourLb) console.log("Season1Leaderboard: ", ourLb.address.toBase58());
}

main().then(() => process.exit(0)).catch((err) => { console.error("FAILED:", err); process.exit(1); });
