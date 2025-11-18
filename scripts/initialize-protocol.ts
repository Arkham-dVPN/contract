import * as anchor from "@coral-xyz/anchor";
import { Program, AnchorProvider, BN } from "@coral-xyz/anchor";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import * as fs from "fs";
import * as path from "path";

// ============================================================================
// Configuration Parameters
// ============================================================================

const CONFIG_PARAMS = {
  baseRatePerMb: new BN(1500),
  protocolFeeBps: 200,
  tierThresholds: [new BN(100), new BN(500), new BN(1000)],
  tierMultipliers: [10000, 12000, 15000],
  tokensPer5gb: new BN(500000000),
  geoPremiums: [
    { regionCode: 0, premiumBps: 5000 },
    { regionCode: 1, premiumBps: 4000 },
    { regionCode: 2, premiumBps: 2000 },
  ],
  // Oracle Authority - this is the public key of our oracle server
  oracleAuthority: new PublicKey("9WE7mxzUNFGJ4df3kuALhAWLmBmFkYvTt8LkMDFVuycC"),
};

// ============================================================================
// Helper Functions
// ============================================================================

/**
 * Loads the IDL from the JSON file
 */
function loadIdl(): any {
  const idlPath = path.join(
    __dirname,
    "..",
    "target",
    "idl",
    "arkham_protocol.json"
  );

  if (!fs.existsSync(idlPath)) {
    throw new Error(`IDL file not found at: ${idlPath}`);
  }

  const idlString = fs.readFileSync(idlPath, "utf8");
  return JSON.parse(idlString);
}

/**
 * Derives the Protocol Config PDA
 */
function getProtocolConfigPda(programId: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("protocol_config")],
    programId
  );
}

/**
 * Derives the ARKHAM Mint PDA
 */
function getArkhamMintPda(programId: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("arkham_mint")],
    programId
  );
}

/**
 * Derives the Mint Authority PDA
 */
function getMintAuthorityPda(programId: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("arkham"), Buffer.from("mint"), Buffer.from("authority")],
    programId
  );
}

/**
 * Checks if an account exists on-chain (raw check, no deserialization)
 */
async function accountExists(
  connection: anchor.web3.Connection,
  address: PublicKey
): Promise<boolean> {
  try {
    const accountInfo = await connection.getAccountInfo(address);
    return accountInfo !== null;
  } catch (error) {
    return false;
  }
}

/**
 * Safely fetches the Protocol Config account
 */
async function fetchProtocolConfig(
  program: Program,
  protocolConfigPda: PublicKey
): Promise<any | null> {
  try {
    // @ts-ignore - IDL is loaded dynamically, types may not be available
    const config = await program.account.protocolConfig.fetch(protocolConfigPda);
    return config;
  } catch (error) {
    // Account doesn't exist or can't be deserialized
    console.log("   ⚠️  Could not deserialize config (might be from old version)");
    return null;
  }
}

/**
 * Gets raw account data to check if account exists
 */
async function getAccountInfo(
  connection: anchor.web3.Connection,
  address: PublicKey
) {
  try {
    return await connection.getAccountInfo(address);
  } catch (error) {
    return null;
  }
}

// ============================================================================
// Main Initialization Functions
// ============================================================================

/**
 * Initializes the Protocol Config account
 */
async function initializeProtocolConfig(
  program: Program,
  authority: PublicKey,
  protocolConfigPda: PublicKey,
  treasury: PublicKey
): Promise<void> {
  console.log("\n📝 Initializing Protocol Config...");

  try {
    const tx = await program.methods
      .initializeProtocolConfig(
        CONFIG_PARAMS.baseRatePerMb,
        CONFIG_PARAMS.protocolFeeBps,
        CONFIG_PARAMS.tierThresholds,
        CONFIG_PARAMS.tierMultipliers,
        CONFIG_PARAMS.tokensPer5gb,
        CONFIG_PARAMS.geoPremiums,
        CONFIG_PARAMS.oracleAuthority
      )
      .accounts({
        protocolConfig: protocolConfigPda,
        treasury: treasury,
        authority: authority,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    console.log("✅ Protocol Config initialized successfully!");
    console.log(`   Transaction: ${tx}`);
    console.log(`   Config PDA: ${protocolConfigPda.toBase58()}`);
  } catch (error: any) {
    console.error("❌ Failed to initialize Protocol Config:");
    console.error(`   ${error.message}`);
    throw error;
  }
}

/**
 * Updates the Protocol Config account
 */
async function updateProtocolConfig(
  program: Program,
  authority: PublicKey,
  protocolConfigPda: PublicKey
): Promise<void> {
  console.log("\n🔄 Updating Protocol Config...");

  try {
    const tx = await program.methods
      .updateProtocolConfig(
        CONFIG_PARAMS.baseRatePerMb,
        CONFIG_PARAMS.protocolFeeBps,
        CONFIG_PARAMS.tierThresholds,
        CONFIG_PARAMS.tierMultipliers,
        CONFIG_PARAMS.tokensPer5gb,
        CONFIG_PARAMS.geoPremiums,
        null, // reputationUpdater - keep existing
        CONFIG_PARAMS.oracleAuthority  // Set the oracle authority
      )
      .accounts({
        protocolConfig: protocolConfigPda,
        authority: authority,
      })
      .rpc();

    console.log("✅ Protocol Config updated successfully!");
    console.log(`   Transaction: ${tx}`);
  } catch (error: any) {
    console.error("❌ Failed to update Protocol Config:");
    console.error(`   ${error.message}`);
    throw error;
  }
}

/**
 * Closes an existing account (for cleanup)
 */
async function closeProtocolConfig(
  program: Program,
  authority: PublicKey,
  protocolConfigPda: PublicKey
): Promise<void> {
  console.log("\n🗑️  Attempting to close existing Protocol Config...");

  try {
    const tx = await program.methods
      .closeProtocolConfig()
      .accounts({
        protocolConfig: protocolConfigPda,
        authority: authority,
        receiver: authority,
      })
      .rpc();

    console.log("✅ Protocol Config closed successfully!");
    console.log(`   Transaction: ${tx}`);
  } catch (error: any) {
    console.error("❌ Failed to close Protocol Config:");
    console.error(`   ${error.message}`);
    throw error;
  }
}

/**
 * Initializes the ARKHAM token mint
 */
async function initializeArkhamMint(
  program: Program,
  authority: PublicKey,
  protocolConfigPda: PublicKey,
  arkhamMintPda: PublicKey,
  mintAuthorityPda: PublicKey
): Promise<void> {
  console.log("\n🪙 Initializing ARKHAM Token Mint...");

  try {
    const tx = await program.methods
      .initializeArkhamMint()
      .accounts({
        arkhamMint: arkhamMintPda,
        mintAuthority: mintAuthorityPda,
        protocolConfig: protocolConfigPda,
        authority: authority,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    console.log("✅ ARKHAM Mint initialized successfully!");
    console.log(`   Transaction: ${tx}`);
    console.log(`   Mint PDA: ${arkhamMintPda.toBase58()}`);
  } catch (error: any) {
    console.error("❌ Failed to initialize ARKHAM Mint:");
    console.error(`   ${error.message}`);
    throw error;
  }
}

// ============================================================================
// Main Script Logic
// ============================================================================

async function main() {
  console.log("🚀 Arkham Protocol Initialization Script");
  console.log("==========================================\n");

  // Step 1: Set up provider and load program
  console.log("📡 Setting up connection...");
  const provider = AnchorProvider.env();
  anchor.setProvider(provider);

  const authority = provider.wallet.publicKey;
  console.log(`   Authority: ${authority.toBase58()}`);
  console.log(`   Network: ${provider.connection.rpcEndpoint}`);

  // Step 2: Load IDL and create program instance
  console.log("\n📚 Loading program IDL...");
  const idl = loadIdl();
  const programId = new PublicKey(idl.address || idl.metadata.address);
  console.log(`   Program ID: ${programId.toBase58()}`);

  // Cast to 'any' to avoid type issues with dynamically loaded IDL
  const program = new Program(idl as any, provider) as Program;
  console.log("✅ Program loaded successfully!");

  // Step 3: Derive PDAs
  console.log("\n🔑 Deriving PDAs...");
  const [protocolConfigPda, protocolConfigBump] = getProtocolConfigPda(programId);
  const [arkhamMintPda, arkhamMintBump] = getArkhamMintPda(programId);
  const [mintAuthorityPda, mintAuthorityBump] = getMintAuthorityPda(programId);

  console.log(`   Protocol Config PDA: ${protocolConfigPda.toBase58()} (bump: ${protocolConfigBump})`);
  console.log(`   ARKHAM Mint PDA: ${arkhamMintPda.toBase58()} (bump: ${arkhamMintBump})`);
  console.log(`   Mint Authority PDA: ${mintAuthorityPda.toBase58()} (bump: ${mintAuthorityBump})`);

  // Step 4: Handle Protocol Config with robust checking
  console.log("\n🔍 Checking Protocol Config status...");
  
  // First check if account exists at all
  const accountInfo = await getAccountInfo(provider.connection, protocolConfigPda);
  const accountPhysicallyExists = accountInfo !== null;
  
  // Then try to deserialize it
  const existingConfig = await fetchProtocolConfig(program, protocolConfigPda);

  if (!accountPhysicallyExists) {
    // Account doesn't exist at all - fresh initialization
    console.log("   Status: Not initialized (no account)");
    await initializeProtocolConfig(
      program,
      authority,
      protocolConfigPda,
      authority // Using authority as treasury for now
    );
  } else if (existingConfig === null) {
    // Account exists but can't be deserialized - likely old version
    console.log("   Status: Account exists but cannot be deserialized");
    console.log("   This likely means the account is from an old program version.");
    console.log("\n⚠️  OPTIONS:");
    console.log("   1. Close the old account and reinitialize (requires close_protocol_config instruction)");
    console.log("   2. Manually migrate the data");
    console.log("   3. Use a different PDA seed\n");
    
    // Try to close and reinitialize
    console.log("   Attempting to close and reinitialize...");
    try {
      await closeProtocolConfig(program, authority, protocolConfigPda);
      // Wait a bit for the account to be closed
      await new Promise(resolve => setTimeout(resolve, 2000));
      await initializeProtocolConfig(program, authority, protocolConfigPda, authority);
    } catch (closeError: any) {
      console.error("\n❌ Could not close existing account.");
      console.error("   You may need to:");
      console.error("   1. Add a close_protocol_config instruction to your program");
      console.error("   2. Redeploy with a different program ID");
      console.error("   3. Use solana CLI to close the account manually");
      throw new Error("Cannot proceed - account exists but is incompatible");
    }
  } else {
    // Account exists and can be deserialized
    console.log("   Status: Already initialized and compatible");
    console.log(`   Current authority: ${existingConfig.authority.toBase58()}`);
    console.log(`   Current base rate: ${existingConfig.baseRatePerMb.toString()}`);
    
    // Check if oracle authority needs to be set
    const needsOracleUpdate = 
      !existingConfig.oracleAuthority || 
      existingConfig.oracleAuthority.equals(PublicKey.default) ||
      !existingConfig.oracleAuthority.equals(CONFIG_PARAMS.oracleAuthority);
    
    if (needsOracleUpdate) {
      console.log("   ⚠️  Oracle authority needs to be set/updated");
      await updateProtocolConfig(program, authority, protocolConfigPda);
    } else {
      console.log("   ✅ Configuration is up to date");
    }
  }

  // Step 5: Handle ARKHAM Mint
  console.log("\n🔍 Checking ARKHAM Mint status...");

  // Re-fetch config to get latest state
  const currentConfig = await fetchProtocolConfig(program, protocolConfigPda);

  if (!currentConfig) {
    throw new Error("Protocol Config must be initialized before mint initialization");
  }

  const mintIsInitialized =
    currentConfig.arkhamTokenMint &&
    !currentConfig.arkhamTokenMint.equals(PublicKey.default);

  if (mintIsInitialized) {
    console.log("   Status: Already initialized");
    console.log(`   Mint address: ${currentConfig.arkhamTokenMint.toBase58()}`);
  } else {
    console.log("   Status: Not initialized");

    // Double-check if the mint account exists (in case config is stale)
    const mintExists = await accountExists(provider.connection, arkhamMintPda);

    if (mintExists) {
      console.log("⚠️  Mint account exists but not linked in config!");
      console.log("   This might indicate a previous failed transaction.");
      console.log("   Manual intervention may be required.");
    } else {
      await initializeArkhamMint(
        program,
        authority,
        protocolConfigPda,
        arkhamMintPda,
        mintAuthorityPda
      );
    }
  }

  // Step 6: Summary
  console.log("\n" + "=".repeat(50));
  console.log("✨ Initialization Complete!");
  console.log("=".repeat(50));

  const finalConfig = await fetchProtocolConfig(program, protocolConfigPda);
  console.log("\n📋 Final Configuration:");
  console.log(`   Protocol Config: ${protocolConfigPda.toBase58()}`);
  console.log(`   Authority: ${finalConfig.authority.toBase58()}`);
  console.log(`   Treasury: ${finalConfig.treasury.toBase58()}`);
  console.log(`   Base Rate (per MB): ${finalConfig.baseRatePerMb.toString()} lamports`);
  console.log(`   Protocol Fee: ${finalConfig.protocolFeeBps / 100}%`);
  console.log(`   Tier Thresholds: [${finalConfig.tierThresholds.map((t: any) => t.toString()).join(", ")}]`);
  console.log(`   Tier Multipliers: [${finalConfig.tierMultipliers.join(", ")}]`);
  console.log(`   Tokens per 5GB: ${finalConfig.tokensPer5gb ? finalConfig.tokensPer5gb.toString() : 'N/A'}`);
  console.log(`   ARKHAM Mint: ${finalConfig.arkhamTokenMint.toBase58()}`);
  console.log(`   Oracle Authority: ${finalConfig.oracleAuthority ? finalConfig.oracleAuthority.toBase58() : 'Not set'}`);
  console.log(`   Reputation Updater: ${finalConfig.reputationUpdater.toBase58()}`);
  console.log(`   Geographic Premiums: ${finalConfig.geoPremiums ? finalConfig.geoPremiums.length : 0} regions configured`);
  console.log("\n✅ Protocol is ready for use!\n");
}

// ============================================================================
// Script Entry Point
// ============================================================================

main()
  .then(() => {
    process.exit(0);
  })
  .catch((error) => {
    console.error("\n❌ Script failed with error:");
    console.error(error);
    process.exit(1);
  });
