use anchor_lang::{prelude::*, system_program};
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use pyth_sdk_solana::state::PriceAccount as SolanaPriceAccount;
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
) -> Result<()> {
    let config = &ctx.accounts.protocol_config;
    let clock = Clock::get()?;
    let current_timestamp = clock.unix_timestamp;

    // 1. Calculate USD value of the stake
    let stake_value_usd = get_stake_usd_value(
        &stake_token,
        stake_amount,
        &ctx.accounts.sol_usd_price_feed,
        &ctx.accounts.usdt_usd_price_feed,
        current_timestamp,
    )?;

    // 2. Determine the tier
    let tier = if stake_value_usd >= config.tier_thresholds[2] {
        Tier::Gold
    } else if stake_value_usd >= config.tier_thresholds[1] {
        Tier::Silver
    } else if stake_value_usd >= config.tier_thresholds[0] {
        Tier::Bronze
    } else {
        return err!(ArkhamErrorCode::InsufficientStake);
    };

    // 3. Transfer stake tokens to the appropriate vault
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

    // 4. Initialize the Warden account
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

    // 5. Emit a registration event
    emit!(WardenRegistered {
        authority: warden.authority,
        tier: warden.tier.clone(),
        stake_amount: warden.stake_amount,
        stake_token: warden.stake_token.clone(),
    });

    Ok(())
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



/// Calculates the USD value of a given stake amount, normalized to 6 decimal places.
fn get_stake_usd_value<'info>(
    stake_token: &StakeToken,
    stake_amount: u64,
    sol_price_feed: &AccountInfo<'info>,
    usdt_price_feed: &AccountInfo<'info>,
    current_timestamp: i64,
) -> Result<u64> {
    match stake_token {
        StakeToken::Sol => {
            let price_feed = SolanaPriceAccount::account_info_to_feed(sol_price_feed)
                .map_err(|_| ArkhamErrorCode::InvalidPriceAccount)?;
            let price = price_feed.get_price_no_older_than(current_timestamp, 60)
                .ok_or(ArkhamErrorCode::StalePrice)?;
            
            // price.expo is negative, e.g., -8 for SOL/USD
            let exponent = (SOL_DECIMALS as i32 + price.expo) - USD_DECIMALS as i32;
            if exponent < 0 {
                // This should not happen with standard tokens but as a safeguard
                return err!(ArkhamErrorCode::InvalidPriceAccount);
            }
            let usd_value = (stake_amount as u128)
                .checked_mul(price.price as u128)
                .unwrap()
                .checked_div(10u128.pow(exponent as u32))
                .unwrap_or(0);
            Ok(usd_value as u64)
        }
        StakeToken::Usdc => {
            // Assume 1:1 peg with USD. USDC has 6 decimals, which matches our target USD decimals.
            Ok(stake_amount)
        }
        StakeToken::Usdt => {
            let price_feed = SolanaPriceAccount::account_info_to_feed(usdt_price_feed)
                .map_err(|_| ArkhamErrorCode::InvalidPriceAccount)?;
            let price = price_feed.get_price_no_older_than(current_timestamp, 60)
                .ok_or(ArkhamErrorCode::StalePrice)?;

            let exponent = (USDT_DECIMALS as i32 + price.expo) - USD_DECIMALS as i32;
            if exponent < 0 {
                return err!(ArkhamErrorCode::InvalidPriceAccount);
            }
            let usd_value = (stake_amount as u128)
                .checked_mul(price.price as u128)
                .unwrap()
                .checked_div(10u128.pow(exponent as u32))
                .unwrap_or(0);
            Ok(usd_value as u64)
        }
    }
}

#[derive(Accounts)]
#[instruction(stake_token: StakeToken, stake_amount: u64, peer_id: String, region_code: u8, ip_hash: [u8; 32])]
pub struct InitializeWarden<'info> {
    #[account(
        init,
        payer = authority,
        // Space calculation needs to be precise.
        // Using a generous 512 bytes for now to accommodate string and option types.
        space = 8 + 512,
        seeds = [b"warden", authority.key().as_ref()],
        bump
    )]
    pub warden: Account<'info, Warden>,

    #[account(mut)]
    pub authority: Signer<'info>,

    // Ensure this is the correct protocol config account
    #[account(seeds = [b"protocol", b"config"], bump)]
    pub protocol_config: Account<'info, ProtocolConfig>,

    /// CHECK: User's source account for the stake. For SOL, this is the signer. For SPL, it's a token account.
    #[account(mut)]
    pub stake_from_account: AccountInfo<'info>,

    /// The protocol's SOL vault (PDA).
    #[account(mut, seeds = [b"sol_vault"], bump)]
    pub sol_vault: SystemAccount<'info>,

    #[account(mut)]
    pub usdc_vault: Account<'info, TokenAccount>,

    #[account(mut)]
    pub usdt_vault: Account<'info, TokenAccount>,

    /// CHECK: Pyth SOL/USD price feed. Address constraint will be added in production.
    pub sol_usd_price_feed: AccountInfo<'info>,
    /// CHECK: Pyth USDT/USD price feed. Address constraint will be added in production.
    pub usdt_usd_price_feed: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}

// Add account contexts at the end of staking.rs:

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