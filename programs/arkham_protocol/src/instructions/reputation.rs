use anchor_lang::prelude::*;
use crate::state::{Warden, ProtocolConfig};
use crate::ArkhamErrorCode;

/// Updates a Warden's reputation score based on performance metrics
/// This instruction should typically be called by an off-chain cron job
pub fn update_reputation_handler(
    ctx: Context<UpdateReputation>,
    connection_success: bool,
    uptime_report: u16, // Uptime as basis points (0-10000)
) -> Result<()> {
    let warden = &mut ctx.accounts.warden;
    let config = &ctx.accounts.protocol_config;
    let clock = Clock::get()?;

    // Verify the caller is the authorized updater
    require!(
        ctx.accounts.authority.key() == config.reputation_updater,
        ArkhamErrorCode::UnauthorizedReputationUpdate
    );

    // Update connection statistics
    if connection_success {
        warden.successful_connections = warden.successful_connections
            .checked_add(1)
            .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;
    } else {
        warden.failed_connections = warden.failed_connections
            .checked_add(1)
            .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;
    }

    // Update uptime percentage
    warden.uptime_percentage = uptime_report;

    // Calculate new reputation score using weighted formula
    let new_reputation = calculate_reputation_score(warden, clock.unix_timestamp)?;

    // Update the reputation score
    warden.reputation_score = new_reputation;

    // Update last active timestamp
    warden.last_active = clock.unix_timestamp;

    // Check if the warden qualifies for premium pool based on reputation
    // This will be updated by a separate ranking function called off-chain
    if new_reputation >= 8000 { // 80% threshold for premium eligibility
        // Premium pool ranking will be handled by a separate off-chain process
        // The actual ranking is computed off-chain and only the rank is stored
    }

    emit!(ReputationUpdated {
        warden: warden.authority,
        new_score: new_reputation,
        uptime_report,
        connection_success,
    });

    Ok(())
}

/// Calculates the reputation score using a weighted formula:
/// - Connection success rate: 40% weight
/// - Uptime percentage: 30% weight  
/// - Recent bandwidth contribution: 20% weight
/// - Time since last active: 10% weight (decays over time)
fn calculate_reputation_score(warden: &Warden, current_timestamp: i64) -> Result<u32> {
    // 1. Connection success rate (40% weight)
    let total_connections = warden.successful_connections
        .checked_add(warden.failed_connections)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;
    
    let success_rate = if total_connections > 0 {
        let success_bps = (warden.successful_connections as u128)
            .checked_mul(10000)
            .ok_or(ArkhamErrorCode::ArithmeticOverflow)?
            .checked_div(total_connections as u128)
            .ok_or(ArkhamErrorCode::ArithmeticOverflow)? as u32;
        success_bps.min(10000) // Cap at 100%
    } else {
        10000 // New wardens start with perfect score
    };
    
    let success_contribution = (success_rate as u128)
        .checked_mul(40)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?
        .checked_div(100)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)? as u32;

    // 2. Uptime percentage (30% weight)
    let uptime_contribution = (warden.uptime_percentage as u128)
        .checked_mul(30)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?
        .checked_div(100)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)? as u32;

    // 3. Recent bandwidth contribution (20% weight)
    // Calculate bandwidth served in the last 7 days
    // For simplicity, we'll use a decay function based on last_active timestamp
    // In a full implementation, we'd track bandwidth per time period
    let days_since_active = (current_timestamp - warden.last_active)
        .checked_div(24 * 3600) // seconds in a day
        .unwrap_or(0);
    
    let max_days = 7i64; // Consider activity in last 7 days
    let activity_score = if days_since_active <= max_days {
        10000u32.saturating_sub(
            (days_since_active as u32)
                .checked_mul(10000)
                .ok_or(ArkhamErrorCode::ArithmeticOverflow)?
                .checked_div(max_days as u32)
                .unwrap_or(10000)
        )
    } else {
        0 // No contribution if inactive for more than a week
    };
    
    let bandwidth_contribution = (activity_score as u128)
        .checked_mul(20)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?
        .checked_div(100)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)? as u32;

    // 4. Recency bonus/penalty (10% weight)
    // Decay reputation for inactivity
    let recency_penalty = if days_since_active <= max_days {
        // No penalty if active recently
        0
    } else {
        // Apply penalty for each day beyond the max active period
        let days_beyond = days_since_active.saturating_sub(max_days);
        let penalty = (days_beyond as u32)
            .checked_mul(100) // 100 points per day (adjust as needed)
            .unwrap_or(10000)
            .min(5000); // Cap penalty at 50% to prevent zeroing reputation instantly
        
        penalty
    };
    
    let recency_contribution = 10000u32.saturating_sub(recency_penalty);
    let recency_contribution = (recency_contribution as u128)
        .checked_mul(10)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?
        .checked_div(100)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)? as u32;

    // Sum all contributions
    let total_contribution = success_contribution
        .checked_add(uptime_contribution)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?
        .checked_add(bandwidth_contribution)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?
        .checked_add(recency_contribution)
        .ok_or(ArkhamErrorCode::ArithmeticOverflow)?;

    // Cap at 10000 (100%)
    Ok(total_contribution.min(10000))
}

/// Updates premium pool rankings by calculating all wardens' reputation scores
/// This is typically called off-chain as a batch operation since it requires scanning all accounts
pub fn update_premium_pool_rankings_handler(
    ctx: Context<UpdatePremiumPoolRankings>,
    top_wardens: Vec<Pubkey>, // Top 100 warden pubkeys in reputation order
) -> Result<()> {
    // Verify the caller is authorized to update rankings
    let config = &ctx.accounts.protocol_config;
    require!(
        ctx.accounts.authority.key() == config.reputation_updater,
        ArkhamErrorCode::UnauthorizedReputationUpdate
    );

    // This would typically iterate through a list of wardens and assign ranks
    // In practice, this might be computed off-chain and only the rankings stored
    // For now, we'll emit an event to signal that rankings have been updated
    emit!(PremiumPoolRankingsUpdated {
        updater: ctx.accounts.authority.key(),
        top_wardens_count: top_wardens.len() as u32,
    });

    Ok(())
}

// Account contexts:

#[derive(Accounts)]
pub struct UpdateReputation<'info> {
    #[account(
        mut,
        seeds = [b"warden", warden_authority.key().as_ref()],
        bump,
    )]
    pub warden: Account<'info, Warden>,

    #[account(
        seeds = [b"protocol", b"config"],
        bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(mut)]
    pub warden_authority: SystemAccount<'info>, // The warden's authority (for PDA derivation)

    #[account(mut)]
    pub authority: Signer<'info>, // The authorized reputation updater
}

#[derive(Accounts)]
pub struct UpdatePremiumPoolRankings<'info> {
    #[account(
        seeds = [b"protocol", b"config"],
        bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(mut)]
    pub authority: Signer<'info>, // The authorized reputation updater
}

// Events:

#[event]
pub struct ReputationUpdated {
    pub warden: Pubkey,
    pub new_score: u32,
    pub uptime_report: u16,
    pub connection_success: bool,
}

#[event]
pub struct PremiumPoolRankingsUpdated {
    pub updater: Pubkey,
    pub top_wardens_count: u32,
}