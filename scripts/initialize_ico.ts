import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import { TokenSaleProject } from "../target/types/token_sale_project";

// Cluster + signing wallet are read from the environment
// (ANCHOR_PROVIDER_URL / ANCHOR_WALLET) so nothing is hardcoded.
const provider = anchor.AnchorProvider.env();
anchor.setProvider(provider);

const program = anchor.workspace.TokenSaleProject as Program<TokenSaleProject>;

async function initializeSale() {
  const admin = provider.wallet.publicKey;
  const [saleConfig] = PublicKey.findProgramAddressSync(
    [Buffer.from("sale_config")],
    program.programId
  );

  // ProgramData account (holds the upgrade authority). initialize is gated to it.
  const [programData] = PublicKey.findProgramAddressSync(
    [program.programId.toBuffer()],
    new PublicKey("BPFLoaderUpgradeab1e11111111111111111111111")
  );

  // Configure for your sale (or wire to env / CLI args).
  const usdcMint = new PublicKey(process.env.USDC_MINT ?? "");
  const tokenMint = new PublicKey(process.env.TOKEN_MINT ?? "");
  const tokenPriceCents = new anchor.BN(1); // $0.01 per token
  const tokenDecimals = 9;
  const usdcDecimals = 6;

  const now = Math.floor(Date.now() / 1000);
  const startTs = new anchor.BN(now); // opens immediately
  const endTs = new anchor.BN(now + 7 * 24 * 3600); // 7-day window

  const tx = await program.methods
    .initialize(
      usdcMint,
      tokenMint,
      tokenPriceCents,
      tokenDecimals,
      usdcDecimals,
      startTs,
      endTs
    )
    .accountsPartial({
      saleConfig,
      admin,
      program: program.programId,
      programData,
      systemProgram: SystemProgram.programId,
    })
    .rpc();

  console.log(`Sale initialized. Tx: ${tx}`);
  console.log(`sale_config PDA: ${saleConfig.toBase58()}`);
  console.log(`Admin: ${admin.toBase58()}`);
}

initializeSale().catch((err) => {
  console.error("Initialization failed:", err);
  process.exit(1);
});
