import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import { ArkhamProtocol } from "../target/types/arkham_protocol";

module.exports = async function (provider) {
  anchor.setProvider(provider);
  
  const program = anchor.workspace.ArkhamProtocol as Program<ArkhamProtocol>;
  const wallet = provider.wallet;

  console.log("Deploying Arkham Protocol...");
  console.log("Program ID:", program.programId.toString());
  console.log("Deploying from wallet:", wallet.publicKey.toString());

  try {
    // Derive the protocol config PDA
    const [protocolConfigPDA] = PublicKey.findProgramAddressSync(
      [Buffer.from("protocol_config")],
      program.programId
    );

    console.log("\nInitializing Protocol Configuration...");
    console.log("Protocol Config PDA:", protocolConfigPDA.toString());

    // Initialize the protocol with default parameters
    const tx = await program.methods
      .initializeProtocolConfig(
        new anchor.BN(1000), // base_rate_per_mb: 0.000001 SOL = 1000 lamports
        200, // protocol_fee_bps: 2%
        [
          new anchor.BN(100_000_000), // Bronze: $100 (6 decimals)
          new anchor.BN(500_000_000), // Silver: $500
          new anchor.BN(1_000_000_000), // Gold: $1000
        ],
        [10000, 12000, 15000], // Multipliers: 1x, 1.2x, 1.5x
        new anchor.BN(500_000_000), // tokens_per_5gb: 0.5 ARKHAM (9 decimals)
        [
          { regionCode: 0, premiumBps: 5000 }, // US: +50%
          { regionCode: 1, premiumBps: 4000 }, // EU: +40%
          { regionCode: 2, premiumBps: 2000 }, // Asia: +20%
        ]
      )
      .accounts({
        protocolConfig: protocolConfigPDA,
        treasury: wallet.publicKey, // Use deployer as initial treasury
        authority: wallet.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    console.log("✅ Protocol initialized! Transaction:", tx);
    console.log("\n📊 Configuration:");
    console.log("  - Base rate: 0.000001 SOL per MB");
    console.log("  - Protocol fee: 2%");
    console.log("  - Tiers: Bronze($100), Silver($500), Gold($1000)");
    console.log("  - Geographic premiums: US(+50%), EU(+40%), Asia(+20%)");
    console.log("\n🎯 Next steps:");
    console.log("  1. Deploy frontend with program ID:", program.programId.toString());
    console.log("  2. Initialize SOL vault PDA");
    console.log("  3. Create USDC/USDT vault token accounts");
    console.log("  4. Set up Pyth oracle feeds");
    console.log("  5. Initialize ARKHAM token mint");
    
  } catch (error) {
    console.error("❌ Deployment failed:", error);
    throw error;
  }
};