use anchor_lang::prelude::*;

// Program id of the channel being built, from Solex.yaml.
solex::declare_id!();

#[solex::program]
pub mod counter {
    use super::*;

    /// Ships on `stable` and every channel that includes it.
    #[channel(stable)]
    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        ctx.accounts.counter.count = 0;
        msg!("counter ({}) initialized", solex::active_channel!());
        Ok(())
    }

    #[channel(stable)]
    pub fn increment(ctx: Context<Update>) -> Result<()> {
        ctx.accounts.counter.count += 1;
        Ok(())
    }

    /// Only compiled into the `nightly` channel (and channels including it), and
    /// refuses to run until epoch 1000 even there.
    #[channel([nightly, 1000])]
    pub fn reset(ctx: Context<Update>) -> Result<()> {
        ctx.accounts.counter.count = 0;
        Ok(())
    }
}

#[account]
#[derive(InitSpace)]
pub struct Counter {
    pub count: u64,
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(init, payer = payer, space = 8 + Counter::INIT_SPACE)]
    pub counter: Account<'info, Counter>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Update<'info> {
    #[account(mut)]
    pub counter: Account<'info, Counter>,
}
