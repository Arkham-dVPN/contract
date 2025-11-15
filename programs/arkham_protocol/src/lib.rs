use anchor_lang::prelude::*;

declare_id!("6Qj2WJcAmvQUh6tTYMT1yuDLL6eSpp8cFY9PzPLCeSgj");

pub mod state;
pub use state::*;
pub mod instructions;
pub use instructions::*;

#[program]
pub mod arkham_protocol {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        msg!("Greetings from: {:?}", ctx.program_id);
        Ok(())
    }

    pub fn initialize_warden(
        ctx: Context<InitializeWarden>,
        peer_id: String,
        region_code: u8,
        ip_hash: [u8; 32],
    ) -> Result<()> {
        initialize_warden_handler(ctx, peer_id, region_code, ip_hash)
    }
}

#[derive(Accounts)]
pub struct Initialize {}
