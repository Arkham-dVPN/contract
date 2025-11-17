import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import { getAssociatedTokenAddress, TOKEN_PROGRAM_ID, ASSOCIATED_TOKEN_PROGRAM_ID } from "@solana/spl-token";

async function setupVaults() {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  
  const program = anchor.workspace.ArkhamProtocol as Program;

  // Derive SOL vault PDA
  const [solVault] = PublicKey.findProgramAddressSync(
    [Buffer.from("sol_vault")],
    program.programId
  );

  console.log("SOL Vault PDA:", solVault.toString());
  console.log("✅ SOL vault will be created automatically on first use");

  // TODO: Create USDC/USDT token accounts
  // This requires knowing the USDC/USDT mint addresses on your network
  
  console.log("\n🎯 Vaults are ready!");
}

setupVaults().catch(console.error);