use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount};

declare_id!("GhTuoRpAxo9YBUDUt7c4nPSnGuPtg1kRASaA3sshveoD");

#[program]
pub mod token_sale_project {
    use super::*;

    pub fn initialize(
        ctx: Context<Initialize>,
        usdc_mint: Pubkey,
        token_mint: Pubkey,
        token_price_cents: u64,
        token_decimals: u8,
        usdc_decimals: u8,
        start_ts: i64,
        end_ts: i64,
    ) -> Result<()> {
        require!(end_ts > start_ts, ErrorCode::InvalidSaleWindow);

        let sale = &mut ctx.accounts.sale_config;
        sale.admin = ctx.accounts.admin.key();
        sale.usdc_mint = usdc_mint;
        sale.token_mint = token_mint;
        sale.token_price_cents = token_price_cents;
        sale.token_decimals = token_decimals;
        sale.usdc_decimals = usdc_decimals;
        sale.start_ts = start_ts;
        sale.end_ts = end_ts;
        sale.bump = ctx.bumps.sale_config;

        emit!(SaleInitialized {
            admin: sale.admin,
            token_price_cents,
            start_ts,
            end_ts,
        });
        Ok(())
    }

    pub fn purchase(ctx: Context<Purchase>, amount: u64) -> Result<()> {
        let sale = &ctx.accounts.sale_config;

        // The sale window gates buying only; withdraw/close stay open so unsold
        // inventory is always reclaimable by the admin.
        let now = Clock::get()?.unix_timestamp;
        require!(now >= sale.start_ts, ErrorCode::SaleNotStarted);
        require!(now < sale.end_ts, ErrorCode::SaleEnded);

        // Both vaults must still be owned by the sale PDA.
        require!(ctx.accounts.vault_usdc.owner == sale.key(), ErrorCode::InvalidVaultOwner);
        require!(ctx.accounts.vault.owner == sale.key(), ErrorCode::InvalidVaultOwner);

        // Cost = amount * price_cents * 10^usdc_dec / (10^token_dec * 100), in u128.
        let amount_u128 = amount as u128;
        let token_price_cents_u128 = sale.token_price_cents as u128;
        let usdc_decimals_u32 = sale.usdc_decimals as u32;
        let token_decimals_u32 = sale.token_decimals as u32;

        let numerator = amount_u128
            .checked_mul(token_price_cents_u128)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_mul(10u128.pow(usdc_decimals_u32))
            .ok_or(ErrorCode::MathOverflow)?;

        let denominator = 10u128
            .checked_pow(token_decimals_u32)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_mul(100)
            .ok_or(ErrorCode::MathOverflow)?;

        let usdc_amount_u128 = numerator.checked_div(denominator).ok_or(ErrorCode::MathOverflow)?;

        let usdc_amount: u64 = usdc_amount_u128.try_into().map_err(|_| ErrorCode::MathOverflow)?;
        require!(usdc_amount > 0, ErrorCode::BelowMinimumPurchase);

        // Buyer pays USDC into the vault.
        let cpi_accounts_usdc = token::Transfer {
            from: ctx.accounts.buyer_usdc.to_account_info(),
            to: ctx.accounts.vault_usdc.to_account_info(),
            authority: ctx.accounts.buyer.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_context = CpiContext::new(cpi_program, cpi_accounts_usdc);
        token::transfer(cpi_context, usdc_amount)?;

        // Program-signed transfer of tokens out of the vault to the buyer.
        let cpi_accounts_token = token::Transfer {
            from: ctx.accounts.vault.to_account_info(),
            to: ctx.accounts.buyer_token.to_account_info(),
            authority: ctx.accounts.sale_config.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let seeds = &[b"sale_config" as &[u8], &[ctx.accounts.sale_config.bump]];
        let signer = &[&seeds[..]];
        let cpi_context = CpiContext::new_with_signer(cpi_program, cpi_accounts_token, signer);
        token::transfer(cpi_context, amount)?;

        emit!(SalePurchase {
            buyer: ctx.accounts.buyer.key(),
            token_amount: amount,
            usdc_amount,
        });
        Ok(())
    }

    pub fn withdraw(ctx: Context<Withdraw>, usdc_amount: u64, token_amount: u64) -> Result<()> {
        let sale = &ctx.accounts.sale_config;

        require!(ctx.accounts.admin.key() == sale.admin, ErrorCode::Unauthorized);

        let seeds = &[b"sale_config" as &[u8], &[ctx.accounts.sale_config.bump]];
        let signer = &[&seeds[..]];

        if usdc_amount > 0 {
            let cpi_accounts_usdc = token::Transfer {
                from: ctx.accounts.vault_usdc.to_account_info(),
                to: ctx.accounts.admin_usdc.to_account_info(),
                authority: ctx.accounts.sale_config.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_context = CpiContext::new_with_signer(cpi_program, cpi_accounts_usdc, signer);
            token::transfer(cpi_context, usdc_amount)?;
        }

        if token_amount > 0 {
            let cpi_accounts_token = token::Transfer {
                from: ctx.accounts.vault.to_account_info(),
                to: ctx.accounts.admin_token.to_account_info(),
                authority: ctx.accounts.sale_config.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_context = CpiContext::new_with_signer(cpi_program, cpi_accounts_token, signer);
            token::transfer(cpi_context, token_amount)?;
        }

        Ok(())
    }

    pub fn update_price(ctx: Context<UpdatePrice>, new_price_cents: u64) -> Result<()> {
        require!(ctx.accounts.admin.key() == ctx.accounts.sale_config.admin, ErrorCode::Unauthorized);
        require!(new_price_cents > 0, ErrorCode::InvalidPrice);
        ctx.accounts.sale_config.token_price_cents = new_price_cents;

        emit!(PriceUpdated { new_price_cents });
        Ok(())
    }

    pub fn close(ctx: Context<Close>) -> Result<()> {
        require!(ctx.accounts.admin.key() == ctx.accounts.sale_config.admin, ErrorCode::Unauthorized);

        let seeds = &[b"sale_config" as &[u8], &[ctx.accounts.sale_config.bump]];
        let signer = &[&seeds[..]];

        token::close_account(CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            token::CloseAccount {
                account: ctx.accounts.vault_usdc.to_account_info(),
                destination: ctx.accounts.admin.to_account_info(),
                authority: ctx.accounts.sale_config.to_account_info(),
            },
            signer,
        ))?;

        token::close_account(CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            token::CloseAccount {
                account: ctx.accounts.vault.to_account_info(),
                destination: ctx.accounts.admin.to_account_info(),
                authority: ctx.accounts.sale_config.to_account_info(),
            },
            signer,
        ))?;

        emit!(SaleClosed { admin: ctx.accounts.admin.key() });
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = admin,
        // disc(8) + admin(32) + usdc_mint(32) + token_mint(32) + price(8)
        //   + token_decimals(1) + usdc_decimals(1) + bump(1) + start_ts(8) + end_ts(8)
        space = 8 + 32 + 32 + 32 + 8 + 1 + 1 + 1 + 8 + 8,
        seeds = [b"sale_config"],
        bump
    )]
    pub sale_config: Account<'info, IcoState>,
    #[account(mut)]
    pub admin: Signer<'info>,

    // Gate initialization to the program's upgrade authority. Without this,
    // anyone could front-run `initialize` to claim the singleton sale_config
    // PDA and become the permanent admin (there is no admin-transfer path).
    #[account(constraint = program.programdata_address()? == Some(program_data.key()) @ ErrorCode::Unauthorized)]
    pub program: Program<'info, program::TokenSaleProject>,
    #[account(constraint = program_data.upgrade_authority_address == Some(admin.key()) @ ErrorCode::Unauthorized)]
    pub program_data: Account<'info, ProgramData>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Purchase<'info> {
    #[account(seeds = [b"sale_config"], bump)]
    pub sale_config: Account<'info, IcoState>,
    #[account(mut)]
    pub buyer: Signer<'info>,
    #[account(mut, constraint = buyer_usdc.mint == sale_config.usdc_mint @ ErrorCode::InvalidUsdcMint)]
    pub buyer_usdc: Account<'info, TokenAccount>,
    #[account(mut, constraint = vault_usdc.mint == sale_config.usdc_mint @ ErrorCode::InvalidUsdcMint)]
    pub vault_usdc: Account<'info, TokenAccount>,
    #[account(mut, constraint = buyer_token.mint == sale_config.token_mint @ ErrorCode::InvalidTokenMint)]
    pub buyer_token: Account<'info, TokenAccount>,
    #[account(mut, constraint = vault.mint == sale_config.token_mint @ ErrorCode::InvalidTokenMint)]
    pub vault: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(seeds = [b"sale_config"], bump)]
    pub sale_config: Account<'info, IcoState>,
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(mut)]
    pub admin_usdc: Account<'info, TokenAccount>,
    #[account(mut)]
    pub vault_usdc: Account<'info, TokenAccount>,
    #[account(mut)]
    pub admin_token: Account<'info, TokenAccount>,
    #[account(mut)]
    pub vault: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct UpdatePrice<'info> {
    #[account(mut, seeds = [b"sale_config"], bump)]
    pub sale_config: Account<'info, IcoState>,
    pub admin: Signer<'info>,
}

#[derive(Accounts)]
pub struct Close<'info> {
    #[account(mut, seeds = [b"sale_config"], bump, close = admin)]
    pub sale_config: Account<'info, IcoState>,
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(mut, constraint = vault_usdc.owner == sale_config.key() @ ErrorCode::InvalidVaultOwner)]
    pub vault_usdc: Account<'info, TokenAccount>,
    #[account(mut, constraint = vault.owner == sale_config.key() @ ErrorCode::InvalidVaultOwner)]
    pub vault: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

#[account]
#[derive(Default)]
pub struct IcoState {
    pub admin: Pubkey,
    pub usdc_mint: Pubkey,
    pub token_mint: Pubkey,
    pub token_price_cents: u64,
    pub token_decimals: u8,
    pub usdc_decimals: u8,
    pub start_ts: i64,
    pub end_ts: i64,
    pub bump: u8,
}

#[event]
pub struct SaleInitialized {
    pub admin: Pubkey,
    pub token_price_cents: u64,
    pub start_ts: i64,
    pub end_ts: i64,
}

#[event]
pub struct SalePurchase {
    pub buyer: Pubkey,
    pub token_amount: u64,
    pub usdc_amount: u64,
}

#[event]
pub struct PriceUpdated {
    pub new_price_cents: u64,
}

#[event]
pub struct SaleClosed {
    pub admin: Pubkey,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Caller is not the configured admin")]
    Unauthorized,
    #[msg("USDC mint does not match the sale configuration")]
    InvalidUsdcMint,
    #[msg("Token mint does not match the sale configuration")]
    InvalidTokenMint,
    #[msg("Vault is not owned by the sale authority")]
    InvalidVaultOwner,
    #[msg("Arithmetic overflow")]
    MathOverflow,
    #[msg("Computed cost rounds to zero; increase the amount")]
    BelowMinimumPurchase,
    #[msg("Price must be greater than zero")]
    InvalidPrice,
    #[msg("Sale has not started yet")]
    SaleNotStarted,
    #[msg("Sale has ended")]
    SaleEnded,
    #[msg("End time must be after start time")]
    InvalidSaleWindow,
}
