use anchor_lang::prelude::*;
use anchor_spl::token::Token;
use crate::state::{ProtocolConfig, GeoPremium};
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