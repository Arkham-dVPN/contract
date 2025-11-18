use anchor_lang::{prelude::*, system_program};
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use anchor_lang::solana_program::{
    keccak,
    sysvar::instructions::{load_instruction_at_checked, ID as INSTRUCTIONS_SYSVAR_ID},
    ed25519_program,
};
use crate::state::{Warden, StakeToken, ProtocolConfig, Tier};
use crate::ArkhamErrorCode;

const USD_DECIMALS: u32 = 6;
const SOL_DECIMALS: u32 = 9;
const USDT_DECIMALS: u32 = 6;

pub fn initialize_warden_handler(
    ctx: Context<InitializeWarden>,
    stake_token: StakeToken,
    stake_amount: u64,
    peer_id: String,
    region_code: u8,
    ip_hash: [u8; 32],
    price: u64,           // Price in micro-units (6 decimals) of USD per token
    timestamp: i64,       // Timestamp of the price data
    signature: [u8; 64],  // Ed25519 signature of the price and timestamp by the oracle
) -> Result<()> {
    let config = &ctx.accounts.protocol_config;
    let clock = Clock::get()?;
    let current_timestamp = clock.unix_timestamp;

    // Verify that the price data is recent (within 5 minutes)
    require!(
        current_timestamp - timestamp <= 300, // 5 minutes
        ArkhamErrorCode::StalePrice
    );

    // Create the message that should have been signed (price + timestamp)
    let oracle_message = create_oracle_message(price, timestamp);

    // Verify the signature using instruction introspection
    if let Err(error) = verify_oracle_signature_via_sysvar(
        &ctx.accounts.instructions_sysvar,
        &oracle_message,
        &signature,
        &config.oracle_authority,
        0, // Ed25519 instruction should be at index 0
    ) {
        // Convert the OracleError to ArkhamErrorCode
        return Err(error.into());
    }

    // Calculate USD value of the stake using the provided price
    let stake_value_usd = calculate_stake_value_usd(&stake_token, stake_amount, price)?;

    // Determine the tier based on USD value
    let tier = if stake_value_usd >= config.tier_thresholds[2] {
        Tier::Gold
    } else if stake_value_usd >= config.tier_thresholds[1] {
        Tier::Silver
    } else if stake_value_usd >= config.tier_thresholds[0] {
        Tier::Bronze
    } else {
        return err!(ArkhamErrorCode::InsufficientStake);
    };

    // Transfer stake tokens to the appropriate vault
    match stake_token {
        StakeToken::Sol => {
            let cpi_context = CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.authority.to_account_info(),
                    to: ctx.accounts.sol_vault.to_account_info(),
                },
            );
            system_program::transfer(cpi_context, stake_amount)?;
        }
        StakeToken::Usdc => {
            let cpi_accounts = Transfer {
                from: ctx.accounts.stake_from_account.to_account_info(),
                to: ctx.accounts.usdc_vault.to_account_info(),
                authority: ctx.accounts.authority.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_context = CpiContext::new(cpi_program, cpi_accounts);
            token::transfer(cpi_context, stake_amount)?;
        }
        StakeToken::Usdt => {
            let cpi_accounts = Transfer {
                from: ctx.accounts.stake_from_account.to_account_info(),
                to: ctx.accounts.usdt_vault.to_account_info(),
                authority: ctx.accounts.authority.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_context = CpiContext::new(cpi_program, cpi_accounts);
            token::transfer(cpi_context, stake_amount)?;
        }
    }

    // Initialize the Warden account
    let warden = &mut ctx.accounts.warden;
    warden.authority = ctx.accounts.authority.key();
    warden.peer_id = peer_id;
    warden.stake_token = stake_token;
    warden.stake_amount = stake_amount;
    warden.stake_value_usd = stake_value_usd;
    warden.tier = tier;
    warden.staked_at = current_timestamp;
    warden.unstake_requested_at = None;
    warden.total_bandwidth_served = 0;
    warden.total_earnings = 0;
    warden.pending_claims = 0;
    warden.arkham_tokens_earned = 0;
    warden.reputation_score = 10000; // Start with a perfect score
    warden.successful_connections = 0;
    warden.failed_connections = 0;
    warden.uptime_percentage = 10000; // Start at 100%
    warden.last_active = current_timestamp;
    warden.region_code = region_code;
    warden.ip_hash = ip_hash;
    warden.premium_pool_rank = None;
    warden.active_connections = 0;

    // Emit a registration event
    emit!(WardenRegistered {
        authority: warden.authority,
        tier: warden.tier.clone(),
        stake_amount: warden.stake_amount,
        stake_token: warden.stake_token.clone(),
    });

    Ok(())
}

/// Creates a deterministic message for oracle price signing
/// 
/// The oracle signs: price (8 bytes LE) + timestamp (8 bytes LE)
/// This creates a 16-byte message that is then hashed for signing
/// 
/// # Arguments
/// * `price` - Price in micro-units (6 decimals)
/// * `timestamp` - Unix timestamp of the price data
/// 
/// # Returns
/// * `Vec<u8>` - The deterministic message bytes to be signed (32 bytes after hashing)
pub fn create_oracle_message(price: u64, timestamp: i64) -> Vec<u8> {
    let mut message = Vec::new();
    
    // Add price (8 bytes, little-endian)
    message.extend_from_slice(&price.to_le_bytes());
    
    // Add timestamp (8 bytes, little-endian)
    message.extend_from_slice(&timestamp.to_le_bytes());
    
    // Hash the combined data for a fixed-size message
    // This provides a 32-byte message suitable for Ed25519 signing
    let hash = keccak::hash(&message);
    
    hash.to_bytes().to_vec()
}

/// Verifies oracle Ed25519 signature by checking that an Ed25519Program instruction
/// was included in the same transaction.
/// 
/// This approach uses instruction introspection - the client MUST include
/// an Ed25519Program verification instruction before calling initialize_warden.
/// 
/// # Security Model
/// - Client creates Ed25519Program.createInstructionWithPublicKey() for the oracle signature
/// - This instruction is placed BEFORE the initialize_warden instruction
/// - This function verifies that instruction exists and matches our expected data
/// 
/// # Arguments
/// * `instructions_sysvar` - The Instructions sysvar account
/// * `message` - The message that was signed (hashed price + timestamp)
/// * `signature` - The 64-byte Ed25519 signature from the oracle
/// * `oracle_pubkey` - The oracle's public key (from protocol config)
/// * `instruction_index` - Which instruction index to check (typically 0)
/// 
/// # Returns
/// * `Result<()>` - Ok if signature is valid via Ed25519Program, error otherwise
pub fn verify_oracle_signature_via_sysvar(
    instructions_sysvar: &AccountInfo,
    message: &[u8],
    signature: &[u8; 64],
    oracle_pubkey: &Pubkey,
    instruction_index: u16,
) -> Result<()> {
    // Verify we're actually looking at the Instructions sysvar
    require!(
        instructions_sysvar.key() == INSTRUCTIONS_SYSVAR_ID,
        OracleError::InvalidInstructionsSysvar
    );

    // Load the Ed25519Program instruction at the specified index
    let ed25519_ix = load_instruction_at_checked(
        instruction_index as usize,
        instructions_sysvar,
    ).map_err(|_| OracleError::Ed25519InstructionNotFound)?;

    // Verify it's actually an Ed25519Program instruction
    require!(
        ed25519_ix.program_id == ed25519_program::ID,
        OracleError::InvalidEd25519Instruction
    );

    // Parse the Ed25519Program instruction data
    // Format: [num_signatures: u8, padding: u8, signature_offset: u16, 
    //          signature_instruction_index: u16, public_key_offset: u16,
    //          public_key_instruction_index: u16, message_data_offset: u16,
    //          message_data_size: u16, message_instruction_index: u16,
    //          ...signature(64), ...pubkey(32), ...message]
    
    let data = &ed25519_ix.data;
    require!(
        data.len() >= 2 + 5*2 + 64 + 32 + message.len(),
        OracleError::InvalidEd25519Data
    );

    // Extract signature from instruction data (starts at byte 14)
    let sig_start = 16;
    let sig_end = sig_start + 64;
    let ix_signature = &data[sig_start..sig_end];
    
    // Extract public key (starts after signature)
    let pk_start = sig_end;
    let pk_end = pk_start + 32;
    let ix_pubkey = &data[pk_start..pk_end];
    
    // Extract message (starts after public key)
    let msg_start = pk_end;
    let msg_end = msg_start + message.len();
    require!(
        data.len() >= msg_end,
        OracleError::InvalidEd25519Data
    );
    let ix_message = &data[msg_start..msg_end];

    // Verify the signature matches what we expect
    require!(
        ix_signature == signature,
        OracleError::SignatureMismatch
    );

    // Verify the public key matches the oracle authority
    require!(
        ix_pubkey == oracle_pubkey.to_bytes().as_ref(),
        OracleError::PublicKeyMismatch
    );

    // Verify the message matches (hashed price + timestamp)
    require!(
        ix_message == message,
        OracleError::MessageMismatch
    );

    // If we get here, the Ed25519Program instruction exists and matches our data
    // The Ed25519Program already verified the signature cryptographically
    Ok(())
}

/// Calculates the USD value of a stake using the provided oracle price
fn calculate_stake_value_usd(stake_token: &StakeToken, stake_amount: u64, oracle_price: u64) -> Result<u64> {
    match stake_token {
        StakeToken::Sol => {
            // SOL has 9 decimals, price is in micro-units (6 decimals) per SOL
            // So we need to handle the decimal conversion properly
            let usd_value = (stake_amount as u128)
                .checked_mul(oracle_price as u128)
                .ok_or(ArkhamErrorCode::ArithmeticOverflow)?
                .checked_div(1_000_000_000) // Divide by 10^9 to account for SOL's 9 decimals
                .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;
            Ok(usd_value as u64)
        }
        StakeToken::Usdc => {
            // USDC has 6 decimals, price is in micro-units (6 decimals) per USDC
            // So 1 USDC at $1.00 price = 1_000_000 micro-units
            let usd_value = (stake_amount as u128)
                .checked_mul(oracle_price as u128)
                .ok_or(ArkhamErrorCode::ArithmeticOverflow)?
                .checked_div(1_000_000) // Divide by 10^6 to account for USDC's 6 decimals
                .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;
            Ok(usd_value as u64)
        }
        StakeToken::Usdt => {
            // USDT has 6 decimals, price is in micro-units (6 decimals) per USDT
            // So 1 USDT at $1.00 price = 1_000_000 micro-units
            let usd_value = (stake_amount as u128)
                .checked_mul(oracle_price as u128)
                .ok_or(ArkhamErrorCode::ArithmeticOverflow)?
                .checked_div(1_000_000) // Divide by 10^6 to account for USDT's 6 decimals
                .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;
            Ok(usd_value as u64)
        }
    }
}

/// Initiates the unstaking process with a 7-day cooldown period
pub fn unstake_warden_handler(ctx: Context<UnstakeWarden>) -> Result<()> {
    let warden = &mut ctx.accounts.warden;
    let clock = Clock::get()?;

    // 1. Verify no active connections
    require!(
        warden.active_connections == 0,
        ArkhamErrorCode::HasActiveConnections
    );

    // 2. Verify reputation meets minimum threshold (80%)
    require!(
        warden.reputation_score >= 8000,
        ArkhamErrorCode::ReputationTooLow
    );

    // 3. Set unstake request timestamp to begin cooldown
    warden.unstake_requested_at = Some(clock.unix_timestamp);

    // 4. Emit event
    emit!(UnstakeRequested {
        authority: warden.authority,
        requested_at: clock.unix_timestamp,
    });

    Ok(())
}

/// Completes the unstaking process after the 7-day cooldown period
pub fn claim_unstake_handler(ctx: Context<ClaimUnstake>) -> Result<()> {
    let warden = &ctx.accounts.warden;
    let clock = Clock::get()?;

    // 1. Verify unstake was requested
    let unstake_requested_at = warden.unstake_requested_at
        .ok_or(ArkhamErrorCode::UnstakeNotRequested)?;

    // 2. Verify 7-day cooldown has elapsed (604800 seconds = 7 days)
    const COOLDOWN_PERIOD: i64 = 604_800;
    require!(
        clock.unix_timestamp >= unstake_requested_at + COOLDOWN_PERIOD,
        ArkhamErrorCode::CooldownNotComplete
    );

    // 3. Transfer staked tokens back to authority based on stake_token type
    let stake_amount = warden.stake_amount;
    match warden.stake_token {
        StakeToken::Sol => {
            // Transfer SOL from vault back to authority
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
            system_program::transfer(cpi_context, stake_amount)?;
        }
        StakeToken::Usdc => {
            // Transfer USDC from vault back to authority's token account
            let vault_seeds = &[b"sol_vault".as_ref(), &[ctx.bumps.sol_vault]];
            let signer_seeds = &[&vault_seeds[..]];

            let cpi_accounts = Transfer {
                from: ctx.accounts.usdc_vault.to_account_info(),
                to: ctx.accounts.stake_to_account.to_account_info(),
                authority: ctx.accounts.sol_vault.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_context = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer_seeds);
            token::transfer(cpi_context, stake_amount)?;
        }
        StakeToken::Usdt => {
            // Transfer USDT from vault back to authority's token account
            let vault_seeds = &[b"sol_vault".as_ref(), &[ctx.bumps.sol_vault]];
            let signer_seeds = &[&vault_seeds[..]];

            let cpi_accounts = Transfer {
                from: ctx.accounts.usdt_vault.to_account_info(),
                to: ctx.accounts.stake_to_account.to_account_info(),
                authority: ctx.accounts.sol_vault.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_context = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer_seeds);
            token::transfer(cpi_context, stake_amount)?;
        }
    }

    // 4. Emit event
    emit!(WardenUnstaked {
        authority: warden.authority,
        stake_amount: warden.stake_amount,
        stake_token: warden.stake_token.clone(),
    });

    // Note: Warden account will be closed automatically via the close constraint
    Ok(())
}

// Account Contexts

#[derive(Accounts)]
#[instruction(stake_token: StakeToken, stake_amount: u64, peer_id: String, region_code: u8, ip_hash: [u8; 32], price: u64, timestamp: i64, signature: [u8; 64])]
pub struct InitializeWarden<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + 512,
        seeds = [b"warden", authority.key().as_ref()],
        bump
    )]
    pub warden: Account<'info, Warden>,

    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(seeds = [b"protocol_config"], bump)]
    pub protocol_config: Account<'info, ProtocolConfig>,

    /// CHECK: Instructions sysvar for Ed25519 verification
    #[account(address = INSTRUCTIONS_SYSVAR_ID)]
    pub instructions_sysvar: AccountInfo<'info>,

    /// CHECK: User's source account for the stake
    #[account(mut)]
    pub stake_from_account: AccountInfo<'info>,

    #[account(mut, seeds = [b"sol_vault"], bump)]
    pub sol_vault: SystemAccount<'info>,

    #[account(
        init_if_needed,
        payer = authority,
        associated_token::mint = usdc_mint,
        associated_token::authority = sol_vault,
    )]
    pub usdc_vault: Account<'info, anchor_spl::token::TokenAccount>,

    #[account(
        init_if_needed,
        payer = authority,
        associated_token::mint = usdt_mint,
        associated_token::authority = sol_vault,
    )]
    pub usdt_vault: Account<'info, anchor_spl::token::TokenAccount>,

    pub usdc_mint: Account<'info, anchor_spl::token::Mint>,
    pub usdt_mint: Account<'info, anchor_spl::token::Mint>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, anchor_spl::token::Token>,
    pub associated_token_program: Program<'info, anchor_spl::associated_token::AssociatedToken>,
}

#[derive(Accounts)]
pub struct UnstakeWarden<'info> {
    #[account(
        mut,
        seeds = [b"warden", authority.key().as_ref()],
        bump,
        has_one = authority
    )]
    pub warden: Account<'info, Warden>,

    #[account(mut)]
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct ClaimUnstake<'info> {
    #[account(
        mut,
        seeds = [b"warden", authority.key().as_ref()],
        bump,
        has_one = authority,
        close = authority  // Automatically refund rent to authority when closing
    )]
    pub warden: Account<'info, Warden>,

    #[account(mut)]
    pub authority: Signer<'info>,

    /// The protocol's SOL vault (PDA)
    #[account(mut, seeds = [b"sol_vault"], bump)]
    pub sol_vault: SystemAccount<'info>,

    #[account(mut)]
    pub usdc_vault: Account<'info, TokenAccount>,

    #[account(mut)]
    pub usdt_vault: Account<'info, TokenAccount>,

    /// CHECK: Destination token account for USDC/USDT unstaking
    #[account(mut)]
    pub stake_to_account: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}

// Events

#[event]
pub struct WardenRegistered {
    pub authority: Pubkey,
    pub tier: Tier,
    pub stake_amount: u64,
    pub stake_token: StakeToken,
}

#[event]
pub struct UnstakeRequested {
    pub authority: Pubkey,
    pub requested_at: i64,
}

#[event]
pub struct WardenUnstaked {
    pub authority: Pubkey,
    pub stake_amount: u64,
    pub stake_token: StakeToken,
}

// Custom error codes specific to oracle verification
#[error_code]
pub enum OracleError {
    #[msg("Invalid Instructions sysvar account")]
    InvalidInstructionsSysvar,
    
    #[msg("Ed25519Program instruction not found at expected index")]
    Ed25519InstructionNotFound,
    
    #[msg("Instruction is not an Ed25519Program instruction")]
    InvalidEd25519Instruction,
    
    #[msg("Ed25519Program instruction data is invalid or too short")]
    InvalidEd25519Data,
    
    #[msg("Signature in Ed25519 instruction doesn't match expected signature")]
    SignatureMismatch,
    
    #[msg("Public key in Ed25519 instruction doesn't match oracle authority")]
    PublicKeyMismatch,
    
    #[msg("Message in Ed25519 instruction doesn't match expected message")]
    MessageMismatch,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_create_oracle_message() {
        let price = 150_000_000u64; // $150 in micro-units
        let timestamp = 1234567890i64;
        
        let message = create_oracle_message(price, timestamp);
        let message2 = create_oracle_message(price, timestamp);
        
        // Messages should be deterministic
        assert_eq!(message, message2);
        assert_eq!(message.len(), 32); // Keccak hash is 32 bytes
        
        // Different price should produce different message
        let message3 = create_oracle_message(price + 1, timestamp);
        assert_ne!(message, message3);
        
        // Different timestamp should produce different message
        let message4 = create_oracle_message(price, timestamp + 1);
        assert_ne!(message, message4);
    }
}
