use anchor_lang::prelude::*;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct ReputationMetrics {
    pub connection_success_weight: u16, // basis points
    pub uptime_weight: u16, // basis points
    pub bandwidth_contribution_weight: u16, // basis points
    pub recency_weight: u16, // basis points
}
