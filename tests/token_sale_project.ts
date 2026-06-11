import * as anchor from "@coral-xyz/anchor";
import { BN, Program } from "@coral-xyz/anchor";
import { TokenSaleProject } from "../target/types/token_sale_project";
import {
  createMint,
  createAccount,
  mintTo,
  getAccount,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { Keypair, PublicKey, SystemProgram, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { assert } from "chai";

describe("token_sale_project", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.TokenSaleProject as Program<TokenSaleProject>;
  const connection = provider.connection;
  const payer = (provider.wallet as anchor.Wallet).payer;

  const TOKEN_DECIMALS = 9;
  const USDC_DECIMALS = 6;
  const TOKEN_PRICE_CENTS = 1; // $0.01

  let usdcMint: PublicKey;
  let tokenMint: PublicKey;
  let icoStatePda: PublicKey;
  let vaultUsdc: PublicKey;
  let vault: PublicKey;
  let adminUsdc: PublicKey;
  let adminToken: PublicKey;
  let buyer: Keypair;
  let buyerUsdc: PublicKey;
  let buyerToken: PublicKey;

  before(async () => {
    [icoStatePda] = PublicKey.findProgramAddressSync(
      [Buffer.from("sale_config")],
      program.programId
    );

    usdcMint = await createMint(connection, payer, payer.publicKey, null, USDC_DECIMALS);
    tokenMint = await createMint(connection, payer, payer.publicKey, null, TOKEN_DECIMALS);

    adminUsdc = await createAccount(connection, payer, usdcMint, payer.publicKey);
    adminToken = await createAccount(connection, payer, tokenMint, payer.publicKey);

    // Vaults are owned by the ICO PDA, not ATAs. A PDA is off-curve, so we must
    // create plain token accounts (by passing a keypair) rather than ATAs —
    // getAssociatedTokenAddressSync rejects off-curve owners (TokenOwnerOffCurveError).
    vaultUsdc = await createAccount(connection, payer, usdcMint, icoStatePda, Keypair.generate());
    vault = await createAccount(connection, payer, tokenMint, icoStatePda, Keypair.generate());

    // 1,000,000 tokens available to sell
    await mintTo(connection, payer, tokenMint, vault, payer, 1_000_000 * 10 ** TOKEN_DECIMALS);

    buyer = Keypair.generate();
    const sig = await connection.requestAirdrop(buyer.publicKey, 2 * LAMPORTS_PER_SOL);
    await connection.confirmTransaction(sig);

    buyerUsdc = await createAccount(connection, payer, usdcMint, buyer.publicKey);
    buyerToken = await createAccount(connection, payer, tokenMint, buyer.publicKey);

    // Fund buyer with 1,000 mock USDC
    await mintTo(connection, payer, usdcMint, buyerUsdc, payer, 1_000 * 10 ** USDC_DECIMALS);
  });

  it("initializes the ICO", async () => {
    const now = Math.floor(Date.now() / 1000);
    const startTs = new BN(now - 60); // sale already open
    const endTs = new BN(now + 3600); // sale closes in 1 hour
    await program.methods
      .initialize(
        usdcMint,
        tokenMint,
        new BN(TOKEN_PRICE_CENTS),
        TOKEN_DECIMALS,
        USDC_DECIMALS,
        startTs,
        endTs
      )
      .accounts({
        saleConfig: icoStatePda,
        admin: payer.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const state = await program.account.icoState.fetch(icoStatePda);
    assert.ok(state.admin.equals(payer.publicKey));
    assert.ok(state.usdcMint.equals(usdcMint));
    assert.ok(state.tokenMint.equals(tokenMint));
    assert.equal(state.tokenPriceCents.toNumber(), TOKEN_PRICE_CENTS);
    assert.equal(state.tokenDecimals, TOKEN_DECIMALS);
    assert.equal(state.usdcDecimals, USDC_DECIMALS);
    assert.equal(state.startTs.toNumber(), startTs.toNumber());
    assert.equal(state.endTs.toNumber(), endTs.toNumber());
  });

  it("purchases tokens and transfers correct USDC", async () => {
    // Buying 100 tokens at $0.01: USDC = (100e9 * 1 * 1e6) / (1e9 * 100) = 1e6 = 1 USDC
    const tokensToBuy = new BN(100 * 10 ** TOKEN_DECIMALS);
    const expectedUsdcCost = 1 * 10 ** USDC_DECIMALS;

    const buyerUsdcBefore = await getAccount(connection, buyerUsdc);
    const buyerTokenBefore = await getAccount(connection, buyerToken);
    const vaultUsdcBefore = await getAccount(connection, vaultUsdc);
    const vaultBefore = await getAccount(connection, vault);

    await program.methods
      .purchase(tokensToBuy)
      .accounts({
        saleConfig: icoStatePda,
        buyer: buyer.publicKey,
        buyerUsdc,
        vaultUsdc,
        buyerToken,
        vault,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .signers([buyer])
      .rpc();

    const buyerUsdcAfter = await getAccount(connection, buyerUsdc);
    const buyerTokenAfter = await getAccount(connection, buyerToken);
    const vaultUsdcAfter = await getAccount(connection, vaultUsdc);
    const vaultAfter = await getAccount(connection, vault);

    assert.equal(
      Number(buyerUsdcBefore.amount) - Number(buyerUsdcAfter.amount),
      expectedUsdcCost,
      "buyer USDC debit incorrect"
    );
    assert.equal(
      Number(buyerTokenAfter.amount) - Number(buyerTokenBefore.amount),
      tokensToBuy.toNumber(),
      "buyer token credit incorrect"
    );
    assert.equal(
      Number(vaultUsdcAfter.amount) - Number(vaultUsdcBefore.amount),
      expectedUsdcCost,
      "vault USDC credit incorrect"
    );
    assert.equal(
      Number(vaultBefore.amount) - Number(vaultAfter.amount),
      tokensToBuy.toNumber(),
      "vault token debit incorrect"
    );
  });

  it("updates the token price", async () => {
    await program.methods
      .updatePrice(new BN(50))
      .accounts({
        saleConfig: icoStatePda,
        admin: payer.publicKey,
      })
      .rpc();

    const state = await program.account.icoState.fetch(icoStatePda);
    assert.equal(state.tokenPriceCents.toNumber(), 50);

    // Reset for remaining tests
    await program.methods
      .updatePrice(new BN(TOKEN_PRICE_CENTS))
      .accounts({
        saleConfig: icoStatePda,
        admin: payer.publicKey,
      })
      .rpc();
  });

  it("rejects a price update from a non-admin", async () => {
    try {
      await program.methods
        .updatePrice(new BN(999))
        .accounts({
          saleConfig: icoStatePda,
          admin: buyer.publicKey,
        })
        .signers([buyer])
        .rpc();
      assert.fail("expected Unauthorized error");
    } catch (err: any) {
      assert.ok(
        err.message.includes("Unauthorized") || err.error?.errorCode?.code === "Unauthorized",
        `unexpected error: ${err.message}`
      );
    }
  });

  it("rejects a zero price", async () => {
    try {
      await program.methods
        .updatePrice(new BN(0))
        .accounts({
          saleConfig: icoStatePda,
          admin: payer.publicKey,
        })
        .rpc();
      assert.fail("expected InvalidPrice error");
    } catch (err: any) {
      assert.ok(
        err.message.includes("InvalidPrice") || err.error?.errorCode?.code === "InvalidPrice",
        `unexpected error: ${err.message}`
      );
    }
  });

  it("allows admin to withdraw from vaults", async () => {
    const vaultUsdcInfo = await getAccount(connection, vaultUsdc);
    const vaultInfo = await getAccount(connection, vault);
    const withdrawUsdc = new BN(vaultUsdcInfo.amount.toString());
    const withdrawTokens = new BN(vaultInfo.amount.toString());

    const adminUsdcBefore = await getAccount(connection, adminUsdc);
    const adminTokenBefore = await getAccount(connection, adminToken);

    await program.methods
      .withdraw(withdrawUsdc, withdrawTokens)
      .accounts({
        saleConfig: icoStatePda,
        admin: payer.publicKey,
        adminUsdc,
        vaultUsdc,
        adminToken,
        vault,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    const adminUsdcAfter = await getAccount(connection, adminUsdc);
    const adminTokenAfter = await getAccount(connection, adminToken);
    const vaultUsdcAfter = await getAccount(connection, vaultUsdc);
    const vaultAfter = await getAccount(connection, vault);

    assert.equal(Number(vaultUsdcAfter.amount), 0, "vault USDC should be empty");
    assert.equal(Number(vaultAfter.amount), 0, "token vault should be empty");
    assert.equal(
      Number(adminUsdcAfter.amount) - Number(adminUsdcBefore.amount),
      Number(withdrawUsdc)
    );
    assert.equal(
      Number(adminTokenAfter.amount) - Number(adminTokenBefore.amount),
      Number(withdrawTokens)
    );
  });

  it("rejects withdrawal from a non-admin", async () => {
    try {
      await program.methods
        .withdraw(new BN(0), new BN(0))
        .accounts({
          saleConfig: icoStatePda,
          admin: buyer.publicKey,
          adminUsdc: buyerUsdc,
          vaultUsdc,
          adminToken: buyerToken,
          vault,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([buyer])
        .rpc();
      assert.fail("expected Unauthorized error");
    } catch (err: any) {
      assert.ok(
        err.message.includes("Unauthorized") || err.error?.errorCode?.code === "Unauthorized",
        `unexpected error: ${err.message}`
      );
    }
  });

  it("closes the ICO and reclaims vault rent", async () => {
    await program.methods
      .close()
      .accounts({
        saleConfig: icoStatePda,
        admin: payer.publicKey,
        vaultUsdc,
        vault,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    try {
      await program.account.icoState.fetch(icoStatePda);
      assert.fail("icoState account should be closed");
    } catch (err: any) {
      assert.ok(
        err.message.includes("Account does not exist") ||
          err.message.includes("could not find account"),
        `unexpected error: ${err.message}`
      );
    }
  });
});
