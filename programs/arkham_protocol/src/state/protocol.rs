use anchor_lang::prelude::*;

#[account]
pub struct ProtocolConfig {
    pub authority: Pubkey,
    pub treasury: Pubkey,
    pub arkham_token_mint: Pubkey,
    pub base_rate_per_mb: u64, // in lamports
    pub protocol_fee_bps: u16,
    pub tier_thresholds: [u64; 3], // USD value
    pub tier_multipliers: [u16; 3], // basis points
    pub tokens_per_5gb: u64,
    pub geo_premiums: Vec<GeoPremium>,
    pub reputation_updater: Pubkey, // Authority allowed to update reputations
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct GeoPremium {
    pub region_code: u8,
    pub premium_bps: u16,
}
