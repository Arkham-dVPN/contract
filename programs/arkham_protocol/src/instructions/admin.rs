use anchor_lang::prelude::*;
use anchor_spl::token::Token;
use crate::state::{ProtocolConfig, GeoPremium, Warden};
use crate::ArkhamErrorCode;

/// Initializes the protocol configuration with default parameters
/// This must be called once before any other protocol operations
pub fn initialize_protocol_config_handler(
    ctx: Context<InitializeProtocolConfig>,
    base_rate_per_mb: u64,
    protocol_fee_bps: u16,
    tier_thresholds: [u64; 3],
    tier_multipliers: [u16; 3],
    tokens_per_5gb: u64,
    geo_premiums: Vec<GeoPremium>,
    oracle_authority: Pubkey,
) -> Result<()> {
    let protocol_config = &mut ctx.accounts.protocol_config;
    
    // Validate parameters
    require!(protocol_fee_bps <= 10000, ArkhamErrorCode::InvalidFeeBps);
    require!(
        tier_thresholds[0] <= tier_thresholds[1] && tier_thresholds[1] <= tier_thresholds[2],
        ArkhamErrorCode::InvalidTierThresholds
    );
    
    for &multiplier in &tier_multipliers {
        require!(multiplier <= 50000, ArkhamErrorCode::InvalidTierMultiplier);
    }

    // Initialize all fields
    protocol_config.authority = ctx.accounts.authority.key();
    protocol_config.treasury = ctx.accounts.treasury.key();
    protocol_config.arkham_token_mint = Pubkey::default(); // Will be set later via initialize_arkham_mint
    protocol_config.oracle_authority = oracle_authority; // Set the new oracle authority
    protocol_config.base_rate_per_mb = base_rate_per_mb;
    protocol_config.protocol_fee_bps = protocol_fee_bps;
    protocol_config.tier_thresholds = tier_thresholds;
    protocol_config.tier_multipliers = tier_multipliers;
    protocol_config.tokens_per_5gb = tokens_per_5gb;
    protocol_config.geo_premiums = geo_premiums;
    protocol_config.reputation_updater = ctx.accounts.authority.key(); // Default to authority

    emit!(ProtocolConfigInitialized {
        authority: ctx.accounts.authority.key(),
        base_rate_per_mb,
        protocol_fee_bps,
    });

    Ok(())
}

/// Updates protocol configuration parameters
/// Only callable by the protocol authority
pub fn update_protocol_config_handler(
    ctx: Context<UpdateProtocolConfig>,
    new_base_rate_per_mb: Option<u64>,
    new_protocol_fee_bps: Option<u16>,
    new_tier_thresholds: Option<[u64; 3]>,
    new_tier_multipliers: Option<[u16; 3]>,
    new_tokens_per_5gb: Option<u64>,
    new_geo_premiums: Option<Vec<GeoPremium>>,
    new_reputation_updater: Option<Pubkey>,
    new_oracle_authority: Option<Pubkey>,
) -> Result<()> {
    let protocol_config = &mut ctx.accounts.protocol_config;
    
    // Verify the caller is the protocol authority
    require!(
        ctx.accounts.authority.key() == protocol_config.authority,
        ArkhamErrorCode::UnauthorizedAdminAction
    );

    // Update parameters if provided
    if let Some(rate) = new_base_rate_per_mb {
        protocol_config.base_rate_per_mb = rate;
    }
    
    if let Some(fee_bps) = new_protocol_fee_bps {
        require!(fee_bps <= 10000, ArkhamErrorCode::InvalidFeeBps); // Max 100%
        protocol_config.protocol_fee_bps = fee_bps;
    }
    
    if let Some(thresholds) = new_tier_thresholds {
        // Verify thresholds are in ascending order
        require!(
            thresholds[0] <= thresholds[1] && thresholds[1] <= thresholds[2],
            ArkhamErrorCode::InvalidTierThresholds
        );
        protocol_config.tier_thresholds = thresholds;
    }
    
    if let Some(multipliers) = new_tier_multipliers {
        // Verify multipliers are reasonable (max 5x = 50,000 basis points)
        for &multiplier in &multipliers {
            require!(multiplier <= 50000, ArkhamErrorCode::InvalidTierMultiplier);
        }
        protocol_config.tier_multipliers = multipliers;
    }
    
    if let Some(tokens) = new_tokens_per_5gb {
        protocol_config.tokens_per_5gb = tokens;
    }
    
    if let Some(geo_premiums) = new_geo_premiums {
        // Verify no duplicate regions
        let mut region_codes: Vec<u8> = geo_premiums.iter().map(|gp| gp.region_code).collect();
        region_codes.sort();
        region_codes.dedup();
        require!(
            region_codes.len() == geo_premiums.len(),
            ArkhamErrorCode::DuplicateRegionCode
        );
        
        // Verify premium values are reasonable (max 500% = 50,000 basis points)
        for premium in &geo_premiums {
            require!(premium.premium_bps <= 50000, ArkhamErrorCode::InvalidGeoPremium);
        }
        
        protocol_config.geo_premiums = geo_premiums;
    }

    if let Some(updater) = new_reputation_updater {
        protocol_config.reputation_updater = updater;
    }

    if let Some(oracle) = new_oracle_authority {
        protocol_config.oracle_authority = oracle;
    }

    emit!(ProtocolConfigUpdated {
        authority: ctx.accounts.authority.key(),
        new_base_rate_per_mb: new_base_rate_per_mb,
        new_protocol_fee_bps: new_protocol_fee_bps,
        new_tier_thresholds: new_tier_thresholds,
        new_tier_multipliers: new_tier_multipliers,
        new_tokens_per_5gb: new_tokens_per_5gb,
    });

    Ok(())
}

/// Initializes the ARKHAM token mint
/// Only callable by the protocol authority
pub fn initialize_arkham_mint_handler(ctx: Context<InitializeArkhamMint>) -> Result<()> {
    let protocol_config = &mut ctx.accounts.protocol_config;
    let mint = &mut ctx.accounts.arkham_mint;
    
    // Verify the caller is the protocol authority
    require!(
        ctx.accounts.authority.key() == protocol_config.authority,
        ArkhamErrorCode::UnauthorizedAdminAction
    );

    // Verify the mint hasn't been initialized yet
    require!(
        protocol_config.arkham_token_mint == Pubkey::default(),
        ArkhamErrorCode::TokenMintAlreadyInitialized
    );

    // Mint initialization will be handled by Anchor's mint constraints
    // The mint authority is set up in the InitializeArkhamMint context
    // The bump is automatically handled by Anchor's init constraint

    // Update the protocol config with the new mint address
    protocol_config.arkham_token_mint = mint.key();

    emit!(ArkhamMintInitialized {
        authority: ctx.accounts.authority.key(),
        mint: mint.key(),
    });

    Ok(())
}

/// Distributes bootstrap subsidies to Wardens
/// This is the mechanism to attract early participants during the first 6 months
pub fn distribute_subsidies_handler(
    ctx: Context<DistributeSubsidies>,
    warden_keys: Vec<Pubkey>,
    subsidy_amounts: Vec<u64>,
) -> Result<()> {
    let protocol_config = &ctx.accounts.protocol_config;
    let treasury = &mut ctx.accounts.treasury;
    
    // Verify the caller is the protocol authority
    require!(
        ctx.accounts.authority.key() == protocol_config.authority,
        ArkhamErrorCode::UnauthorizedAdminAction
    );

    // Verify that the vectors have the same length
    require!(
        warden_keys.len() == subsidy_amounts.len(),
        ArkhamErrorCode::InvalidSubsidyDistribution
    );

    // Verify that we're not distributing more than available in treasury
    let total_subsidy: u64 = subsidy_amounts.iter().map(|&x| x).sum();
    
    require!(
        treasury.amount >= total_subsidy,
        ArkhamErrorCode::InsufficientTreasuryBalance
    );

    // Process each subsidy distribution
    for (i, _warden_key) in warden_keys.iter().enumerate() {
        // Load the warden account to update pending claims
        // In a real implementation, this would use CPI to update warden pending claims
        // For this implementation, we're emitting an event to indicate the intended distribution
        let _subsidy_amount = subsidy_amounts[i];
        
        // NOTE: In a real implementation, we'd need to load each warden account
        // and update their pending_claims balance using CPI
        // For this version, we're emitting an event to indicate the intended distribution
    }

    emit!(SubsidiesDistributed {
        authority: ctx.accounts.authority.key(),
        warden_count: warden_keys.len() as u32,
        total_amount: total_subsidy,
    });

    Ok(())
}

/// Updates a Warden's Peer ID. Only callable by the protocol authority.
/// This is a developer/admin tool to fix registration errors.
pub fn update_warden_peer_id_handler(ctx: Context<UpdateWardenPeerId>, new_peer_id: String) -> Result<()> {
    let protocol_config = &ctx.accounts.protocol_config;
    let warden = &mut ctx.accounts.warden;

    // Verify the caller is the protocol authority
    require!(
        ctx.accounts.authority.key() == protocol_config.authority,
        ArkhamErrorCode::UnauthorizedAdminAction
    );

    // Basic validation for Peer ID format
    require!(
        new_peer_id.starts_with("12D3KooW"),
        ArkhamErrorCode::InvalidPeerId
    );
    require!(
        new_peer_id.len() > 40 && new_peer_id.len() < 60,
        ArkhamErrorCode::InvalidPeerId
    );

    let old_peer_id = warden.peer_id.clone();
    warden.peer_id = new_peer_id.clone();

    emit!(WardenPeerIdUpdated {
        warden_authority: warden.authority,
        old_peer_id,
        new_peer_id,
    });

    Ok(())
}

/// Migrates a Warden account with corrupted PeerId field by fixing the Borsh string length prefix
/// This is a one-time migration for accounts where the length prefix is incorrect but the data is intact
pub fn migrate_warden_peer_id_handler(ctx: Context<MigrateWardenPeerId>) -> Result<()> {
    let warden_account = &ctx.accounts.warden;
    let authority = &ctx.accounts.authority;

    // Get raw account data
    let data = warden_account.try_borrow_data()?;

    // Verify we have enough data for a Warden account
    require!(data.len() >= 200, ArkhamErrorCode::InvalidPeerId);

    // Verify discriminator matches Warden account type
    // Discriminator bytes from IDL: [73, 11, 82, 46, 202, 0, 179, 133]
    let expected_discriminator: [u8; 8] = [73, 11, 82, 46, 202, 0, 179, 133];
    require!(
        &data[0..8] == expected_discriminator,
        ArkhamErrorCode::InvalidPeerId
    );

    // Extract and verify authority (bytes 8-40)
    let stored_authority_bytes = &data[8..40];
    let stored_authority = Pubkey::try_from(stored_authority_bytes)
        .map_err(|_| ArkhamErrorCode::UnauthorizedWardenUpdate)?;

    require!(
        stored_authority == authority.key(),
        ArkhamErrorCode::UnauthorizedWardenUpdate
    );

    msg!("Verified authority: {}", stored_authority);

    // PeerId field starts at offset 40 (after discriminator + authority)
    // Structure: 4-byte length prefix (little-endian u32) + UTF-8 string bytes
    let peer_id_offset = 40;
    let length_bytes = &data[peer_id_offset..peer_id_offset + 4];
    let stored_length = u32::from_le_bytes([
        length_bytes[0],
        length_bytes[1],
        length_bytes[2],
        length_bytes[3],
    ]);

    msg!("Current PeerId length prefix: {}", stored_length);

    // The actual PeerId string data starts at offset 44
    let peer_id_data_offset = peer_id_offset + 4;

    // Find the actual string length by detecting boundaries
    // Valid PeerIDs start with "12D3KooW" and are typically 40-53 characters
    let max_peer_id_length = 60; // Safety bound
    let search_end = std::cmp::min(peer_id_data_offset + max_peer_id_length, data.len());
    let peer_id_slice = &data[peer_id_data_offset..search_end];

    // Strategy 1: Find boundary by looking for StakeToken enum (should be 0, 1, or 2)
    let mut actual_length = 0;
    for i in 40..max_peer_id_length {
        let test_offset = peer_id_data_offset + i;
        if test_offset >= data.len() {
            break;
        }

        // Check if this position could be the start of StakeToken enum
        let potential_stake_token = data[test_offset];

        // StakeToken enum values: Sol = 0, Usdc = 1, Usdt = 2
        if potential_stake_token <= 2 {
            // Validate by checking if we have a valid PeerId up to this point
            if i <= peer_id_slice.len() {
                let potential_peer_id = &peer_id_slice[..i];
                if let Ok(peer_id_str) = std::str::from_utf8(potential_peer_id) {
                    // Valid PeerIDs must start with "12D3KooW" and be at least 40 chars
                    if peer_id_str.starts_with("12D3KooW") && peer_id_str.len() >= 40 && peer_id_str.len() <= 60 {
                        actual_length = i;
                        msg!("Found PeerId boundary at offset {} (length: {})", test_offset, actual_length);
                        break;
                    }
                }
            }
        }
    }

    // Strategy 2: If boundary detection failed, try common PeerId lengths
    if actual_length == 0 {
        msg!("Boundary detection failed, trying common PeerId lengths");
        for test_len in [44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 42, 43, 54, 55, 56] {
            if test_len <= peer_id_slice.len() {
                if let Ok(peer_id_str) = std::str::from_utf8(&peer_id_slice[..test_len]) {
                    if peer_id_str.starts_with("12D3KooW") && peer_id_str.len() >= 40 {
                        actual_length = test_len;
                        msg!("Using common PeerId length: {}", actual_length);
                        break;
                    }
                }
            }
        }
    }

    // Validate we found a valid length
    require!(actual_length > 0, ArkhamErrorCode::InvalidPeerId);
    require!(actual_length >= 40, ArkhamErrorCode::InvalidPeerId);
    require!(actual_length <= 60, ArkhamErrorCode::InvalidPeerId);

    // Extract and validate the PeerId string
    let peer_id_bytes = &peer_id_slice[..actual_length];
    let peer_id_str = std::str::from_utf8(peer_id_bytes)
        .map_err(|_| ArkhamErrorCode::InvalidPeerId)?;

    // Final validation: must be a valid libp2p PeerId format
    require!(
        peer_id_str.starts_with("12D3KooW"),
        ArkhamErrorCode::InvalidPeerId
    );
    require!(
        peer_id_str.len() >= 40 && peer_id_str.len() <= 60,
        ArkhamErrorCode::InvalidPeerId
    );

    msg!("Detected PeerId: {}", peer_id_str);
    msg!("Actual length: {} bytes", actual_length);

    // Only update if the length prefix is incorrect
    if stored_length != actual_length as u32 {
        // Clone the string before dropping the borrow
        let peer_id_clone = peer_id_str.to_string();

        drop(data); // Drop the immutable borrow before getting mutable

        // Get mutable reference and update the length prefix
        let mut data_mut = warden_account.try_borrow_mut_data()?;

        // Write the correct length as little-endian u32
        let correct_length_bytes = (actual_length as u32).to_le_bytes();
        data_mut[peer_id_offset..peer_id_offset + 4].copy_from_slice(&correct_length_bytes);

        msg!(
            "✓ Fixed PeerId length prefix: {} -> {}",
            stored_length,
            actual_length
        );

        // Emit event for audit trail
        emit!(WardenPeerIdMigrated {
            warden_authority: authority.key(),
            peer_id: peer_id_clone,
            old_length: stored_length,
            new_length: actual_length as u32,
        });

        msg!("Migration completed successfully");
    } else {
        msg!("PeerId length prefix is already correct ({}), no migration needed", stored_length);
    }

    Ok(())
}


// Account contexts:

#[derive(Accounts)]
pub struct UpdateProtocolConfig<'info> {
    #[account(
        mut,
        seeds = [b"protocol_config"],
        bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(mut)]
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct InitializeArkhamMint<'info> {
    #[account(
        init,
        seeds = [b"arkham_mint"],
        bump,
        payer = authority,
        mint::decimals = 9,
        mint::authority = mint_authority,
        mint::freeze_authority = mint_authority,
    )]
    pub arkham_mint: Account<'info, anchor_spl::token::Mint>,

    /// CHECK: Mint authority for the ARKHAM token - this is a PDA controlled by the program
    #[account(
        seeds = [b"arkham", b"mint", b"authority"],
        bump,
    )]
    pub mint_authority: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [b"protocol_config"],
        bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct DistributeSubsidies<'info> {
    #[account(
        seeds = [b"protocol_config"],
        bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        associated_token::mint = arkham_mint,
        associated_token::authority = treasury_authority,
    )]
    pub treasury: Account<'info, anchor_spl::token::TokenAccount>,

    pub arkham_mint: Account<'info, anchor_spl::token::Mint>,

    /// CHECK: Treasury authority (e.g., multisig wallet)
    pub treasury_authority: AccountInfo<'info>,

    #[account(mut)]
    pub authority: Signer<'info>,

    // The actual warden accounts would need to be loaded dynamically
    // This is simplified for the core implementation
}

#[derive(Accounts)]
pub struct InitializeProtocolConfig<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + // discriminator
                32 + // authority
                32 + // treasury
                32 + // arkham_token_mint
                32 + // oracle_authority
                8 +  // base_rate_per_mb
                2 +  // protocol_fee_bps
                (8 * 3) + // tier_thresholds
                (2 * 3) + // tier_multipliers
                8 +  // tokens_per_5gb
                4 + (10 * (1 + 2)) + // geo_premiums vec (assume max 10 regions)
                32, // reputation_updater
        seeds = [b"protocol_config"],
        bump
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    /// CHECK: Treasury can be any account (e.g., multisig)
    pub treasury: AccountInfo<'info>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdateWardenPeerId<'info> {
    #[account(seeds = [b"protocol_config"], bump)]
    pub protocol_config: Account<'info, ProtocolConfig>,

    /// The protocol authority must sign to authorize this change.
    #[account(mut)]
    pub authority: Signer<'info>,

    /// The warden account to be updated.
    #[account(mut)]
    pub warden: Account<'info, Warden>,
}

/// Account context for migrating a Warden's corrupted PeerId field
#[derive(Accounts)]
pub struct MigrateWardenPeerId<'info> {
    /// The warden account to migrate - using AccountInfo to avoid deserialization errors
    /// CHECK: We manually verify the PDA seeds and authority without deserializing the account
    #[account(
        mut,
        seeds = [b"warden", authority.key().as_ref()],
        bump,
    )]
    pub warden: AccountInfo<'info>,

    /// The warden's authority must sign to authorize this migration
    /// Only the warden owner can migrate their own account
    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}


// Events:

#[event]
pub struct ProtocolConfigUpdated {
    pub authority: Pubkey,
    pub new_base_rate_per_mb: Option<u64>,
    pub new_protocol_fee_bps: Option<u16>,
    pub new_tier_thresholds: Option<[u64; 3]>,
    pub new_tier_multipliers: Option<[u16; 3]>,
    pub new_tokens_per_5gb: Option<u64>,
}

#[event]
pub struct ArkhamMintInitialized {
    pub authority: Pubkey,
    pub mint: Pubkey,
}

#[event]
pub struct SubsidiesDistributed {
    pub authority: Pubkey,
    pub warden_count: u32,
    pub total_amount: u64,
}

#[event]
pub struct ProtocolConfigInitialized {
    pub authority: Pubkey,
    pub base_rate_per_mb: u64,
    pub protocol_fee_bps: u16,
}

#[event]
pub struct WardenPeerIdUpdated {
    pub warden_authority: Pubkey,
    pub old_peer_id: String,
    pub new_peer_id: String,
}

/// Event emitted when a Warden's PeerId is successfully migrated
#[event]
pub struct WardenPeerIdMigrated {
    /// The authority (owner) of the warden
    pub warden_authority: Pubkey,
    /// The corrected PeerId string
    pub peer_id: String,
    /// The incorrect length that was stored before migration
    pub old_length: u32,
    /// The correct length after migration
    pub new_length: u32,
}


/// Handler for force closing the protocol config
/// This manually checks authority and transfers lamports without deserializing
pub fn close_protocol_config_handler(ctx: Context<CloseProtocolConfig>) -> Result<()> {
    let protocol_config = &ctx.accounts.protocol_config;
    let receiver = &ctx.accounts.receiver;
    
    // Manual authority check by reading raw account data
    // The authority pubkey is stored at bytes 8-40 (after 8-byte discriminator)
    let data = protocol_config.try_borrow_data()?;
    
    // Check if account has enough data
    require!(data.len() >= 40, ArkhamErrorCode::UnauthorizedAdminAction);
    
    // Extract authority from raw bytes
    let stored_authority_bytes = &data[8..40];
    let stored_authority = Pubkey::try_from(stored_authority_bytes)
        .map_err(|_| ArkhamErrorCode::UnauthorizedAdminAction)?;
    
    // Verify the signer matches the stored authority
    require!(
        stored_authority == ctx.accounts.authority.key(),
        ArkhamErrorCode::UnauthorizedAdminAction
    );
    
    // Transfer all lamports to receiver
    let protocol_config_lamports = protocol_config.lamports();
    **protocol_config.try_borrow_mut_lamports()? = 0;
    **receiver.try_borrow_mut_lamports()? = receiver
        .lamports()
        .checked_add(protocol_config_lamports)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;
    
    // Zero out the data
    let mut data_mut = protocol_config.try_borrow_mut_data()?;
    data_mut.fill(0);
    
    msg!("Protocol config force closed. Lamports returned: {}", protocol_config_lamports);
    
    Ok(())
}

/// For accounts that have the old structure (without oracle_authority), we need a migration function
pub fn migrate_protocol_config_handler(ctx: Context<MigrateProtocolConfig>) -> Result<()> {
    // Verify the caller is the protocol authority
    require!(
        ctx.accounts.authority.key() == ctx.accounts.protocol_config.authority,
        ArkhamErrorCode::UnauthorizedAdminAction
    );

    // Set the new oracle authority
    ctx.accounts.protocol_config.oracle_authority = ctx.accounts.new_oracle_authority.key();

    emit!(ProtocolConfigUpdated {
        authority: ctx.accounts.authority.key(),
        new_base_rate_per_mb: None,
        new_protocol_fee_bps: None,
        new_tier_thresholds: None,
        new_tier_multipliers: None,
        new_tokens_per_5gb: None,
    });

    Ok(())
}

// Add this new context to your admin.rs file
// This replaces the existing CloseProtocolConfig context

/// Force closes the protocol config account without deserializing
/// This is useful for cleaning up accounts with incompatible data structures
#[derive(Accounts)]
pub struct CloseProtocolConfig<'info> {
    /// Protocol config account to close - using AccountInfo to avoid deserialization
    /// CHECK: We manually verify the PDA and authority without deserializing
    #[account(
        mut,
        seeds = [b"protocol_config"],
        bump,
    )]
    pub protocol_config: AccountInfo<'info>,

    /// The authority that should match the one stored in the account
    #[account(mut)]
    pub authority: Signer<'info>,

    /// Receiver of the rent (can be the authority or another account)
    /// CHECK: Receiver of rent
    #[account(mut)]
    pub receiver: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}


#[derive(Accounts)]
pub struct MigrateProtocolConfig<'info> {
    #[account(
        mut,
        seeds = [b"protocol_config"],
        bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(mut)]
    pub authority: Signer<'info>,
    
    /// New oracle authority to set
    /// CHECK: Just a public key, doesn't need to sign
    pub new_oracle_authority: AccountInfo<'info>,
}
