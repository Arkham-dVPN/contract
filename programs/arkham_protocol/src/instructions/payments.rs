use anchor_lang::{prelude::*, system_program};
use anchor_spl::token::{self, Mint, Token, TokenAccount, MintTo};
use anchor_lang::solana_program::sysvar::instructions::ID as INSTRUCTIONS_SYSVAR_ID;
use crate::state::{Seeker, Warden, Connection, ProtocolConfig, BandwidthProof};
use crate::ArkhamErrorCode;

const ESCROW_BUFFER_BPS: u16 = 1000; // 10% buffer

/// Deposits SOL into a Seeker's escrow account
pub fn deposit_escrow_handler(
    ctx: Context<DepositEscrow>,
    amount: u64,
    use_private: bool,
) -> Result<()> {
    let seeker = &mut ctx.accounts.seeker;

    if use_private {
        // TODO: Implement Elusiv CPI for private deposits
        // This requires integrating the Elusiv SDK and performing a CPI
        // to their deposit instruction. For now, we'll return an error.
        return err!(ArkhamErrorCode::PrivatePaymentsNotImplemented);
    } else {
        // Public deposit: Transfer SOL from authority to seeker's escrow PDA
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.authority.to_account_info(),
                to: ctx.accounts.seeker_escrow.to_account_info(),
            },
        );
        system_program::transfer(cpi_context, amount)?;

        // Update seeker's escrow balance
        seeker.escrow_balance = seeker.escrow_balance
            .checked_add(amount)
            .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;
    }

    emit!(EscrowDeposited {
        authority: seeker.authority,
        amount,
        use_private,
    });

    Ok(())
}

/// Starts a new VPN connection between a Seeker and Warden
pub fn start_connection_handler(
    ctx: Context<StartConnection>,
    estimated_mb: u64,
) -> Result<()> {
    let config = &ctx.accounts.protocol_config;
    let warden = &mut ctx.accounts.warden;
    let seeker = &mut ctx.accounts.seeker;
    let connection = &mut ctx.accounts.connection;
    let clock = Clock::get()?;

    // 1. Calculate effective rate per MB
    let base_rate = config.base_rate_per_mb;
    
    // Get geographic premium for this warden's region
    let geo_premium_bps = config.geo_premiums
        .iter()
        .find(|gp| gp.region_code == warden.region_code)
        .map(|gp| gp.premium_bps)
        .unwrap_or(0);

    // Get tier multiplier
    let tier_multiplier = match warden.tier {
        crate::state::Tier::Bronze => config.tier_multipliers[0],
        crate::state::Tier::Silver => config.tier_multipliers[1],
        crate::state::Tier::Gold => config.tier_multipliers[2],
    };

    // Calculate: rate = base * (1 + geo_premium) * tier_multiplier
    // All in basis points for precision
    let rate_with_geo = (base_rate as u128)
        .checked_mul((10000 + geo_premium_bps) as u128)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?
        .checked_div(10000)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)? as u64;

    let rate_per_mb = (rate_with_geo as u128)
        .checked_mul(tier_multiplier as u128)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?
        .checked_div(10000)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)? as u64;

    // 2. Calculate total escrow needed (with 10% buffer)
    let base_escrow = (estimated_mb as u128)
        .checked_mul(rate_per_mb as u128)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)? as u64;
    
    let escrow_needed = (base_escrow as u128)
        .checked_mul((10000 + ESCROW_BUFFER_BPS) as u128)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?
        .checked_div(10000)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)? as u64;

    // 3. Verify sufficient escrow balance
    require!(
        seeker.escrow_balance >= escrow_needed,
        ArkhamErrorCode::InsufficientEscrow
    );

    // 4. Initialize Connection account
    connection.seeker = seeker.key();
    connection.warden = warden.key();
    connection.started_at = clock.unix_timestamp;
    connection.last_proof_at = clock.unix_timestamp;
    connection.bandwidth_consumed = 0;
    connection.bandwidth_proofs = Vec::new();
    connection.amount_escrowed = escrow_needed;
    connection.amount_paid = 0;
    connection.rate_per_mb = rate_per_mb;
    connection.warden_multiplier = tier_multiplier;

    // 5. Move funds from seeker escrow to connection escrow
    seeker.escrow_balance = seeker.escrow_balance
        .checked_sub(escrow_needed)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;

    // 6. Update active connection counters
    seeker.active_connections = seeker.active_connections
        .checked_add(1)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;
    
    warden.active_connections = warden.active_connections
        .checked_add(1)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;

    emit!(ConnectionStarted {
        seeker: seeker.key(),
        warden: warden.key(),
        estimated_mb,
        rate_per_mb,
        escrow_amount: escrow_needed,
    });

    Ok(())
}

/// Submits a bandwidth proof and processes micropayment
pub fn submit_bandwidth_proof_handler(
    ctx: Context<SubmitBandwidthProof>,
    mb_consumed: u64,
    seeker_signature: [u8; 64],
    warden_signature: [u8; 64],
) -> Result<()> {
    // Get the connection key before we mutably borrow the connection
    let connection_key = ctx.accounts.connection.key();
    let warden_key = ctx.accounts.warden.key();
    let seeker_key = ctx.accounts.seeker.key();
    
    let connection = &mut ctx.accounts.connection;
    let warden = &mut ctx.accounts.warden;
    let seeker = &ctx.accounts.seeker;
    let config = &ctx.accounts.protocol_config;
    let clock = Clock::get()?;

    // 1. Validate the proof using bandwidth module helpers
    crate::instructions::bandwidth::validate_bandwidth_proof(
        mb_consumed,
        clock.unix_timestamp,
        clock.unix_timestamp,
        &seeker_signature,
        &warden_signature,
    )?;

    // 2. Create the deterministic message and verify Ed25519 signatures
    let proof_message = crate::instructions::bandwidth::create_proof_message(
        &connection_key,
        mb_consumed,
        clock.unix_timestamp,
    );
    
    // REAL Ed25519 VERIFICATION using instruction introspection
    crate::instructions::bandwidth::verify_dual_signatures(
        &ctx.accounts.instructions_sysvar,
        &proof_message,
        &seeker_signature,
        &seeker.authority,
        &warden_signature,
        &warden.authority,
    )?;

    // 3. Check for duplicate proofs (prevent replay attacks)
    let proof_hash = crate::instructions::bandwidth::hash_bandwidth_proof(
        &connection_key,
        mb_consumed,
        clock.unix_timestamp,
        &seeker_signature,
        &warden_signature,
    );
    
    // Check if this proof hash already exists in our history
    for existing_proof in &connection.bandwidth_proofs {
        let existing_hash = crate::instructions::bandwidth::hash_bandwidth_proof(
            &connection_key,
            existing_proof.mb_consumed,
            existing_proof.timestamp,
            &existing_proof.seeker_signature,
            &existing_proof.warden_signature,
        );
        
        require!(
            proof_hash != existing_hash,
            crate::instructions::bandwidth::BandwidthError::InvalidSignature
        );
    }

    // 4. Anomaly detection (optional - flag suspicious claims)
    if connection.bandwidth_proofs.len() >= 3 {
        let historical: Vec<u64> = connection.bandwidth_proofs
            .iter()
            .map(|p| p.mb_consumed)
            .collect();
        
        let expected = crate::instructions::bandwidth::calculate_expected_bandwidth(&historical, 5);
        
        if crate::instructions::bandwidth::detect_bandwidth_anomaly(mb_consumed, expected, 3.0) {
            msg!("Warning: Anomalous bandwidth detected. Expected: {}, Claimed: {}", expected, mb_consumed);
            // Continue processing but log the warning for reputation system
        }
    }

    // 5. Calculate payment amount
    let payment_amount = (mb_consumed as u128)
        .checked_mul(connection.rate_per_mb as u128)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)? as u64;

    // 6. Verify payment doesn't exceed available escrow
    let new_total_paid = connection.amount_paid
        .checked_add(payment_amount)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;
    
    require!(
        new_total_paid <= connection.amount_escrowed,
        ArkhamErrorCode::InsufficientConnectionEscrow
    );

    // 7. Transfer payment to warden's pending claims
    warden.pending_claims = warden.pending_claims
        .checked_add(payment_amount)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;

    // 8. Update connection bandwidth and payment tracking
    connection.bandwidth_consumed = connection.bandwidth_consumed
        .checked_add(mb_consumed)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;
    
    connection.amount_paid = new_total_paid;

    // 9. Update warden statistics
    warden.total_bandwidth_served = warden.total_bandwidth_served
        .checked_add(mb_consumed)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;
    
    warden.total_earnings = warden.total_earnings
        .checked_add(payment_amount)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;

    // 10. Calculate and add ARKHAM token allocation
    let tokens_per_mb = config.tokens_per_5gb / 5120;
    let arkham_earned = (mb_consumed as u128)
        .checked_mul(tokens_per_mb as u128)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)? as u64;
    
    warden.arkham_tokens_earned = warden.arkham_tokens_earned
        .checked_add(arkham_earned)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;

    // 11. Add proof to bandwidth_proofs vector (limit to last 10)
    let proof = BandwidthProof {
        timestamp: clock.unix_timestamp,
        mb_consumed,
        seeker_signature,
        warden_signature,
    };

    if connection.bandwidth_proofs.len() >= 10 {
        connection.bandwidth_proofs.remove(0);
    }
    connection.bandwidth_proofs.push(proof);

    // 12. Update last proof timestamp
    connection.last_proof_at = clock.unix_timestamp;

    // 13. Update warden's last active timestamp
    warden.last_active = clock.unix_timestamp;

    emit!(BandwidthProofSubmitted {
        connection: connection_key,
        mb_consumed,
        payment_amount,
        arkham_earned,
    });

    Ok(())
}

/// Ends a VPN connection and settles final amounts
pub fn end_connection_handler(ctx: Context<EndConnection>) -> Result<()> {
    let connection = &ctx.accounts.connection;
    let warden = &mut ctx.accounts.warden;
    let seeker = &mut ctx.accounts.seeker;

    // 1. Calculate unused escrow
    let unused_escrow = connection.amount_escrowed
        .checked_sub(connection.amount_paid)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;

    // 2. Refund unused escrow to seeker
    if unused_escrow > 0 {
        seeker.escrow_balance = seeker.escrow_balance
            .checked_add(unused_escrow)
            .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;
    }

    // 3. Update warden reputation (increment successful connections)
    warden.successful_connections = warden.successful_connections
        .checked_add(1)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;

    // 4. Decrement active connection counters
    seeker.active_connections = seeker.active_connections
        .checked_sub(1)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;
    
    warden.active_connections = warden.active_connections
        .checked_sub(1)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;

    // 5. Update seeker's total consumption and spending
    seeker.total_bandwidth_consumed = seeker.total_bandwidth_consumed
        .checked_add(connection.bandwidth_consumed)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;
    
    seeker.total_spent = seeker.total_spent
        .checked_add(connection.amount_paid)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;

    emit!(ConnectionEnded {
        seeker: seeker.key(),
        warden: warden.key(),
        bandwidth_consumed: connection.bandwidth_consumed,
        total_paid: connection.amount_paid,
        refunded: unused_escrow,
    });

    // Note: Connection account will be closed automatically via close constraint
    Ok(())
}

/// Claims accumulated earnings for a Warden
pub fn claim_earnings_handler(
    ctx: Context<ClaimEarnings>,
    use_private: bool,
) -> Result<()> {
    let warden = &mut ctx.accounts.warden;

    // 1. Verify there are earnings to claim
    require!(
        warden.pending_claims > 0,
        ArkhamErrorCode::NothingToClaim
    );

    let amount = warden.pending_claims;

    if use_private {
        // TODO: Implement Elusiv CPI for private withdrawals
        return err!(ArkhamErrorCode::PrivatePaymentsNotImplemented);
    } else {
        // Public claim: Transfer from protocol vault to warden's authority
        let vault_seeds = &[b"sol_vault".as_ref(), &[ctx.bumps.sol_vault]];
        let signer_seeds = &[&vault_seeds[..]];

        let cpi_context = CpiContext::new_with_signer(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.sol_vault.to_account_info(),
                to: ctx.accounts.authority.to_account_info(),
            },
            signer_seeds,
        );
        system_program::transfer(cpi_context, amount)?;
    }

    // 2. Reset pending claims
    warden.pending_claims = 0;

    emit!(EarningsClaimed {
        authority: warden.authority,
        amount,
        use_private,
    });

    Ok(())
}

/// Claims earned ARKHAM tokens
pub fn claim_arkham_tokens_handler(ctx: Context<ClaimArkhamTokens>) -> Result<()> {
    let warden = &mut ctx.accounts.warden;
    let config = &ctx.accounts.protocol_config;
    let amount = warden.arkham_tokens_earned;

    // 1. Verify there are tokens to claim
    require!(
        amount > 0,
        ArkhamErrorCode::NothingToClaim
    );

    // 2. Verify ARKHAM mint is initialized
    require!(
        config.arkham_token_mint != Pubkey::default(),
        ArkhamErrorCode::TokenMintNotInitialized
    );

    // 3. Mint tokens to warden's token account using PDA authority
    let authority_bump = ctx.bumps.mint_authority;
    
    let seeds = &[
        b"arkham".as_ref(),
        b"mint".as_ref(),
        b"authority".as_ref(),
        &[authority_bump]
    ];
    let signer_seeds = &[&seeds[..]];

    let cpi_accounts = MintTo {
        mint: ctx.accounts.arkham_mint.to_account_info(),
        to: ctx.accounts.warden_arkham_token_account.to_account_info(),
        authority: ctx.accounts.mint_authority.to_account_info(),
    };

    let cpi_program = ctx.accounts.token_program.to_account_info();
    let cpi_context = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer_seeds);
    token::mint_to(cpi_context, amount)?;

    // 4. Reset earned tokens counter
    warden.arkham_tokens_earned = 0;

    emit!(TokensClaimed {
        authority: warden.authority,
        amount,
    });

    Ok(())
}

// Account contexts:

#[derive(Accounts)]
pub struct DepositEscrow<'info> {
    #[account(
        mut,
        seeds = [b"seeker", authority.key().as_ref()],
        bump,
        has_one = authority
    )]
    pub seeker: Account<'info, Seeker>,

    #[account(mut)]
    pub authority: Signer<'info>,

    /// CHECK: Seeker's escrow PDA
    #[account(mut, seeds = [b"seeker_escrow", authority.key().as_ref()], bump)]
    pub seeker_escrow: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct StartConnection<'info> {
    #[account(
        init,
        payer = seeker_authority,
        space = 8 + 32 + 32 + 8 + 8 + 8 + 4 + (10 * (8 + 8 + 64 + 64)) + 8 + 8 + 8 + 2,
        seeds = [b"connection", seeker.key().as_ref(), warden.key().as_ref()],
        bump
    )]
    pub connection: Account<'info, Connection>,

    #[account(mut)]
    pub seeker: Account<'info, Seeker>,

    #[account(mut)]
    pub warden: Account<'info, Warden>,

    #[account(mut)]
    pub seeker_authority: Signer<'info>,

    pub protocol_config: Account<'info, ProtocolConfig>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SubmitBandwidthProof<'info> {
    #[account(
        mut,
        seeds = [b"connection", connection.seeker.as_ref(), connection.warden.as_ref()],
        bump,
        has_one = warden,
        has_one = seeker
    )]
    pub connection: Account<'info, Connection>,

    #[account(mut)]
    pub warden: Account<'info, Warden>,

    #[account(mut)]
    pub seeker: Account<'info, Seeker>,

    pub protocol_config: Account<'info, ProtocolConfig>,

    /// CHECK: Instructions sysvar for Ed25519 verification
    #[account(address = INSTRUCTIONS_SYSVAR_ID)]
    pub instructions_sysvar: AccountInfo<'info>,

    /// Either seeker or warden can submit proofs
    pub submitter: Signer<'info>,
}

#[derive(Accounts)]
pub struct EndConnection<'info> {
    #[account(
        mut,
        seeds = [b"connection", seeker.key().as_ref(), warden.key().as_ref()],
        bump,
        close = seeker_authority  // Refund rent to seeker
    )]
    pub connection: Account<'info, Connection>,

    #[account(mut)]
    pub seeker: Account<'info, Seeker>,

    #[account(mut)]
    pub warden: Account<'info, Warden>,

    #[account(mut)]
    pub seeker_authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct ClaimEarnings<'info> {
    #[account(
        mut,
        seeds = [b"warden", authority.key().as_ref()],
        bump,
        has_one = authority
    )]
    pub warden: Account<'info, Warden>,

    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(mut, seeds = [b"sol_vault"], bump)]
    pub sol_vault: SystemAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ClaimArkhamTokens<'info> {
    #[account(
        mut,
        seeds = [b"warden", authority.key().as_ref()],
        bump,
        has_one = authority
    )]
    pub warden: Account<'info, Warden>,

    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        seeds = [b"protocol_config"],
        bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        seeds = [b"arkham_mint"],
        bump,
    )]
    pub arkham_mint: Account<'info, Mint>,

    #[account(
        mut,
        associated_token::mint = arkham_mint,
        associated_token::authority = authority,
    )]
    pub warden_arkham_token_account: Account<'info, TokenAccount>,

    /// CHECK: Mint authority for the ARKHAM token - PDA controlled by the program
    #[account(
        seeds = [b"arkham", b"mint", b"authority"],
        bump,
    )]
    pub mint_authority: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, anchor_spl::associated_token::AssociatedToken>,
    pub system_program: Program<'info, System>,
}

// Events:

#[event]
pub struct EscrowDeposited {
    pub authority: Pubkey,
    pub amount: u64,
    pub use_private: bool,
}

#[event]
pub struct ConnectionStarted {
    pub seeker: Pubkey,
    pub warden: Pubkey,
    pub estimated_mb: u64,
    pub rate_per_mb: u64,
    pub escrow_amount: u64,
}

#[event]
pub struct BandwidthProofSubmitted {
    pub connection: Pubkey,
    pub mb_consumed: u64,
    pub payment_amount: u64,
    pub arkham_earned: u64,
}

#[event]
pub struct ConnectionEnded {
    pub seeker: Pubkey,
    pub warden: Pubkey,
    pub bandwidth_consumed: u64,
    pub total_paid: u64,
    pub refunded: u64,
}

#[event]
pub struct EarningsClaimed {
    pub authority: Pubkey,
    pub amount: u64,
    pub use_private: bool,
}

#[event]
pub struct TokensClaimed {
    pub authority: Pubkey,
    pub amount: u64,
}