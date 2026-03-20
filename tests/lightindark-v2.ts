import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { LightindarkV2 } from "../target/types/lightindark_v2";
import { PublicKey, Keypair } from "@solana/web3.js";
import { assert } from "chai";

describe("lightindark-v2", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.LightindarkV2 as Program<LightindarkV2>;

  const SEASON_ID = 0;
  const LID_MINT = new PublicKey("3TX7tdXJLnJ51aBRR3TkVocnyFaiyNhETK3CQFp3E6bf");

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

  it("Season config PDA derives correctly", () => {
    assert.ok(seasonPDA, "Season PDA should exist");
    console.log("Season PDA:", seasonPDA.toBase58());
    console.log("Vault PDA: ", vaultPDA.toBase58());
  });

  it("Season config is initialized on-chain", async () => {
    const season = await program.account.seasonConfig.fetch(seasonPDA);
    assert.equal(season.seasonId, SEASON_ID, "Season ID should match");
    assert.ok(season.authority.toBase58(), "Authority should be set");
    assert.equal(
      season.stakeAmount.toNumber(),
      100 * 1_000_000,
      "Stake amount should be 100 LID"
    );
    console.log("Season status:", JSON.stringify(season.status));
    console.log("Prize pool:   ", season.prizePool.toString());
    console.log("Player count: ", season.playerCount.toString());
  });

  it("Season status is Active", async () => {
    const season = await program.account.seasonConfig.fetch(seasonPDA);
    assert.ok(
      JSON.stringify(season.status).includes("active"),
      "Season should be Active"
    );
  });

  it("PlayerEntry PDA derives correctly for a wallet", () => {
    const testWallet = provider.wallet.publicKey;
    const [entryPDA] = PublicKey.findProgramAddressSync(
      [Buffer.from("entry"), seasonIdBuffer, testWallet.toBytes()],
      program.programId
    );
    assert.ok(entryPDA, "PlayerEntry PDA should derive");
    console.log("PlayerEntry PDA for authority:", entryPDA.toBase58());
  });

  it("ActiveRun PDA derives correctly for a wallet", () => {
    const testWallet = provider.wallet.publicKey;
    const [runPDA] = PublicKey.findProgramAddressSync(
      [Buffer.from("run"), testWallet.toBytes()],
      program.programId
    );
    assert.ok(runPDA, "ActiveRun PDA should derive");
    console.log("ActiveRun PDA for authority:", runPDA.toBase58());
  });

  it("Top 3 slots are empty on fresh season", async () => {
    const season = await program.account.seasonConfig.fetch(seasonPDA);
    const defaultKey = "11111111111111111111111111111111";
    season.topPlayers.forEach((p: PublicKey, i: number) => {
      assert.equal(
        p.toBase58(),
        defaultKey,
        `Top player slot ${i} should be empty`
      );
    });
    console.log("All top 3 slots confirmed empty");
  });
});