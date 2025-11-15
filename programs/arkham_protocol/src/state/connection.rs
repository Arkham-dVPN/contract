use anchor_lang::prelude::*;

#[account]
pub struct Connection {
    pub seeker: Pubkey,
    pub warden: Pubkey,
    pub started_at: i64,
    pub last_proof_at: i64,
    pub bandwidth_consumed: u64, // in megabytes
    pub bandwidth_proofs: Vec<BandwidthProof>,
    pub amount_escrowed: u64, // in lamports
    pub amount_paid: u64, // in lamports
    pub rate_per_mb: u64, // in lamports
    pub warden_multiplier: u16, // basis points
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct BandwidthProof {
    pub timestamp: i64,
    pub mb_consumed: u64,
    pub seeker_signature: [u8; 64],
    pub warden_signature: [u8; 64],
}
