use anchor_lang::prelude::*;

#[account]
pub struct Seeker {
    pub authority: Pubkey,
    pub escrow_balance: u64, // in lamports
    pub private_escrow: Option<Pubkey>,
    pub total_bandwidth_consumed: u64, // in megabytes
    pub total_spent: u64, // in lamports
    pub active_connections: u8,
    pub premium_expires_at: Option<i64>,
}
