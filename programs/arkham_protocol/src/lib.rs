use anchor_lang::prelude::*;

declare_id!("6Qj2WJcAmvQUh6tTYMT1yuDLL6eSpp8cFY9PzPLCeSgj");

pub mod state;
pub mod instructions;

pub use instructions::*;
pub use state::*;

#[program]
pub mod arkham_protocol {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        msg!("Greetings from: {:?}", ctx.program_id);
        Ok(())
    }

    // ============================================
    // Staking Instructions
    // ============================================
    
    pub fn initialize_warden(
        ctx: Context<InitializeWarden>,
        stake_token: StakeToken,
        stake_amount: u64,
        peer_id: String,
        region_code: u8,
        ip_hash: [u8; 32],
    ) -> Result<()> {
        instructions::staking::initialize_warden_handler(
            ctx,
            stake_token,
            stake_amount,
            peer_id,
            region_code,
            ip_hash,
        )
    }

    pub fn unstake_warden(ctx: Context<UnstakeWarden>) -> Result<()> {
        instructions::staking::unstake_warden_handler(ctx)
    }

    pub fn claim_unstake(ctx: Context<ClaimUnstake>) -> Result<()> {
        instructions::staking::claim_unstake_handler(ctx)
    }

    // ============================================
    // Payment Instructions
    // ============================================

    pub fn deposit_escrow(
        ctx: Context<DepositEscrow>,
        amount: u64,
        use_private: bool,
    ) -> Result<()> {
        instructions::payments::deposit_escrow_handler(ctx, amount, use_private)
    }

    pub fn start_connection(
        ctx: Context<StartConnection>,
        estimated_mb: u64,
    ) -> Result<()> {
        instructions::payments::start_connection_handler(ctx, estimated_mb)
    }

    pub fn submit_bandwidth_proof(
        ctx: Context<SubmitBandwidthProof>,
        mb_consumed: u64,
        seeker_signature: [u8; 64],
        warden_signature: [u8; 64],
    ) -> Result<()> {
        instructions::payments::submit_bandwidth_proof_handler(
            ctx,
            mb_consumed,
            seeker_signature,
            warden_signature,
        )
    }

    pub fn end_connection(ctx: Context<EndConnection>) -> Result<()> {
        instructions::payments::end_connection_handler(ctx)
    }

    pub fn claim_earnings(
        ctx: Context<ClaimEarnings>,
        use_private: bool,
    ) -> Result<()> {
        instructions::payments::claim_earnings_handler(ctx, use_private)
    }

    pub fn claim_arkham_tokens(ctx: Context<ClaimArkhamTokens>) -> Result<()> {
        instructions::payments::claim_arkham_tokens_handler(ctx)
    }

    // ============================================
    // Reputation Instructions
    // ============================================

    pub fn update_reputation(
        ctx: Context<UpdateReputation>,
        connection_success: bool,
        uptime_report: u16,
    ) -> Result<()> {
        instructions::reputation::update_reputation_handler(
            ctx,
            connection_success,
            uptime_report,
        )
    }

    pub fn update_premium_pool_rankings(
        ctx: Context<UpdatePremiumPoolRankings>,
        top_wardens: Vec<Pubkey>,
    ) -> Result<()> {
        instructions::reputation::update_premium_pool_rankings_handler(
            ctx,
            top_wardens,
        )
    }

    // ============================================
    // Admin Instructions
    // ============================================

    pub fn update_protocol_config(
        ctx: Context<UpdateProtocolConfig>,
        new_base_rate_per_mb: Option<u64>,
        new_protocol_fee_bps: Option<u16>,
        new_tier_thresholds: Option<[u64; 3]>,
        new_tier_multipliers: Option<[u16; 3]>,
        new_tokens_per_5gb: Option<u64>,
        new_geo_premiums: Option<Vec<GeoPremium>>,
        new_reputation_updater: Option<Pubkey>,
    ) -> Result<()> {
        instructions::admin::update_protocol_config_handler(
            ctx,
            new_base_rate_per_mb,
            new_protocol_fee_bps,
            new_tier_thresholds,
            new_tier_multipliers,
            new_tokens_per_5gb,
            new_geo_premiums,
            new_reputation_updater,
        )
    }

    pub fn initialize_arkham_mint(ctx: Context<InitializeArkhamMint>) -> Result<()> {
        instructions::admin::initialize_arkham_mint_handler(ctx)
    }

    pub fn distribute_subsidies(
        ctx: Context<DistributeSubsidies>,
        warden_keys: Vec<Pubkey>,
        subsidy_amounts: Vec<u64>,
    ) -> Result<()> {
        instructions::admin::distribute_subsidies_handler(
            ctx,
            warden_keys,
            subsidy_amounts,
        )
    }
}

#[derive(Accounts)]
pub struct Initialize {}

#[error_code]
pub enum ArkhamErrorCode {
    // Staking errors
    #[msg("Stake amount is insufficient to qualify for the lowest tier.")]
    InsufficientStake,
    #[msg("Warden has active connections and cannot unstake.")]
    HasActiveConnections,
    #[msg("Reputation score too low to unstake (must be at least 80%).")]
    ReputationTooLow,
    #[msg("Unstake not requested - must call unstake_warden first.")]
    UnstakeNotRequested,
    #[msg("Cooldown period not complete - must wait 7 days.")]
    CooldownNotComplete,

    // Oracle errors
    #[msg("The provided oracle price feed is invalid.")]
    InvalidPriceAccount,
    #[msg("The oracle price is too old.")]
    StalePrice,
    #[msg("The oracle price has too wide of a confidence interval.")]
    InvalidPriceConfidence,

    // Payment errors
    #[msg("Insufficient escrow balance.")]
    InsufficientEscrow,
    #[msg("Insufficient connection escrow for payment.")]
    InsufficientConnectionEscrow,
    #[msg("Nothing to claim.")]
    NothingToClaim,

    // Token errors
    #[msg("Invalid stake token type provided.")]
    InvalidStakeToken,
    #[msg("Token mint not initialized.")]
    TokenMintNotInitialized,
    #[msg("Token minting not yet implemented.")]
    TokenMintingNotImplemented,

    // Privacy errors
    #[msg("Private payments not yet implemented.")]
    PrivatePaymentsNotImplemented,

    // Reputation errors
    #[msg("Unauthorized reputation update attempt.")]
    UnauthorizedReputationUpdate,

    // Admin errors
    #[msg("Unauthorized admin action - caller is not the protocol authority.")]
    UnauthorizedAdminAction,
    #[msg("Invalid fee basis points - must be <= 10000 (100%).")]
    InvalidFeeBps,
    #[msg("Invalid tier thresholds - must be in ascending order.")]
    InvalidTierThresholds,
    #[msg("Invalid tier multiplier - must be <= 50000 (5x).")]
    InvalidTierMultiplier,
    #[msg("Invalid geographic premium - must be <= 50000 (500%).")]
    InvalidGeoPremium,
    #[msg("Duplicate region code found in geographic premiums.")]
    DuplicateRegionCode,
    #[msg("ARKHAM token mint is already initialized.")]
    TokenMintAlreadyInitialized,
    #[msg("Invalid subsidy distribution - vectors must have the same length.")]
    InvalidSubsidyDistribution,
    #[msg("Insufficient treasury balance for subsidy distribution.")]
    InsufficientTreasuryBalance,

    // General errors
    #[msg("Arithmetic operation resulted in overflow.")]
    ArithmeticOverflow,
}