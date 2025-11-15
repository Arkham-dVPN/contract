use anchor_lang::prelude::*;

#[account]
pub struct Warden {
    pub authority: Pubkey,
    pub peer_id: String,
    pub stake_token: StakeToken,
    pub stake_amount: u64,
    pub stake_value_usd: u64,
    pub tier: Tier,
    pub staked_at: i64,
    pub unstake_requested_at: Option<i64>,
    pub total_bandwidth_served: u64, // in megabytes
    pub total_earnings: u64, // in lamports
    pub pending_claims: u64, // in lamports
    pub arkham_tokens_earned: u64,
    pub reputation_score: u32, // 0-10000
    pub successful_connections: u64,
    pub failed_connections: u64,
    pub uptime_percentage: u16, // basis points
    pub last_active: i64,
    pub region_code: u8,
    pub ip_hash: [u8; 32],
    pub premium_pool_rank: Option<u16>,
    pub active_connections: u8,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub enum StakeToken {
    Sol,
    Usdc,
    Usdt,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub enum Tier {
    Bronze,
    Silver,
    Gold,
}
