use anchor_lang::prelude::*;

declare_id!("6Qj2WJcAmvQUh6tTYMT1yuDLL6eSpp8cFY9PzPLCeSgj");

pub mod state;
pub use state::*;

#[program]
pub mod arkham_protocol {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        msg!("Greetings from: {:?}", ctx.program_id);
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize {}
