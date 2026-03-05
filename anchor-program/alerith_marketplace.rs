use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Burn, Mint, MintTo, Token, TokenAccount, Transfer as TokenTransfer};
use mpl_core::instructions::TransferV1CpiBuilder;

declare_id!("Beva7XHsfKZM7zTZUz4dgXqCxfDM3Xc4wVSx9swYWf3F");

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MARKET_CONFIG_SEED: &[u8] = b"market_config";
const REGISTERED_ITEM_SEED: &[u8] = b"registered_item";
const ITEM_MINT_SEED: &[u8] = b"item_mint";
const LISTING_SEED: &[u8] = b"listing";
const NFT_CONFIG_SEED: &[u8] = b"nft_config";
const NFT_LISTING_SEED: &[u8] = b"nft_listing";
const PLAYER_ESCROW_SEED: &[u8] = b"player_escrow";

const MIN_DURATION_HOURS: u16 = 1;
const MAX_DURATION_HOURS: u16 = 48;
const SECONDS_PER_HOUR: i64 = 3600;

// ---------------------------------------------------------------------------
// Program
// ---------------------------------------------------------------------------

#[program]
pub mod alerith_marketplace {
    use super::*;

    /// Creates the singleton MarketConfig PDA that governs the marketplace.
    pub fn initialize(
        ctx: Context<Initialize>,
        treasury: Pubkey,
        listing_fee_bps: u16,
        sale_fee_bps: u16,
    ) -> Result<()> {
        let config = &mut ctx.accounts.market_config;
        config.authority = ctx.accounts.authority.key();
        config.treasury = treasury;
        config.listing_fee_bps = listing_fee_bps;
        config.sale_fee_bps = sale_fee_bps;
        config.listing_count = 0;
        config.paused = false;
        config.bump = ctx.bumps.market_config;
        Ok(())
    }

    /// Registers a game item type for on-chain trading by creating a
    /// RegisteredItem PDA and its corresponding SPL Token Mint PDA.
    pub fn register_item(
        ctx: Context<RegisterItem>,
        item_id: u32,
        name_hash: [u8; 32],
        is_tradeable: bool,
    ) -> Result<()> {
        let config = &ctx.accounts.market_config;
        require!(
            ctx.accounts.authority.key() == config.authority,
            MarketError::NotAuthority
        );

        let registered = &mut ctx.accounts.registered_item;
        registered.item_id = item_id;
        registered.mint = ctx.accounts.item_mint.key();
        registered.is_tradeable = is_tradeable;
        registered.name_hash = name_hash;
        registered.bump = ctx.bumps.registered_item;

        emit!(ItemRegistered {
            item_id,
            mint: ctx.accounts.item_mint.key(),
        });

        Ok(())
    }

    /// Server mints item tokens to a player's ATA.
    pub fn mint_item(ctx: Context<MintItem>, item_id: u32, amount: u64) -> Result<()> {
        let config = &ctx.accounts.market_config;
        require!(
            ctx.accounts.authority.key() == config.authority,
            MarketError::NotAuthority
        );
        require!(!config.paused, MarketError::MarketPaused);

        let bump = config.bump;
        let signer_seeds: &[&[&[u8]]] = &[&[MARKET_CONFIG_SEED, &[bump]]];

        token::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                MintTo {
                    mint: ctx.accounts.item_mint.to_account_info(),
                    to: ctx.accounts.recipient_ata.to_account_info(),
                    authority: ctx.accounts.market_config.to_account_info(),
                },
                signer_seeds,
            ),
            amount,
        )?;

        emit!(ItemMinted {
            item_id,
            recipient: ctx.accounts.recipient.key(),
            amount,
        });

        Ok(())
    }

    /// Player burns their item tokens (consumed in-game).
    pub fn burn_item(ctx: Context<BurnItem>, item_id: u32, amount: u64) -> Result<()> {
        require!(
            ctx.accounts.owner_ata.amount >= amount,
            MarketError::InsufficientTokens
        );

        token::burn(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Burn {
                    mint: ctx.accounts.item_mint.to_account_info(),
                    from: ctx.accounts.owner_ata.to_account_info(),
                    authority: ctx.accounts.owner.to_account_info(),
                },
            ),
            amount,
        )?;

        emit!(ItemBurned {
            item_id,
            owner: ctx.accounts.owner.key(),
            amount,
        });

        Ok(())
    }

    /// Lists items for sale on the marketplace.
    pub fn create_listing(
        ctx: Context<CreateListing>,
        buyout_price: u64,
        duration_hours: u16,
    ) -> Result<()> {
        let config = &ctx.accounts.market_config;
        require!(!config.paused, MarketError::MarketPaused);
        require!(buyout_price > 0, MarketError::InvalidPrice);
        require!(
            duration_hours >= MIN_DURATION_HOURS && duration_hours <= MAX_DURATION_HOURS,
            MarketError::InvalidDuration
        );

        let registered = &ctx.accounts.registered_item;
        require!(registered.is_tradeable, MarketError::ItemNotTradeable);

        let item_amount = ctx.accounts.seller_ata.amount;
        require!(item_amount > 0, MarketError::InsufficientTokens);

        // Calculate deposit fee.
        let deposit_amount = (buyout_price as u128)
            .checked_mul(config.listing_fee_bps as u128)
            .unwrap()
            .checked_div(10_000)
            .unwrap() as u64;

        // Transfer deposit SOL from seller to listing PDA.
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.seller.to_account_info(),
                    to: ctx.accounts.listing.to_account_info(),
                },
            ),
            deposit_amount,
        )?;

        // Transfer item tokens from seller ATA to listing escrow ATA.
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                TokenTransfer {
                    from: ctx.accounts.seller_ata.to_account_info(),
                    to: ctx.accounts.escrow_ata.to_account_info(),
                    authority: ctx.accounts.seller.to_account_info(),
                },
            ),
            item_amount,
        )?;

        let clock = Clock::get()?;
        let listing_id = config.listing_count;

        let listing = &mut ctx.accounts.listing;
        listing.listing_id = listing_id;
        listing.seller = ctx.accounts.seller.key();
        listing.item_mint = ctx.accounts.item_mint.key();
        listing.item_amount = item_amount;
        listing.buyout_price = buyout_price;
        listing.current_bid = 0;
        listing.current_bidder = Pubkey::default();
        listing.deposit_amount = deposit_amount;
        listing.created_at = clock.unix_timestamp;
        listing.expires_at = clock
            .unix_timestamp
            .checked_add(
                (duration_hours as i64)
                    .checked_mul(SECONDS_PER_HOUR)
                    .unwrap(),
            )
            .unwrap();
        listing.state = ListingState::Active;
        listing.bump = ctx.bumps.listing;

        // Increment listing counter.
        let config = &mut ctx.accounts.market_config;
        config.listing_count = config.listing_count.checked_add(1).unwrap();

        emit!(ListingCreated {
            listing_id,
            seller: ctx.accounts.seller.key(),
            item_mint: ctx.accounts.item_mint.key(),
            amount: item_amount,
            buyout_price,
        });

        Ok(())
    }

    /// Places a bid on an active listing.
    pub fn place_bid(ctx: Context<PlaceBid>, listing_id: u64, bid_amount: u64) -> Result<()> {
        let config = &ctx.accounts.market_config;
        require!(!config.paused, MarketError::MarketPaused);

        let listing = &mut ctx.accounts.listing;
        require!(
            listing.state == ListingState::Active,
            MarketError::ListingNotActive
        );

        let clock = Clock::get()?;
        require!(
            clock.unix_timestamp < listing.expires_at,
            MarketError::ListingExpiredError
        );
        require!(bid_amount > listing.current_bid, MarketError::BidTooLow);
        require!(
            bid_amount < listing.buyout_price,
            MarketError::BidExceedsBuyout
        );

        // Refund previous bidder if any.
        if listing.current_bidder != Pubkey::default() {
            let prev_bid = listing.current_bid;
            let listing_info = listing.to_account_info();
            let prev_bidder_info = ctx.accounts.previous_bidder.to_account_info();

            **listing_info.try_borrow_mut_lamports()? = listing_info
                .lamports()
                .checked_sub(prev_bid)
                .unwrap();
            **prev_bidder_info.try_borrow_mut_lamports()? = prev_bidder_info
                .lamports()
                .checked_add(prev_bid)
                .unwrap();
        }

        // Transfer bid SOL from bidder to listing PDA.
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.bidder.to_account_info(),
                    to: listing.to_account_info(),
                },
            ),
            bid_amount,
        )?;

        listing.current_bid = bid_amount;
        listing.current_bidder = ctx.accounts.bidder.key();

        emit!(BidPlaced {
            listing_id,
            bidder: ctx.accounts.bidder.key(),
            amount: bid_amount,
        });

        Ok(())
    }

    /// Instant purchase at buyout price.
    pub fn buyout(ctx: Context<Buyout>, listing_id: u64) -> Result<()> {
        let config = &ctx.accounts.market_config;
        require!(!config.paused, MarketError::MarketPaused);

        let listing = &mut ctx.accounts.listing;
        require!(
            listing.state == ListingState::Active,
            MarketError::ListingNotActive
        );

        let clock = Clock::get()?;
        require!(
            clock.unix_timestamp < listing.expires_at,
            MarketError::ListingExpiredError
        );

        let buyout_price = listing.buyout_price;
        let deposit = listing.deposit_amount;
        let item_amount = listing.item_amount;
        let lid_bytes = listing.listing_id.to_le_bytes();
        let listing_bump = listing.bump;
        let listing_seeds: &[&[&[u8]]] = &[&[LISTING_SEED, &lid_bytes, &[listing_bump]]];

        // ── ALL CPIs FIRST (before any manual lamport manipulation) ──

        // 1. Transfer buyout price SOL from buyer to listing PDA.
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.buyer.to_account_info(),
                    to: listing.to_account_info(),
                },
            ),
            buyout_price,
        )?;

        // 2. Transfer items from escrow to buyer ATA.
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                TokenTransfer {
                    from: ctx.accounts.escrow_ata.to_account_info(),
                    to: ctx.accounts.buyer_ata.to_account_info(),
                    authority: listing.to_account_info(),
                },
                listing_seeds,
            ),
            item_amount,
        )?;

        // 3. Close the escrow ATA, sending rent to seller.
        token::close_account(CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            token::CloseAccount {
                account: ctx.accounts.escrow_ata.to_account_info(),
                destination: ctx.accounts.seller.to_account_info(),
                authority: listing.to_account_info(),
            },
            listing_seeds,
        ))?;

        // ── NOW manual lamport manipulation (after all CPIs) ──

        listing.state = ListingState::Sold;

        // 4. Refund previous bidder if any.
        if listing.current_bidder != Pubkey::default() {
            let prev_bid = listing.current_bid;
            let listing_info = listing.to_account_info();
            let prev_bidder_info = ctx.accounts.previous_bidder.to_account_info();

            **listing_info.try_borrow_mut_lamports()? = listing_info
                .lamports()
                .checked_sub(prev_bid)
                .unwrap();
            **prev_bidder_info.try_borrow_mut_lamports()? = prev_bidder_info
                .lamports()
                .checked_add(prev_bid)
                .unwrap();
        }

        // 5. Calculate and transfer fee to treasury.
        let fee = (buyout_price as u128)
            .checked_mul(config.sale_fee_bps as u128)
            .unwrap()
            .checked_div(10_000)
            .unwrap() as u64;
        let seller_proceeds = buyout_price
            .checked_sub(fee)
            .unwrap()
            .checked_add(deposit)
            .unwrap();

        {
            let listing_info = listing.to_account_info();
            let treasury_info = ctx.accounts.treasury.to_account_info();

            **listing_info.try_borrow_mut_lamports()? = listing_info
                .lamports()
                .checked_sub(fee)
                .unwrap();
            **treasury_info.try_borrow_mut_lamports()? = treasury_info
                .lamports()
                .checked_add(fee)
                .unwrap();
        }

        // 6. Transfer proceeds + deposit to seller.
        {
            let listing_info = listing.to_account_info();
            let seller_info = ctx.accounts.seller.to_account_info();

            **listing_info.try_borrow_mut_lamports()? = listing_info
                .lamports()
                .checked_sub(seller_proceeds)
                .unwrap();
            **seller_info.try_borrow_mut_lamports()? = seller_info
                .lamports()
                .checked_add(seller_proceeds)
                .unwrap();
        }

        // 7. Close listing PDA, sending remaining lamports (rent) to seller.
        {
            let listing_info = listing.to_account_info();
            let seller_info = ctx.accounts.seller.to_account_info();
            let remaining = listing_info.lamports();

            **listing_info.try_borrow_mut_lamports()? = 0;
            **seller_info.try_borrow_mut_lamports()? = seller_info
                .lamports()
                .checked_add(remaining)
                .unwrap();
        }

        emit!(ListingSold {
            listing_id,
            buyer: ctx.accounts.buyer.key(),
            price: buyout_price,
            fee,
        });

        Ok(())
    }

    /// Cancel a listing that has no bids.
    pub fn cancel_listing(ctx: Context<CancelListing>, listing_id: u64) -> Result<()> {
        let listing = &mut ctx.accounts.listing;
        require!(
            listing.state == ListingState::Active,
            MarketError::ListingNotActive
        );
        require!(
            listing.seller == ctx.accounts.seller.key(),
            MarketError::NotSeller
        );
        require!(
            listing.current_bidder == Pubkey::default(),
            MarketError::HasBids
        );

        listing.state = ListingState::Cancelled;

        let item_amount = listing.item_amount;
        let lid_bytes = listing.listing_id.to_le_bytes();
        let listing_bump = listing.bump;
        let listing_seeds: &[&[&[u8]]] = &[&[LISTING_SEED, &lid_bytes, &[listing_bump]]];

        // Return items from escrow to seller ATA.
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                TokenTransfer {
                    from: ctx.accounts.escrow_ata.to_account_info(),
                    to: ctx.accounts.seller_ata.to_account_info(),
                    authority: listing.to_account_info(),
                },
                listing_seeds,
            ),
            item_amount,
        )?;

        // Close the escrow ATA, sending rent to seller.
        token::close_account(CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            token::CloseAccount {
                account: ctx.accounts.escrow_ata.to_account_info(),
                destination: ctx.accounts.seller.to_account_info(),
                authority: listing.to_account_info(),
            },
            listing_seeds,
        ))?;

        // Return deposit + close listing PDA (remaining rent) to seller.
        {
            let listing_info = listing.to_account_info();
            let seller_info = ctx.accounts.seller.to_account_info();
            let remaining = listing_info.lamports();

            **listing_info.try_borrow_mut_lamports()? = 0;
            **seller_info.try_borrow_mut_lamports()? = seller_info
                .lamports()
                .checked_add(remaining)
                .unwrap();
        }

        emit!(ListingCancelled { listing_id });

        Ok(())
    }

    // =====================================================================
    // Authority-operated instructions (server signs on behalf of players)
    // =====================================================================

    /// Authority creates a listing on behalf of a seller.
    /// Mints items directly to escrow instead of transferring from seller ATA.
    pub fn create_listing_operated(
        ctx: Context<CreateListingOperated>,
        buyout_price: u64,
        duration_hours: u16,
        seller: Pubkey,
        item_amount: u64,
    ) -> Result<()> {
        let config = &ctx.accounts.market_config;
        require!(!config.paused, MarketError::MarketPaused);
        require!(
            ctx.accounts.authority.key() == config.authority,
            MarketError::NotAuthority
        );
        require!(buyout_price > 0, MarketError::InvalidPrice);
        require!(
            duration_hours >= MIN_DURATION_HOURS && duration_hours <= MAX_DURATION_HOURS,
            MarketError::InvalidDuration
        );
        require!(item_amount > 0, MarketError::InsufficientTokens);

        let registered = &ctx.accounts.registered_item;
        require!(registered.is_tradeable, MarketError::ItemNotTradeable);

        // Calculate deposit fee.
        let deposit_amount = (buyout_price as u128)
            .checked_mul(config.listing_fee_bps as u128)
            .unwrap()
            .checked_div(10_000)
            .unwrap() as u64;

        // Transfer deposit SOL from authority to listing PDA.
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.authority.to_account_info(),
                    to: ctx.accounts.listing.to_account_info(),
                },
            ),
            deposit_amount,
        )?;

        // Mint items to escrow ATA (market_config PDA is the mint authority).
        let bump = config.bump;
        let signer_seeds: &[&[&[u8]]] = &[&[MARKET_CONFIG_SEED, &[bump]]];

        token::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                MintTo {
                    mint: ctx.accounts.item_mint.to_account_info(),
                    to: ctx.accounts.escrow_ata.to_account_info(),
                    authority: ctx.accounts.market_config.to_account_info(),
                },
                signer_seeds,
            ),
            item_amount,
        )?;

        let clock = Clock::get()?;
        let listing_id = config.listing_count;

        let listing = &mut ctx.accounts.listing;
        listing.listing_id = listing_id;
        listing.seller = seller;
        listing.item_mint = ctx.accounts.item_mint.key();
        listing.item_amount = item_amount;
        listing.buyout_price = buyout_price;
        listing.current_bid = 0;
        listing.current_bidder = Pubkey::default();
        listing.deposit_amount = deposit_amount;
        listing.created_at = clock.unix_timestamp;
        listing.expires_at = clock
            .unix_timestamp
            .checked_add(
                (duration_hours as i64)
                    .checked_mul(SECONDS_PER_HOUR)
                    .unwrap(),
            )
            .unwrap();
        listing.state = ListingState::Active;
        listing.bump = ctx.bumps.listing;

        let config = &mut ctx.accounts.market_config;
        config.listing_count = config.listing_count.checked_add(1).unwrap();

        emit!(ListingCreated {
            listing_id,
            seller,
            item_mint: ctx.accounts.item_mint.key(),
            amount: item_amount,
            buyout_price,
        });

        Ok(())
    }

    /// Authority places a bid on behalf of a bidder.
    pub fn place_bid_operated(
        ctx: Context<PlaceBidOperated>,
        listing_id: u64,
        bid_amount: u64,
        bidder: Pubkey,
    ) -> Result<()> {
        let config = &ctx.accounts.market_config;
        require!(!config.paused, MarketError::MarketPaused);
        require!(
            ctx.accounts.authority.key() == config.authority,
            MarketError::NotAuthority
        );

        let listing = &mut ctx.accounts.listing;
        require!(
            listing.state == ListingState::Active,
            MarketError::ListingNotActive
        );

        let clock = Clock::get()?;
        require!(
            clock.unix_timestamp < listing.expires_at,
            MarketError::ListingExpiredError
        );
        require!(bid_amount > listing.current_bid, MarketError::BidTooLow);
        require!(
            bid_amount < listing.buyout_price,
            MarketError::BidExceedsBuyout
        );

        // Refund previous bid to authority (all bids come from authority).
        if listing.current_bidder != Pubkey::default() {
            let prev_bid = listing.current_bid;
            let listing_info = listing.to_account_info();
            let authority_info = ctx.accounts.authority.to_account_info();

            **listing_info.try_borrow_mut_lamports()? = listing_info
                .lamports()
                .checked_sub(prev_bid)
                .unwrap();
            **authority_info.try_borrow_mut_lamports()? = authority_info
                .lamports()
                .checked_add(prev_bid)
                .unwrap();
        }

        // Transfer bid SOL from authority to listing PDA.
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.authority.to_account_info(),
                    to: listing.to_account_info(),
                },
            ),
            bid_amount,
        )?;

        listing.current_bid = bid_amount;
        listing.current_bidder = bidder;

        emit!(BidPlaced {
            listing_id,
            bidder,
            amount: bid_amount,
        });

        Ok(())
    }

    /// Authority performs buyout on behalf of a buyer.
    /// Burns escrowed items and returns all SOL to authority.
    pub fn buyout_operated(
        ctx: Context<BuyoutOperated>,
        listing_id: u64,
        buyer: Pubkey,
    ) -> Result<()> {
        let config = &ctx.accounts.market_config;
        require!(!config.paused, MarketError::MarketPaused);
        require!(
            ctx.accounts.authority.key() == config.authority,
            MarketError::NotAuthority
        );

        let listing = &mut ctx.accounts.listing;
        require!(
            listing.state == ListingState::Active,
            MarketError::ListingNotActive
        );

        let clock = Clock::get()?;
        require!(
            clock.unix_timestamp < listing.expires_at,
            MarketError::ListingExpiredError
        );

        let buyout_price = listing.buyout_price;
        let item_amount = listing.item_amount;
        let lid_bytes = listing.listing_id.to_le_bytes();
        let listing_bump = listing.bump;
        let listing_seeds: &[&[&[u8]]] = &[&[LISTING_SEED, &lid_bytes, &[listing_bump]]];

        // ── ALL CPIs FIRST ──

        // 1. Transfer buyout price from authority to listing PDA.
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.authority.to_account_info(),
                    to: listing.to_account_info(),
                },
            ),
            buyout_price,
        )?;

        // 2. Burn items from escrow (virtual items minted by authority).
        token::burn(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Burn {
                    mint: ctx.accounts.item_mint.to_account_info(),
                    from: ctx.accounts.escrow_ata.to_account_info(),
                    authority: listing.to_account_info(),
                },
                listing_seeds,
            ),
            item_amount,
        )?;

        // 3. Close escrow ATA, sending rent to authority.
        token::close_account(CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            token::CloseAccount {
                account: ctx.accounts.escrow_ata.to_account_info(),
                destination: ctx.accounts.authority.to_account_info(),
                authority: listing.to_account_info(),
            },
            listing_seeds,
        ))?;

        // ── Manual lamport manipulation ──

        listing.state = ListingState::Sold;

        // 4. Refund previous bid to authority.
        if listing.current_bidder != Pubkey::default() {
            let prev_bid = listing.current_bid;
            let listing_info = listing.to_account_info();
            let authority_info = ctx.accounts.authority.to_account_info();

            **listing_info.try_borrow_mut_lamports()? = listing_info
                .lamports()
                .checked_sub(prev_bid)
                .unwrap();
            **authority_info.try_borrow_mut_lamports()? = authority_info
                .lamports()
                .checked_add(prev_bid)
                .unwrap();
        }

        // 5. Fee to treasury.
        let fee = (buyout_price as u128)
            .checked_mul(config.sale_fee_bps as u128)
            .unwrap()
            .checked_div(10_000)
            .unwrap() as u64;

        {
            let listing_info = listing.to_account_info();
            let treasury_info = ctx.accounts.treasury.to_account_info();

            **listing_info.try_borrow_mut_lamports()? = listing_info
                .lamports()
                .checked_sub(fee)
                .unwrap();
            **treasury_info.try_borrow_mut_lamports()? = treasury_info
                .lamports()
                .checked_add(fee)
                .unwrap();
        }

        // 6. Close listing PDA, all remaining to authority.
        {
            let listing_info = listing.to_account_info();
            let authority_info = ctx.accounts.authority.to_account_info();
            let remaining = listing_info.lamports();

            **listing_info.try_borrow_mut_lamports()? = 0;
            **authority_info.try_borrow_mut_lamports()? = authority_info
                .lamports()
                .checked_add(remaining)
                .unwrap();
        }

        emit!(ListingSold {
            listing_id,
            buyer,
            price: buyout_price,
            fee,
        });

        Ok(())
    }

    /// Authority cancels a listing. Burns escrowed items and reclaims SOL.
    pub fn cancel_listing_operated(
        ctx: Context<CancelListingOperated>,
        listing_id: u64,
    ) -> Result<()> {
        let config = &ctx.accounts.market_config;
        require!(
            ctx.accounts.authority.key() == config.authority,
            MarketError::NotAuthority
        );

        let listing = &mut ctx.accounts.listing;
        require!(
            listing.state == ListingState::Active,
            MarketError::ListingNotActive
        );
        require!(
            listing.current_bidder == Pubkey::default(),
            MarketError::HasBids
        );

        listing.state = ListingState::Cancelled;

        let item_amount = listing.item_amount;
        let lid_bytes = listing.listing_id.to_le_bytes();
        let listing_bump = listing.bump;
        let listing_seeds: &[&[&[u8]]] = &[&[LISTING_SEED, &lid_bytes, &[listing_bump]]];

        // Burn items from escrow.
        token::burn(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Burn {
                    mint: ctx.accounts.item_mint.to_account_info(),
                    from: ctx.accounts.escrow_ata.to_account_info(),
                    authority: listing.to_account_info(),
                },
                listing_seeds,
            ),
            item_amount,
        )?;

        // Close escrow ATA, rent to authority.
        token::close_account(CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            token::CloseAccount {
                account: ctx.accounts.escrow_ata.to_account_info(),
                destination: ctx.accounts.authority.to_account_info(),
                authority: listing.to_account_info(),
            },
            listing_seeds,
        ))?;

        // Close listing PDA, all remaining to authority.
        {
            let listing_info = listing.to_account_info();
            let authority_info = ctx.accounts.authority.to_account_info();
            let remaining = listing_info.lamports();

            **listing_info.try_borrow_mut_lamports()? = 0;
            **authority_info.try_borrow_mut_lamports()? = authority_info
                .lamports()
                .checked_add(remaining)
                .unwrap();
        }

        emit!(ListingCancelled { listing_id });

        Ok(())
    }

    /// Claim items and funds from an expired listing.
    pub fn claim_expired(ctx: Context<ClaimExpired>, listing_id: u64) -> Result<()> {
        let listing = &mut ctx.accounts.listing;
        require!(
            listing.state == ListingState::Active,
            MarketError::ListingNotActive
        );

        let clock = Clock::get()?;
        require!(
            clock.unix_timestamp >= listing.expires_at,
            MarketError::ListingNotActive
        );

        let item_amount = listing.item_amount;
        let deposit = listing.deposit_amount;
        let lid_bytes = listing.listing_id.to_le_bytes();
        let listing_bump = listing.bump;
        let listing_seeds: &[&[&[u8]]] = &[&[LISTING_SEED, &lid_bytes, &[listing_bump]]];

        let had_bids = listing.current_bidder != Pubkey::default();

        // ── ALL CPIs FIRST (before any manual lamport manipulation) ──
        // token::transfer is the same for both branches (escrow -> bidder_or_seller_ata).
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                TokenTransfer {
                    from: ctx.accounts.escrow_ata.to_account_info(),
                    to: ctx.accounts.bidder_or_seller_ata.to_account_info(),
                    authority: listing.to_account_info(),
                },
                listing_seeds,
            ),
            item_amount,
        )?;

        // Close escrow ATA, sending rent to seller.
        token::close_account(CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            token::CloseAccount {
                account: ctx.accounts.escrow_ata.to_account_info(),
                destination: ctx.accounts.seller.to_account_info(),
                authority: listing.to_account_info(),
            },
            listing_seeds,
        ))?;

        // ── NOW manual lamport manipulation (after all CPIs) ──

        listing.state = ListingState::Expired;

        if had_bids {
            // Treat as a sale at current_bid price.
            let sale_price = listing.current_bid;
            let config = &ctx.accounts.market_config;
            let fee = (sale_price as u128)
                .checked_mul(config.sale_fee_bps as u128)
                .unwrap()
                .checked_div(10_000)
                .unwrap() as u64;
            let seller_proceeds = sale_price
                .checked_sub(fee)
                .unwrap()
                .checked_add(deposit)
                .unwrap();

            // Transfer fee to treasury.
            {
                let listing_info = listing.to_account_info();
                let treasury_info = ctx.accounts.treasury.to_account_info();

                **listing_info.try_borrow_mut_lamports()? = listing_info
                    .lamports()
                    .checked_sub(fee)
                    .unwrap();
                **treasury_info.try_borrow_mut_lamports()? = treasury_info
                    .lamports()
                    .checked_add(fee)
                    .unwrap();
            }

            // Transfer proceeds + deposit to seller.
            {
                let listing_info = listing.to_account_info();
                let seller_info = ctx.accounts.seller.to_account_info();

                **listing_info.try_borrow_mut_lamports()? = listing_info
                    .lamports()
                    .checked_sub(seller_proceeds)
                    .unwrap();
                **seller_info.try_borrow_mut_lamports()? = seller_info
                    .lamports()
                    .checked_add(seller_proceeds)
                    .unwrap();
            }

            emit!(ListingSold {
                listing_id,
                buyer: listing.current_bidder,
                price: sale_price,
                fee,
            });
        }

        // Close listing PDA, sending remaining lamports to seller.
        {
            let listing_info = listing.to_account_info();
            let seller_info = ctx.accounts.seller.to_account_info();
            let remaining = listing_info.lamports();

            **listing_info.try_borrow_mut_lamports()? = 0;
            **seller_info.try_borrow_mut_lamports()? = seller_info
                .lamports()
                .checked_add(remaining)
                .unwrap();
        }

        emit!(ListingExpired {
            listing_id,
            had_bids,
        });

        Ok(())
    }

    // =====================================================================
    // NFT Marketplace Instructions (Metaplex Core assets)
    // =====================================================================

    /// Initialize the NFT config PDA (separate counter for NFT listings).
    pub fn initialize_nft_config(ctx: Context<InitializeNftConfig>) -> Result<()> {
        let config = &ctx.accounts.market_config;
        require!(
            ctx.accounts.authority.key() == config.authority,
            MarketError::NotAuthority
        );

        let nft_config = &mut ctx.accounts.nft_config;
        nft_config.listing_count = 0;
        nft_config.bump = ctx.bumps.nft_config;
        Ok(())
    }

    /// Authority creates an NFT listing. The Metaplex Core asset must already
    /// exist and be owned by the authority. This instruction transfers the NFT
    /// to the NftListing PDA for escrow.
    pub fn create_nft_listing_operated(
        ctx: Context<CreateNftListingOperated>,
        buyout_price: u64,
        duration_hours: u16,
        seller: Pubkey,
        item_id: u32,
    ) -> Result<()> {
        let config = &ctx.accounts.market_config;
        require!(!config.paused, MarketError::MarketPaused);
        require!(
            ctx.accounts.authority.key() == config.authority,
            MarketError::NotAuthority
        );
        require!(buyout_price > 0, MarketError::InvalidPrice);
        require!(
            duration_hours >= MIN_DURATION_HOURS && duration_hours <= MAX_DURATION_HOURS,
            MarketError::InvalidDuration
        );

        // Calculate deposit fee.
        let deposit_amount = (buyout_price as u128)
            .checked_mul(config.listing_fee_bps as u128)
            .unwrap()
            .checked_div(10_000)
            .unwrap() as u64;

        // Transfer deposit SOL from authority to NftListing PDA.
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.authority.to_account_info(),
                    to: ctx.accounts.nft_listing.to_account_info(),
                },
            ),
            deposit_amount,
        )?;

        // Transfer Metaplex Core asset from authority to NftListing PDA.
        TransferV1CpiBuilder::new(&ctx.accounts.mpl_core_program)
            .asset(&ctx.accounts.asset)
            .collection(Some(&ctx.accounts.collection.to_account_info()))
            .payer(&ctx.accounts.authority)
            .authority(Some(&ctx.accounts.authority.to_account_info()))
            .new_owner(&ctx.accounts.nft_listing.to_account_info())
            .system_program(Some(&ctx.accounts.system_program.to_account_info()))
            .invoke()?;

        let clock = Clock::get()?;
        let listing_id = ctx.accounts.nft_config.listing_count;

        let nft_listing = &mut ctx.accounts.nft_listing;
        nft_listing.listing_id = listing_id;
        nft_listing.seller = seller;
        nft_listing.asset = ctx.accounts.asset.key();
        nft_listing.buyout_price = buyout_price;
        nft_listing.current_bid = 0;
        nft_listing.current_bidder = Pubkey::default();
        nft_listing.deposit_amount = deposit_amount;
        nft_listing.created_at = clock.unix_timestamp;
        nft_listing.expires_at = clock
            .unix_timestamp
            .checked_add(
                (duration_hours as i64)
                    .checked_mul(SECONDS_PER_HOUR)
                    .unwrap(),
            )
            .unwrap();
        nft_listing.state = ListingState::Active;
        nft_listing.item_id = item_id;
        nft_listing.bump = ctx.bumps.nft_listing;

        // Increment NFT listing counter.
        let nft_config = &mut ctx.accounts.nft_config;
        nft_config.listing_count = nft_config.listing_count.checked_add(1).unwrap();

        emit!(NftListingCreated {
            listing_id,
            seller,
            asset: ctx.accounts.asset.key(),
            item_id,
            buyout_price,
        });

        Ok(())
    }

    /// Authority places a bid on an NFT listing on behalf of a bidder.
    pub fn place_nft_bid_operated(
        ctx: Context<PlaceNftBidOperated>,
        listing_id: u64,
        bid_amount: u64,
        bidder: Pubkey,
    ) -> Result<()> {
        let config = &ctx.accounts.market_config;
        require!(!config.paused, MarketError::MarketPaused);
        require!(
            ctx.accounts.authority.key() == config.authority,
            MarketError::NotAuthority
        );

        let listing = &mut ctx.accounts.nft_listing;
        require!(
            listing.state == ListingState::Active,
            MarketError::ListingNotActive
        );

        let clock = Clock::get()?;
        require!(
            clock.unix_timestamp < listing.expires_at,
            MarketError::ListingExpiredError
        );
        require!(bid_amount > listing.current_bid, MarketError::BidTooLow);
        require!(
            bid_amount < listing.buyout_price,
            MarketError::BidExceedsBuyout
        );

        // Refund previous bid to authority.
        if listing.current_bidder != Pubkey::default() {
            let prev_bid = listing.current_bid;
            let listing_info = listing.to_account_info();
            let authority_info = ctx.accounts.authority.to_account_info();

            **listing_info.try_borrow_mut_lamports()? = listing_info
                .lamports()
                .checked_sub(prev_bid)
                .unwrap();
            **authority_info.try_borrow_mut_lamports()? = authority_info
                .lamports()
                .checked_add(prev_bid)
                .unwrap();
        }

        // Transfer bid SOL from authority to NftListing PDA.
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.authority.to_account_info(),
                    to: listing.to_account_info(),
                },
            ),
            bid_amount,
        )?;

        listing.current_bid = bid_amount;
        listing.current_bidder = bidder;

        emit!(NftBidPlaced {
            listing_id,
            bidder,
            amount: bid_amount,
        });

        Ok(())
    }

    /// Authority performs buyout of an NFT listing on behalf of a buyer.
    /// Transfers NFT to buyer wallet, distributes SOL (fee to treasury, proceeds to authority).
    pub fn buyout_nft_operated(
        ctx: Context<BuyoutNftOperated>,
        listing_id: u64,
        buyer: Pubkey,
    ) -> Result<()> {
        let config = &ctx.accounts.market_config;
        require!(!config.paused, MarketError::MarketPaused);
        require!(
            ctx.accounts.authority.key() == config.authority,
            MarketError::NotAuthority
        );

        let listing = &mut ctx.accounts.nft_listing;
        require!(
            listing.state == ListingState::Active,
            MarketError::ListingNotActive
        );

        let clock = Clock::get()?;
        require!(
            clock.unix_timestamp < listing.expires_at,
            MarketError::ListingExpiredError
        );

        let buyout_price = listing.buyout_price;
        let lid_bytes = listing.listing_id.to_le_bytes();
        let listing_bump = listing.bump;
        let listing_seeds: &[&[&[u8]]] = &[&[NFT_LISTING_SEED, &lid_bytes, &[listing_bump]]];

        // ── CPIs FIRST ──

        // 1. Transfer buyout price from authority to NftListing PDA.
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.authority.to_account_info(),
                    to: listing.to_account_info(),
                },
            ),
            buyout_price,
        )?;

        // 2. Transfer NFT from NftListing PDA to buyer wallet.
        TransferV1CpiBuilder::new(&ctx.accounts.mpl_core_program)
            .asset(&ctx.accounts.asset)
            .collection(Some(&ctx.accounts.collection.to_account_info()))
            .payer(&ctx.accounts.authority)
            .authority(Some(&listing.to_account_info()))
            .new_owner(&ctx.accounts.buyer_wallet)
            .system_program(Some(&ctx.accounts.system_program.to_account_info()))
            .invoke_signed(listing_seeds)?;

        // ── Manual lamport manipulation ──

        listing.state = ListingState::Sold;

        // 3. Refund previous bid to authority.
        if listing.current_bidder != Pubkey::default() {
            let prev_bid = listing.current_bid;
            let listing_info = listing.to_account_info();
            let authority_info = ctx.accounts.authority.to_account_info();

            **listing_info.try_borrow_mut_lamports()? = listing_info
                .lamports()
                .checked_sub(prev_bid)
                .unwrap();
            **authority_info.try_borrow_mut_lamports()? = authority_info
                .lamports()
                .checked_add(prev_bid)
                .unwrap();
        }

        // 4. Fee to treasury.
        let fee = (buyout_price as u128)
            .checked_mul(config.sale_fee_bps as u128)
            .unwrap()
            .checked_div(10_000)
            .unwrap() as u64;

        {
            let listing_info = listing.to_account_info();
            let treasury_info = ctx.accounts.treasury.to_account_info();

            **listing_info.try_borrow_mut_lamports()? = listing_info
                .lamports()
                .checked_sub(fee)
                .unwrap();
            **treasury_info.try_borrow_mut_lamports()? = treasury_info
                .lamports()
                .checked_add(fee)
                .unwrap();
        }

        // 5. All remaining SOL to authority.
        {
            let listing_info = listing.to_account_info();
            let authority_info = ctx.accounts.authority.to_account_info();
            let remaining = listing_info.lamports();

            **listing_info.try_borrow_mut_lamports()? = 0;
            **authority_info.try_borrow_mut_lamports()? = authority_info
                .lamports()
                .checked_add(remaining)
                .unwrap();
        }

        emit!(NftListingSold {
            listing_id,
            buyer,
            price: buyout_price,
            fee,
        });

        Ok(())
    }

    /// Authority cancels an NFT listing (no bids). NFT transferred to seller wallet.
    pub fn cancel_nft_listing_operated(
        ctx: Context<CancelNftListingOperated>,
        listing_id: u64,
    ) -> Result<()> {
        let config = &ctx.accounts.market_config;
        require!(
            ctx.accounts.authority.key() == config.authority,
            MarketError::NotAuthority
        );

        let listing = &mut ctx.accounts.nft_listing;
        require!(
            listing.state == ListingState::Active,
            MarketError::ListingNotActive
        );
        require!(
            listing.current_bidder == Pubkey::default(),
            MarketError::HasBids
        );

        listing.state = ListingState::Cancelled;

        let lid_bytes = listing.listing_id.to_le_bytes();
        let listing_bump = listing.bump;
        let listing_seeds: &[&[&[u8]]] = &[&[NFT_LISTING_SEED, &lid_bytes, &[listing_bump]]];

        // Transfer NFT from NftListing PDA to seller wallet.
        TransferV1CpiBuilder::new(&ctx.accounts.mpl_core_program)
            .asset(&ctx.accounts.asset)
            .collection(Some(&ctx.accounts.collection.to_account_info()))
            .payer(&ctx.accounts.authority)
            .authority(Some(&listing.to_account_info()))
            .new_owner(&ctx.accounts.seller_wallet)
            .system_program(Some(&ctx.accounts.system_program.to_account_info()))
            .invoke_signed(listing_seeds)?;

        // Return deposit + close listing PDA to authority.
        {
            let listing_info = listing.to_account_info();
            let authority_info = ctx.accounts.authority.to_account_info();
            let remaining = listing_info.lamports();

            **listing_info.try_borrow_mut_lamports()? = 0;
            **authority_info.try_borrow_mut_lamports()? = authority_info
                .lamports()
                .checked_add(remaining)
                .unwrap();
        }

        emit!(NftListingCancelled { listing_id });

        Ok(())
    }

    /// Claim an expired NFT listing. If bids: NFT → highest bidder, SOL → authority.
    /// If no bids: NFT → seller, deposit → authority.
    pub fn claim_nft_expired(
        ctx: Context<ClaimNftExpired>,
        listing_id: u64,
    ) -> Result<()> {
        let listing = &mut ctx.accounts.nft_listing;
        require!(
            listing.state == ListingState::Active,
            MarketError::ListingNotActive
        );

        let clock = Clock::get()?;
        require!(
            clock.unix_timestamp >= listing.expires_at,
            MarketError::ListingNotActive
        );

        let lid_bytes = listing.listing_id.to_le_bytes();
        let listing_bump = listing.bump;
        let listing_seeds: &[&[&[u8]]] = &[&[NFT_LISTING_SEED, &lid_bytes, &[listing_bump]]];

        let had_bids = listing.current_bidder != Pubkey::default();

        // Transfer NFT to recipient (bidder if bids, seller if no bids).
        TransferV1CpiBuilder::new(&ctx.accounts.mpl_core_program)
            .asset(&ctx.accounts.asset)
            .collection(Some(&ctx.accounts.collection.to_account_info()))
            .payer(&ctx.accounts.authority)
            .authority(Some(&listing.to_account_info()))
            .new_owner(&ctx.accounts.nft_recipient)
            .system_program(Some(&ctx.accounts.system_program.to_account_info()))
            .invoke_signed(listing_seeds)?;

        listing.state = ListingState::Expired;

        if had_bids {
            // Treat as sale at current_bid price.
            let sale_price = listing.current_bid;
            let config = &ctx.accounts.market_config;
            let fee = (sale_price as u128)
                .checked_mul(config.sale_fee_bps as u128)
                .unwrap()
                .checked_div(10_000)
                .unwrap() as u64;

            // Fee to treasury.
            {
                let listing_info = listing.to_account_info();
                let treasury_info = ctx.accounts.treasury.to_account_info();

                **listing_info.try_borrow_mut_lamports()? = listing_info
                    .lamports()
                    .checked_sub(fee)
                    .unwrap();
                **treasury_info.try_borrow_mut_lamports()? = treasury_info
                    .lamports()
                    .checked_add(fee)
                    .unwrap();
            }

            emit!(NftListingSold {
                listing_id,
                buyer: listing.current_bidder,
                price: sale_price,
                fee,
            });
        }

        // Close listing PDA, all remaining to authority.
        {
            let listing_info = listing.to_account_info();
            let authority_info = ctx.accounts.authority.to_account_info();
            let remaining = listing_info.lamports();

            **listing_info.try_borrow_mut_lamports()? = 0;
            **authority_info.try_borrow_mut_lamports()? = authority_info
                .lamports()
                .checked_add(remaining)
                .unwrap();
        }

        emit!(NftListingExpired {
            listing_id,
            had_bids,
        });

        Ok(())
    }

    // =====================================================================
    // Escrow Management Instructions
    // =====================================================================

    /// Player deposits SOL into their PlayerEscrow PDA.
    pub fn deposit_escrow(ctx: Context<DepositEscrow>, amount: u64) -> Result<()> {
        require!(amount > 0, MarketError::InvalidPrice);

        // Transfer SOL from player to escrow PDA.
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.player.to_account_info(),
                to: ctx.accounts.player_escrow.to_account_info(),
            },
        );
        system_program::transfer(cpi_context, amount)?;

        let escrow = &mut ctx.accounts.player_escrow;
        escrow.player = ctx.accounts.player.key();
        escrow.balance = escrow.balance.checked_add(amount).unwrap();
        escrow.bump = ctx.bumps.player_escrow;

        Ok(())
    }

    /// Player withdraws SOL from their PlayerEscrow PDA.
    pub fn withdraw_escrow(ctx: Context<WithdrawEscrow>, amount: u64) -> Result<()> {
        require!(amount > 0, MarketError::InvalidPrice);

        let escrow = &mut ctx.accounts.player_escrow;
        require!(
            escrow.balance >= amount,
            MarketError::InsufficientEscrowBalance
        );

        escrow.balance = escrow.balance.checked_sub(amount).unwrap();

        // Transfer SOL from escrow PDA to player using lamport manipulation.
        {
            let escrow_info = escrow.to_account_info();
            let player_info = ctx.accounts.player.to_account_info();

            **escrow_info.try_borrow_mut_lamports()? = escrow_info
                .lamports()
                .checked_sub(amount)
                .unwrap();
            **player_info.try_borrow_mut_lamports()? = player_info
                .lamports()
                .checked_add(amount)
                .unwrap();
        }

        Ok(())
    }

    // =====================================================================
    // Generic Escrow Operated Instructions (for Arena, etc.)
    // =====================================================================

    /// Authority deducts SOL from a player's escrow. Used by arena/duel system
    /// to fund matches from player escrow balances.
    pub fn deduct_escrow_operated(
        ctx: Context<DeductEscrowOperated>,
        amount: u64,
        _player: Pubkey,
    ) -> Result<()> {
        let config = &ctx.accounts.market_config;
        require!(
            ctx.accounts.authority.key() == config.authority,
            MarketError::NotAuthority
        );
        require!(amount > 0, MarketError::InvalidPrice);

        let escrow = &mut ctx.accounts.player_escrow;
        require!(
            escrow.balance >= amount,
            MarketError::InsufficientEscrowBalance
        );

        escrow.balance = escrow.balance.checked_sub(amount).unwrap();

        // Transfer lamports from escrow PDA to authority.
        {
            let escrow_info = escrow.to_account_info();
            let authority_info = ctx.accounts.authority.to_account_info();

            **escrow_info.try_borrow_mut_lamports()? = escrow_info
                .lamports()
                .checked_sub(amount)
                .unwrap();
            **authority_info.try_borrow_mut_lamports()? = authority_info
                .lamports()
                .checked_add(amount)
                .unwrap();
        }

        Ok(())
    }

    /// Authority credits SOL to a player's escrow. Used by arena/duel system
    /// to deposit winnings or refunds into player escrow balances.
    pub fn credit_escrow_operated(
        ctx: Context<CreditEscrowOperated>,
        amount: u64,
        _player: Pubkey,
    ) -> Result<()> {
        let config = &ctx.accounts.market_config;
        require!(
            ctx.accounts.authority.key() == config.authority,
            MarketError::NotAuthority
        );
        require!(amount > 0, MarketError::InvalidPrice);

        // Transfer lamports from authority to escrow PDA.
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.authority.to_account_info(),
                to: ctx.accounts.player_escrow.to_account_info(),
            },
        );
        system_program::transfer(cpi_context, amount)?;

        let escrow = &mut ctx.accounts.player_escrow;
        escrow.balance = escrow.balance.checked_add(amount).unwrap();

        Ok(())
    }

    // =====================================================================
    // Escrow-Funded Operated Instructions
    // =====================================================================

    /// Authority places a bid on an NFT listing, funded from bidder's escrow.
    pub fn place_nft_bid_escrow(
        ctx: Context<PlaceNftBidEscrow>,
        listing_id: u64,
        bid_amount: u64,
        bidder: Pubkey,
    ) -> Result<()> {
        let config = &ctx.accounts.market_config;
        require!(!config.paused, MarketError::MarketPaused);
        require!(
            ctx.accounts.authority.key() == config.authority,
            MarketError::NotAuthority
        );

        let listing = &mut ctx.accounts.nft_listing;
        require!(
            listing.state == ListingState::Active,
            MarketError::ListingNotActive
        );

        let clock = Clock::get()?;
        require!(
            clock.unix_timestamp < listing.expires_at,
            MarketError::ListingExpiredError
        );
        require!(bid_amount > listing.current_bid, MarketError::BidTooLow);
        require!(
            bid_amount < listing.buyout_price,
            MarketError::BidExceedsBuyout
        );

        // Deduct from bidder's escrow.
        let bidder_escrow = &mut ctx.accounts.bidder_escrow;
        require!(
            bidder_escrow.balance >= bid_amount,
            MarketError::InsufficientEscrowBalance
        );
        bidder_escrow.balance = bidder_escrow.balance.checked_sub(bid_amount).unwrap();

        // Transfer SOL from bidder escrow to listing PDA.
        {
            let bidder_escrow_info = bidder_escrow.to_account_info();
            let listing_info = listing.to_account_info();

            **bidder_escrow_info.try_borrow_mut_lamports()? = bidder_escrow_info
                .lamports()
                .checked_sub(bid_amount)
                .unwrap();
            **listing_info.try_borrow_mut_lamports()? = listing_info
                .lamports()
                .checked_add(bid_amount)
                .unwrap();
        }

        // Refund previous bidder to their escrow (if any).
        if listing.current_bid > 0 && listing.current_bidder != Pubkey::default() {
            let refund = listing.current_bid;
            let prev_escrow = &mut ctx.accounts.prev_bidder_escrow;
            {
                let listing_info = listing.to_account_info();
                let prev_escrow_info = prev_escrow.to_account_info();

                **listing_info.try_borrow_mut_lamports()? = listing_info
                    .lamports()
                    .checked_sub(refund)
                    .unwrap();
                **prev_escrow_info.try_borrow_mut_lamports()? = prev_escrow_info
                    .lamports()
                    .checked_add(refund)
                    .unwrap();
            }
            prev_escrow.balance = prev_escrow.balance.checked_add(refund).unwrap();
        }

        listing.current_bid = bid_amount;
        listing.current_bidder = bidder;

        emit!(NftBidPlaced {
            listing_id,
            bidder,
            amount: bid_amount,
        });

        Ok(())
    }

    /// Authority performs buyout of an NFT listing, funded from buyer's escrow.
    /// Transfers NFT to buyer wallet, distributes SOL (fee to treasury,
    /// proceeds to seller's escrow if it exists, otherwise to authority).
    pub fn buyout_nft_escrow(
        ctx: Context<BuyoutNftEscrow>,
        listing_id: u64,
        buyer: Pubkey,
    ) -> Result<()> {
        let config = &ctx.accounts.market_config;
        require!(!config.paused, MarketError::MarketPaused);
        require!(
            ctx.accounts.authority.key() == config.authority,
            MarketError::NotAuthority
        );

        let listing = &mut ctx.accounts.nft_listing;
        require!(
            listing.state == ListingState::Active,
            MarketError::ListingNotActive
        );

        let clock = Clock::get()?;
        require!(
            clock.unix_timestamp < listing.expires_at,
            MarketError::ListingExpiredError
        );

        let buyout_price = listing.buyout_price;
        let lid_bytes = listing.listing_id.to_le_bytes();
        let listing_bump = listing.bump;
        let listing_seeds: &[&[&[u8]]] = &[&[NFT_LISTING_SEED, &lid_bytes, &[listing_bump]]];

        // Validate buyer escrow has sufficient balance.
        let buyer_escrow = &mut ctx.accounts.buyer_escrow;
        require!(
            buyer_escrow.balance >= buyout_price,
            MarketError::InsufficientEscrowBalance
        );

        // ── CPIs FIRST (identical to buyout_nft_operated) ──
        // Authority pays from its own funds. Escrow reimburses authority after all CPIs.

        // 1. Transfer buyout price from authority to NftListing PDA.
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.authority.to_account_info(),
                    to: listing.to_account_info(),
                },
            ),
            buyout_price,
        )?;

        // 2. Transfer NFT from NftListing PDA to buyer wallet.
        TransferV1CpiBuilder::new(&ctx.accounts.mpl_core_program)
            .asset(&ctx.accounts.asset)
            .collection(Some(&ctx.accounts.collection.to_account_info()))
            .payer(&ctx.accounts.authority)
            .authority(Some(&listing.to_account_info()))
            .new_owner(&ctx.accounts.buyer_wallet)
            .system_program(Some(&ctx.accounts.system_program.to_account_info()))
            .invoke_signed(listing_seeds)?;

        // ── All CPIs done. Raw lamport manipulation below. ──

        listing.state = ListingState::Sold;

        // 3. Reimburse authority from buyer escrow (raw, both program-owned).
        buyer_escrow.balance = buyer_escrow.balance.checked_sub(buyout_price).unwrap();
        {
            let buyer_escrow_info = buyer_escrow.to_account_info();
            let authority_info = ctx.accounts.authority.to_account_info();

            **buyer_escrow_info.try_borrow_mut_lamports()? = buyer_escrow_info
                .lamports()
                .checked_sub(buyout_price)
                .unwrap();
            **authority_info.try_borrow_mut_lamports()? = authority_info
                .lamports()
                .checked_add(buyout_price)
                .unwrap();
        }

        // 4. Refund previous bid to authority (operated bids come from authority).
        if listing.current_bidder != Pubkey::default() {
            let prev_bid = listing.current_bid;
            let listing_info = listing.to_account_info();
            let authority_info = ctx.accounts.authority.to_account_info();

            **listing_info.try_borrow_mut_lamports()? = listing_info
                .lamports()
                .checked_sub(prev_bid)
                .unwrap();
            **authority_info.try_borrow_mut_lamports()? = authority_info
                .lamports()
                .checked_add(prev_bid)
                .unwrap();
        }

        // 5. Fee to treasury.
        let fee = (buyout_price as u128)
            .checked_mul(config.sale_fee_bps as u128)
            .unwrap()
            .checked_div(10_000)
            .unwrap() as u64;

        {
            let listing_info = listing.to_account_info();
            let treasury_info = ctx.accounts.treasury.to_account_info();

            **listing_info.try_borrow_mut_lamports()? = listing_info
                .lamports()
                .checked_sub(fee)
                .unwrap();
            **treasury_info.try_borrow_mut_lamports()? = treasury_info
                .lamports()
                .checked_add(fee)
                .unwrap();
        }

        // 6. All remaining SOL (proceeds + deposit) to authority.
        {
            let listing_info = listing.to_account_info();
            let authority_info = ctx.accounts.authority.to_account_info();
            let remaining = listing_info.lamports();

            **listing_info.try_borrow_mut_lamports()? = 0;
            **authority_info.try_borrow_mut_lamports()? = authority_info
                .lamports()
                .checked_add(remaining)
                .unwrap();
        }

        emit!(NftListingSold {
            listing_id,
            buyer,
            price: buyout_price,
            fee,
        });

        Ok(())
    }

    // =====================================================================
    // Non-Operated NFT Instructions (player signs directly, for website)
    // =====================================================================

    /// Player creates an NFT listing. Transfers Metaplex Core asset to NftListing PDA.
    pub fn create_nft_listing(
        ctx: Context<CreateNftListing>,
        buyout_price: u64,
        duration_hours: u16,
        item_id: u32,
    ) -> Result<()> {
        let config = &ctx.accounts.market_config;
        require!(!config.paused, MarketError::MarketPaused);
        require!(buyout_price > 0, MarketError::InvalidPrice);
        require!(
            duration_hours >= MIN_DURATION_HOURS && duration_hours <= MAX_DURATION_HOURS,
            MarketError::InvalidDuration
        );

        // Calculate deposit fee.
        let deposit_amount = (buyout_price as u128)
            .checked_mul(config.listing_fee_bps as u128)
            .unwrap()
            .checked_div(10_000)
            .unwrap() as u64;

        // Transfer deposit SOL from seller to NftListing PDA.
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.seller.to_account_info(),
                    to: ctx.accounts.nft_listing.to_account_info(),
                },
            ),
            deposit_amount,
        )?;

        // Transfer Metaplex Core asset from seller to NftListing PDA.
        TransferV1CpiBuilder::new(&ctx.accounts.mpl_core_program)
            .asset(&ctx.accounts.asset)
            .collection(Some(&ctx.accounts.collection.to_account_info()))
            .payer(&ctx.accounts.seller)
            .authority(Some(&ctx.accounts.seller.to_account_info()))
            .new_owner(&ctx.accounts.nft_listing.to_account_info())
            .system_program(Some(&ctx.accounts.system_program.to_account_info()))
            .invoke()?;

        let clock = Clock::get()?;
        let listing_id = ctx.accounts.nft_config.listing_count;

        let nft_listing = &mut ctx.accounts.nft_listing;
        nft_listing.listing_id = listing_id;
        nft_listing.seller = ctx.accounts.seller.key();
        nft_listing.asset = ctx.accounts.asset.key();
        nft_listing.buyout_price = buyout_price;
        nft_listing.current_bid = 0;
        nft_listing.current_bidder = Pubkey::default();
        nft_listing.deposit_amount = deposit_amount;
        nft_listing.created_at = clock.unix_timestamp;
        nft_listing.expires_at = clock
            .unix_timestamp
            .checked_add(
                (duration_hours as i64)
                    .checked_mul(SECONDS_PER_HOUR)
                    .unwrap(),
            )
            .unwrap();
        nft_listing.state = ListingState::Active;
        nft_listing.item_id = item_id;
        nft_listing.bump = ctx.bumps.nft_listing;

        // Increment NFT listing counter.
        let nft_config = &mut ctx.accounts.nft_config;
        nft_config.listing_count = nft_config.listing_count.checked_add(1).unwrap();

        emit!(NftListingCreated {
            listing_id,
            seller: ctx.accounts.seller.key(),
            asset: ctx.accounts.asset.key(),
            item_id,
            buyout_price,
        });

        Ok(())
    }

    /// Player places a bid on an NFT listing.
    pub fn place_nft_bid(
        ctx: Context<PlaceNftBid>,
        listing_id: u64,
        bid_amount: u64,
    ) -> Result<()> {
        let config = &ctx.accounts.market_config;
        require!(!config.paused, MarketError::MarketPaused);

        let listing = &mut ctx.accounts.nft_listing;
        require!(
            listing.state == ListingState::Active,
            MarketError::ListingNotActive
        );

        let clock = Clock::get()?;
        require!(
            clock.unix_timestamp < listing.expires_at,
            MarketError::ListingExpiredError
        );
        require!(bid_amount > listing.current_bid, MarketError::BidTooLow);
        require!(
            bid_amount < listing.buyout_price,
            MarketError::BidExceedsBuyout
        );

        // Refund previous bidder if any.
        if listing.current_bidder != Pubkey::default() {
            let prev_bid = listing.current_bid;
            let listing_info = listing.to_account_info();
            let prev_bidder_info = ctx.accounts.previous_bidder.to_account_info();

            **listing_info.try_borrow_mut_lamports()? = listing_info
                .lamports()
                .checked_sub(prev_bid)
                .unwrap();
            **prev_bidder_info.try_borrow_mut_lamports()? = prev_bidder_info
                .lamports()
                .checked_add(prev_bid)
                .unwrap();
        }

        // Transfer bid SOL from bidder to NftListing PDA.
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.bidder.to_account_info(),
                    to: listing.to_account_info(),
                },
            ),
            bid_amount,
        )?;

        listing.current_bid = bid_amount;
        listing.current_bidder = ctx.accounts.bidder.key();

        emit!(NftBidPlaced {
            listing_id,
            bidder: ctx.accounts.bidder.key(),
            amount: bid_amount,
        });

        Ok(())
    }

    /// Player buys an NFT at buyout price.
    pub fn buyout_nft(ctx: Context<BuyoutNft>, listing_id: u64) -> Result<()> {
        let config = &ctx.accounts.market_config;
        require!(!config.paused, MarketError::MarketPaused);

        let listing = &mut ctx.accounts.nft_listing;
        require!(
            listing.state == ListingState::Active,
            MarketError::ListingNotActive
        );

        let clock = Clock::get()?;
        require!(
            clock.unix_timestamp < listing.expires_at,
            MarketError::ListingExpiredError
        );

        let buyout_price = listing.buyout_price;
        let deposit = listing.deposit_amount;
        let lid_bytes = listing.listing_id.to_le_bytes();
        let listing_bump = listing.bump;
        let listing_seeds: &[&[&[u8]]] = &[&[NFT_LISTING_SEED, &lid_bytes, &[listing_bump]]];

        // ── ALL CPIs FIRST (before any manual lamport manipulation) ──

        // 1. Transfer buyout price SOL from buyer to NftListing PDA.
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.buyer.to_account_info(),
                    to: listing.to_account_info(),
                },
            ),
            buyout_price,
        )?;

        // 2. Transfer NFT from NftListing PDA to buyer wallet.
        TransferV1CpiBuilder::new(&ctx.accounts.mpl_core_program)
            .asset(&ctx.accounts.asset)
            .collection(Some(&ctx.accounts.collection.to_account_info()))
            .payer(&ctx.accounts.buyer)
            .authority(Some(&listing.to_account_info()))
            .new_owner(&ctx.accounts.buyer.to_account_info())
            .system_program(Some(&ctx.accounts.system_program.to_account_info()))
            .invoke_signed(listing_seeds)?;

        // ── NOW manual lamport manipulation (after all CPIs) ──

        listing.state = ListingState::Sold;

        // 3. Refund previous bidder if any.
        if listing.current_bidder != Pubkey::default() {
            let prev_bid = listing.current_bid;
            let listing_info = listing.to_account_info();
            let prev_bidder_info = ctx.accounts.previous_bidder.to_account_info();

            **listing_info.try_borrow_mut_lamports()? = listing_info
                .lamports()
                .checked_sub(prev_bid)
                .unwrap();
            **prev_bidder_info.try_borrow_mut_lamports()? = prev_bidder_info
                .lamports()
                .checked_add(prev_bid)
                .unwrap();
        }

        // 4. Calculate and transfer fee to treasury.
        let fee = (buyout_price as u128)
            .checked_mul(config.sale_fee_bps as u128)
            .unwrap()
            .checked_div(10_000)
            .unwrap() as u64;
        let seller_proceeds = buyout_price
            .checked_sub(fee)
            .unwrap()
            .checked_add(deposit)
            .unwrap();

        {
            let listing_info = listing.to_account_info();
            let treasury_info = ctx.accounts.treasury.to_account_info();

            **listing_info.try_borrow_mut_lamports()? = listing_info
                .lamports()
                .checked_sub(fee)
                .unwrap();
            **treasury_info.try_borrow_mut_lamports()? = treasury_info
                .lamports()
                .checked_add(fee)
                .unwrap();
        }

        // 5. Transfer proceeds + deposit to seller.
        {
            let listing_info = listing.to_account_info();
            let seller_info = ctx.accounts.seller.to_account_info();

            **listing_info.try_borrow_mut_lamports()? = listing_info
                .lamports()
                .checked_sub(seller_proceeds)
                .unwrap();
            **seller_info.try_borrow_mut_lamports()? = seller_info
                .lamports()
                .checked_add(seller_proceeds)
                .unwrap();
        }

        // 6. Close listing PDA, sending remaining lamports (rent) to seller.
        {
            let listing_info = listing.to_account_info();
            let seller_info = ctx.accounts.seller.to_account_info();
            let remaining = listing_info.lamports();

            **listing_info.try_borrow_mut_lamports()? = 0;
            **seller_info.try_borrow_mut_lamports()? = seller_info
                .lamports()
                .checked_add(remaining)
                .unwrap();
        }

        emit!(NftListingSold {
            listing_id,
            buyer: ctx.accounts.buyer.key(),
            price: buyout_price,
            fee,
        });

        Ok(())
    }

    /// Player (seller) cancels their NFT listing. Must have no bids.
    pub fn cancel_nft_listing(
        ctx: Context<CancelNftListing>,
        listing_id: u64,
    ) -> Result<()> {
        let listing = &mut ctx.accounts.nft_listing;
        require!(
            listing.state == ListingState::Active,
            MarketError::ListingNotActive
        );
        require!(
            listing.seller == ctx.accounts.seller.key(),
            MarketError::NotSeller
        );
        require!(
            listing.current_bidder == Pubkey::default(),
            MarketError::HasBids
        );

        listing.state = ListingState::Cancelled;

        let lid_bytes = listing.listing_id.to_le_bytes();
        let listing_bump = listing.bump;
        let listing_seeds: &[&[&[u8]]] = &[&[NFT_LISTING_SEED, &lid_bytes, &[listing_bump]]];

        // Transfer NFT from NftListing PDA back to seller wallet.
        TransferV1CpiBuilder::new(&ctx.accounts.mpl_core_program)
            .asset(&ctx.accounts.asset)
            .collection(Some(&ctx.accounts.collection.to_account_info()))
            .payer(&ctx.accounts.seller)
            .authority(Some(&listing.to_account_info()))
            .new_owner(&ctx.accounts.seller.to_account_info())
            .system_program(Some(&ctx.accounts.system_program.to_account_info()))
            .invoke_signed(listing_seeds)?;

        // Return deposit + close listing PDA (remaining rent) to seller.
        {
            let listing_info = listing.to_account_info();
            let seller_info = ctx.accounts.seller.to_account_info();
            let remaining = listing_info.lamports();

            **listing_info.try_borrow_mut_lamports()? = 0;
            **seller_info.try_borrow_mut_lamports()? = seller_info
                .lamports()
                .checked_add(remaining)
                .unwrap();
        }

        emit!(NftListingCancelled { listing_id });

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Account Structs
// ---------------------------------------------------------------------------

#[account]
#[derive(InitSpace)]
pub struct MarketConfig {
    /// Game server operator who can register items and mint tokens.
    pub authority: Pubkey,
    /// Treasury wallet that receives marketplace fees.
    pub treasury: Pubkey,
    /// Listing deposit fee in basis points (e.g. 500 = 5%).
    pub listing_fee_bps: u16,
    /// Sale fee (house cut) in basis points.
    pub sale_fee_bps: u16,
    /// Auto-incrementing listing counter.
    pub listing_count: u64,
    /// Emergency pause flag.
    pub paused: bool,
    /// PDA bump seed.
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct RegisteredItem {
    /// Maps to the game's ItemInfo.Id.
    pub item_id: u32,
    /// The SPL token mint address for this item.
    pub mint: Pubkey,
    /// Whether this item can be traded on the marketplace.
    pub is_tradeable: bool,
    /// SHA-256 hash of the item name for verification.
    pub name_hash: [u8; 32],
    /// PDA bump seed.
    pub bump: u8,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace)]
#[repr(u8)]
pub enum ListingState {
    Active = 0,
    Sold = 1,
    Cancelled = 2,
    Expired = 3,
}

#[account]
#[derive(InitSpace)]
pub struct Listing {
    /// Unique listing identifier.
    pub listing_id: u64,
    /// The seller's wallet.
    pub seller: Pubkey,
    /// The SPL token mint of the listed item.
    pub item_mint: Pubkey,
    /// Number of item tokens listed.
    pub item_amount: u64,
    /// Instant-buy price in lamports.
    pub buyout_price: u64,
    /// Current highest bid in lamports.
    pub current_bid: u64,
    /// Current highest bidder (Pubkey::default() if none).
    pub current_bidder: Pubkey,
    /// Deposit held by this listing PDA.
    pub deposit_amount: u64,
    /// Unix timestamp when listing was created.
    pub created_at: i64,
    /// Unix timestamp when listing expires.
    pub expires_at: i64,
    /// Current state of the listing.
    pub state: ListingState,
    /// PDA bump seed.
    pub bump: u8,
}

/// Separate config for NFT listings (avoids resizing existing MarketConfig).
#[account]
#[derive(InitSpace)]
pub struct NftConfig {
    /// Auto-incrementing NFT listing counter.
    pub listing_count: u64,
    /// PDA bump seed.
    pub bump: u8,
}

/// An NFT listing on the Dark Auction House.
/// Holds a Metaplex Core asset in escrow (PDA is the owner).
#[account]
#[derive(InitSpace)]
pub struct NftListing {
    /// Unique NFT listing identifier.
    pub listing_id: u64,
    /// The seller's wallet address.
    pub seller: Pubkey,
    /// The Metaplex Core asset address held in escrow.
    pub asset: Pubkey,
    /// Instant-buy price in lamports.
    pub buyout_price: u64,
    /// Current highest bid in lamports.
    pub current_bid: u64,
    /// Current highest bidder (Pubkey::default() if none).
    pub current_bidder: Pubkey,
    /// Deposit held by this listing PDA.
    pub deposit_amount: u64,
    /// Unix timestamp when listing was created.
    pub created_at: i64,
    /// Unix timestamp when listing expires.
    pub expires_at: i64,
    /// Current state of the listing.
    pub state: ListingState,
    /// Game item ID (for metadata/display).
    pub item_id: u32,
    /// PDA bump seed.
    pub bump: u8,
}

/// Player escrow account for holding SOL deposits used in marketplace bidding/buying.
#[account]
#[derive(InitSpace)]
pub struct PlayerEscrow {
    /// The player wallet this escrow belongs to.
    pub player: Pubkey,     // 32 bytes
    /// Current SOL balance held in escrow.
    pub balance: u64,       // 8 bytes
    /// PDA bump seed.
    pub bump: u8,           // 1 byte
}

// ---------------------------------------------------------------------------
// Instruction Account Contexts
// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + MarketConfig::INIT_SPACE,
        seeds = [MARKET_CONFIG_SEED],
        bump,
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(item_id: u32)]
pub struct RegisterItem<'info> {
    #[account(
        mut,
        seeds = [MARKET_CONFIG_SEED],
        bump = market_config.bump,
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        init,
        payer = authority,
        space = 8 + RegisteredItem::INIT_SPACE,
        seeds = [REGISTERED_ITEM_SEED, &item_id.to_le_bytes()],
        bump,
    )]
    pub registered_item: Account<'info, RegisteredItem>,

    #[account(
        init,
        payer = authority,
        seeds = [ITEM_MINT_SEED, &item_id.to_le_bytes()],
        bump,
        mint::decimals = 0,
        mint::authority = market_config,
    )]
    pub item_mint: Account<'info, Mint>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
#[instruction(item_id: u32)]
pub struct MintItem<'info> {
    #[account(
        seeds = [MARKET_CONFIG_SEED],
        bump = market_config.bump,
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        mut,
        seeds = [ITEM_MINT_SEED, &item_id.to_le_bytes()],
        bump,
        mint::authority = market_config,
    )]
    pub item_mint: Account<'info, Mint>,

    /// CHECK: The recipient wallet; does not need to sign.
    pub recipient: UncheckedAccount<'info>,

    #[account(
        init_if_needed,
        payer = authority,
        associated_token::mint = item_mint,
        associated_token::authority = recipient,
    )]
    pub recipient_ata: Account<'info, TokenAccount>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(item_id: u32)]
pub struct BurnItem<'info> {
    #[account(
        mut,
        seeds = [ITEM_MINT_SEED, &item_id.to_le_bytes()],
        bump,
    )]
    pub item_mint: Account<'info, Mint>,

    #[account(
        mut,
        associated_token::mint = item_mint,
        associated_token::authority = owner,
    )]
    pub owner_ata: Account<'info, TokenAccount>,

    #[account(mut)]
    pub owner: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
#[instruction(buyout_price: u64, duration_hours: u16)]
pub struct CreateListing<'info> {
    #[account(
        mut,
        seeds = [MARKET_CONFIG_SEED],
        bump = market_config.bump,
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        seeds = [REGISTERED_ITEM_SEED, &registered_item.item_id.to_le_bytes()],
        bump = registered_item.bump,
        constraint = registered_item.mint == item_mint.key(),
    )]
    pub registered_item: Account<'info, RegisteredItem>,

    pub item_mint: Account<'info, Mint>,

    #[account(
        init,
        payer = seller,
        space = 8 + Listing::INIT_SPACE,
        seeds = [LISTING_SEED, &market_config.listing_count.to_le_bytes()],
        bump,
    )]
    pub listing: Account<'info, Listing>,

    #[account(
        mut,
        associated_token::mint = item_mint,
        associated_token::authority = seller,
    )]
    pub seller_ata: Account<'info, TokenAccount>,

    #[account(
        init,
        payer = seller,
        associated_token::mint = item_mint,
        associated_token::authority = listing,
    )]
    pub escrow_ata: Account<'info, TokenAccount>,

    #[account(mut)]
    pub seller: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(listing_id: u64)]
pub struct PlaceBid<'info> {
    #[account(
        seeds = [MARKET_CONFIG_SEED],
        bump = market_config.bump,
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        mut,
        seeds = [LISTING_SEED, &listing_id.to_le_bytes()],
        bump = listing.bump,
    )]
    pub listing: Account<'info, Listing>,

    #[account(mut)]
    pub bidder: Signer<'info>,

    /// CHECK: Previous bidder to refund. Must match listing.current_bidder
    /// (or be any account if there is no previous bidder).
    #[account(
        mut,
        constraint = listing.current_bidder == Pubkey::default()
            || previous_bidder.key() == listing.current_bidder
    )]
    pub previous_bidder: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(listing_id: u64)]
pub struct Buyout<'info> {
    #[account(
        seeds = [MARKET_CONFIG_SEED],
        bump = market_config.bump,
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        mut,
        seeds = [LISTING_SEED, &listing_id.to_le_bytes()],
        bump = listing.bump,
    )]
    pub listing: Account<'info, Listing>,

    #[account(mut)]
    pub buyer: Signer<'info>,

    /// CHECK: The seller who receives proceeds. Must match listing.seller.
    #[account(
        mut,
        constraint = seller.key() == listing.seller,
    )]
    pub seller: UncheckedAccount<'info>,

    /// CHECK: Treasury that receives the fee. Must match market_config.treasury.
    #[account(
        mut,
        constraint = treasury.key() == market_config.treasury,
    )]
    pub treasury: UncheckedAccount<'info>,

    /// CHECK: Previous bidder to refund. Must match listing.current_bidder
    /// (or be any account if there is no previous bidder).
    #[account(
        mut,
        constraint = listing.current_bidder == Pubkey::default()
            || previous_bidder.key() == listing.current_bidder
    )]
    pub previous_bidder: UncheckedAccount<'info>,

    #[account(
        mut,
        associated_token::mint = item_mint,
        associated_token::authority = listing,
    )]
    pub escrow_ata: Account<'info, TokenAccount>,

    #[account(
        init_if_needed,
        payer = buyer,
        associated_token::mint = item_mint,
        associated_token::authority = buyer,
    )]
    pub buyer_ata: Account<'info, TokenAccount>,

    pub item_mint: Account<'info, Mint>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(listing_id: u64)]
pub struct CancelListing<'info> {
    #[account(
        mut,
        seeds = [LISTING_SEED, &listing_id.to_le_bytes()],
        bump = listing.bump,
    )]
    pub listing: Account<'info, Listing>,

    #[account(
        mut,
        associated_token::mint = item_mint,
        associated_token::authority = listing,
    )]
    pub escrow_ata: Account<'info, TokenAccount>,

    #[account(
        mut,
        associated_token::mint = item_mint,
        associated_token::authority = seller,
    )]
    pub seller_ata: Account<'info, TokenAccount>,

    pub item_mint: Account<'info, Mint>,

    #[account(mut)]
    pub seller: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(listing_id: u64)]
pub struct ClaimExpired<'info> {
    #[account(
        seeds = [MARKET_CONFIG_SEED],
        bump = market_config.bump,
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        mut,
        seeds = [LISTING_SEED, &listing_id.to_le_bytes()],
        bump = listing.bump,
    )]
    pub listing: Account<'info, Listing>,

    /// CHECK: The seller who listed the item. Must match listing.seller.
    #[account(
        mut,
        constraint = seller.key() == listing.seller,
    )]
    pub seller: UncheckedAccount<'info>,

    /// CHECK: Treasury for fees. Must match market_config.treasury.
    #[account(
        mut,
        constraint = treasury.key() == market_config.treasury,
    )]
    pub treasury: UncheckedAccount<'info>,

    #[account(
        mut,
        associated_token::mint = item_mint,
        associated_token::authority = listing,
    )]
    pub escrow_ata: Account<'info, TokenAccount>,

    /// The destination ATA for items: bidder's ATA if had_bids, seller's ATA if no bids.
    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = item_mint,
        associated_token::authority = item_recipient,
    )]
    pub bidder_or_seller_ata: Account<'info, TokenAccount>,

    /// CHECK: The wallet that should receive the items. If there was a bidder,
    /// this is the bidder; if no bids, this is the seller.
    #[account(
        constraint = (listing.current_bidder == Pubkey::default()
            && item_recipient.key() == listing.seller)
            || (listing.current_bidder != Pubkey::default()
                && item_recipient.key() == listing.current_bidder)
    )]
    pub item_recipient: UncheckedAccount<'info>,

    pub item_mint: Account<'info, Mint>,

    #[account(mut)]
    pub payer: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

// ---------------------------------------------------------------------------
// Authority-operated instruction account structs
// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct CreateListingOperated<'info> {
    #[account(
        mut,
        seeds = [MARKET_CONFIG_SEED],
        bump = market_config.bump,
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        seeds = [REGISTERED_ITEM_SEED, &registered_item.item_id.to_le_bytes()],
        bump = registered_item.bump,
        constraint = registered_item.mint == item_mint.key(),
    )]
    pub registered_item: Account<'info, RegisteredItem>,

    #[account(mut)]
    pub item_mint: Account<'info, Mint>,

    #[account(
        init,
        payer = authority,
        space = 8 + Listing::INIT_SPACE,
        seeds = [LISTING_SEED, &market_config.listing_count.to_le_bytes()],
        bump,
    )]
    pub listing: Account<'info, Listing>,

    #[account(
        init,
        payer = authority,
        associated_token::mint = item_mint,
        associated_token::authority = listing,
    )]
    pub escrow_ata: Account<'info, TokenAccount>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(listing_id: u64)]
pub struct PlaceBidOperated<'info> {
    #[account(
        seeds = [MARKET_CONFIG_SEED],
        bump = market_config.bump,
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        mut,
        seeds = [LISTING_SEED, &listing_id.to_le_bytes()],
        bump = listing.bump,
    )]
    pub listing: Account<'info, Listing>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(listing_id: u64)]
pub struct BuyoutOperated<'info> {
    #[account(
        seeds = [MARKET_CONFIG_SEED],
        bump = market_config.bump,
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        mut,
        seeds = [LISTING_SEED, &listing_id.to_le_bytes()],
        bump = listing.bump,
    )]
    pub listing: Account<'info, Listing>,

    #[account(mut)]
    pub authority: Signer<'info>,

    /// CHECK: Treasury for fees. Validated against market_config.treasury.
    #[account(
        mut,
        constraint = treasury.key() == market_config.treasury,
    )]
    pub treasury: UncheckedAccount<'info>,

    #[account(
        mut,
        associated_token::mint = item_mint,
        associated_token::authority = listing,
    )]
    pub escrow_ata: Account<'info, TokenAccount>,

    #[account(mut)]
    pub item_mint: Account<'info, Mint>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(listing_id: u64)]
pub struct CancelListingOperated<'info> {
    #[account(
        seeds = [MARKET_CONFIG_SEED],
        bump = market_config.bump,
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        mut,
        seeds = [LISTING_SEED, &listing_id.to_le_bytes()],
        bump = listing.bump,
    )]
    pub listing: Account<'info, Listing>,

    #[account(
        mut,
        associated_token::mint = item_mint,
        associated_token::authority = listing,
    )]
    pub escrow_ata: Account<'info, TokenAccount>,

    #[account(mut)]
    pub item_mint: Account<'info, Mint>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

// ---------------------------------------------------------------------------
// NFT Instruction Account Contexts
// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct InitializeNftConfig<'info> {
    #[account(
        seeds = [MARKET_CONFIG_SEED],
        bump = market_config.bump,
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        init,
        payer = authority,
        space = 8 + NftConfig::INIT_SPACE,
        seeds = [NFT_CONFIG_SEED],
        bump,
    )]
    pub nft_config: Account<'info, NftConfig>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(buyout_price: u64, duration_hours: u16, seller: Pubkey, item_id: u32)]
pub struct CreateNftListingOperated<'info> {
    #[account(
        seeds = [MARKET_CONFIG_SEED],
        bump = market_config.bump,
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        mut,
        seeds = [NFT_CONFIG_SEED],
        bump = nft_config.bump,
    )]
    pub nft_config: Account<'info, NftConfig>,

    #[account(
        init,
        payer = authority,
        space = 8 + NftListing::INIT_SPACE,
        seeds = [NFT_LISTING_SEED, &nft_config.listing_count.to_le_bytes()],
        bump,
    )]
    pub nft_listing: Account<'info, NftListing>,

    /// CHECK: The Metaplex Core asset to be listed. Verified via mpl-core CPI.
    #[account(mut)]
    pub asset: UncheckedAccount<'info>,

    #[account(mut)]
    pub authority: Signer<'info>,

    /// CHECK: The Metaplex Core collection. Required for collection asset transfers.
    #[account(mut)]
    pub collection: UncheckedAccount<'info>,

    /// CHECK: Metaplex Core program.
    pub mpl_core_program: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(listing_id: u64)]
pub struct PlaceNftBidOperated<'info> {
    #[account(
        seeds = [MARKET_CONFIG_SEED],
        bump = market_config.bump,
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        mut,
        seeds = [NFT_LISTING_SEED, &listing_id.to_le_bytes()],
        bump = nft_listing.bump,
    )]
    pub nft_listing: Account<'info, NftListing>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(listing_id: u64)]
pub struct BuyoutNftOperated<'info> {
    #[account(
        seeds = [MARKET_CONFIG_SEED],
        bump = market_config.bump,
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        mut,
        seeds = [NFT_LISTING_SEED, &listing_id.to_le_bytes()],
        bump = nft_listing.bump,
    )]
    pub nft_listing: Account<'info, NftListing>,

    #[account(mut)]
    pub authority: Signer<'info>,

    /// CHECK: Treasury for fees. Validated against market_config.treasury.
    #[account(
        mut,
        constraint = treasury.key() == market_config.treasury,
    )]
    pub treasury: UncheckedAccount<'info>,

    /// CHECK: The buyer's wallet that will receive the NFT.
    pub buyer_wallet: UncheckedAccount<'info>,

    /// CHECK: The Metaplex Core asset. Must match nft_listing.asset.
    #[account(
        mut,
        constraint = asset.key() == nft_listing.asset,
    )]
    pub asset: UncheckedAccount<'info>,

    /// CHECK: The Metaplex Core collection. Required for collection asset transfers.
    #[account(mut)]
    pub collection: UncheckedAccount<'info>,

    /// CHECK: Metaplex Core program.
    pub mpl_core_program: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(listing_id: u64)]
pub struct CancelNftListingOperated<'info> {
    #[account(
        seeds = [MARKET_CONFIG_SEED],
        bump = market_config.bump,
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        mut,
        seeds = [NFT_LISTING_SEED, &listing_id.to_le_bytes()],
        bump = nft_listing.bump,
    )]
    pub nft_listing: Account<'info, NftListing>,

    #[account(mut)]
    pub authority: Signer<'info>,

    /// CHECK: The seller's wallet to receive NFT back. Must match nft_listing.seller.
    #[account(
        constraint = seller_wallet.key() == nft_listing.seller,
    )]
    pub seller_wallet: UncheckedAccount<'info>,

    /// CHECK: The Metaplex Core asset. Must match nft_listing.asset.
    #[account(
        mut,
        constraint = asset.key() == nft_listing.asset,
    )]
    pub asset: UncheckedAccount<'info>,

    /// CHECK: The Metaplex Core collection. Required for collection asset transfers.
    #[account(mut)]
    pub collection: UncheckedAccount<'info>,

    /// CHECK: Metaplex Core program.
    pub mpl_core_program: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(listing_id: u64)]
pub struct ClaimNftExpired<'info> {
    #[account(
        seeds = [MARKET_CONFIG_SEED],
        bump = market_config.bump,
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        mut,
        seeds = [NFT_LISTING_SEED, &listing_id.to_le_bytes()],
        bump = nft_listing.bump,
    )]
    pub nft_listing: Account<'info, NftListing>,

    #[account(mut)]
    pub authority: Signer<'info>,

    /// CHECK: Treasury for fees. Validated against market_config.treasury.
    #[account(
        mut,
        constraint = treasury.key() == market_config.treasury,
    )]
    pub treasury: UncheckedAccount<'info>,

    /// CHECK: NFT recipient. If bids, must be current_bidder. If no bids, must be seller.
    #[account(
        constraint = (nft_listing.current_bidder == Pubkey::default()
            && nft_recipient.key() == nft_listing.seller)
            || (nft_listing.current_bidder != Pubkey::default()
                && nft_recipient.key() == nft_listing.current_bidder)
    )]
    pub nft_recipient: UncheckedAccount<'info>,

    /// CHECK: The Metaplex Core asset. Must match nft_listing.asset.
    #[account(
        mut,
        constraint = asset.key() == nft_listing.asset,
    )]
    pub asset: UncheckedAccount<'info>,

    /// CHECK: The Metaplex Core collection. Required for collection asset transfers.
    #[account(mut)]
    pub collection: UncheckedAccount<'info>,

    /// CHECK: Metaplex Core program.
    pub mpl_core_program: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

// ---------------------------------------------------------------------------
// Escrow Instruction Account Contexts
// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct DepositEscrow<'info> {
    #[account(
        init_if_needed,
        payer = player,
        space = 8 + PlayerEscrow::INIT_SPACE,
        seeds = [PLAYER_ESCROW_SEED, player.key().as_ref()],
        bump,
    )]
    pub player_escrow: Account<'info, PlayerEscrow>,

    #[account(mut)]
    pub player: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawEscrow<'info> {
    #[account(
        mut,
        seeds = [PLAYER_ESCROW_SEED, player.key().as_ref()],
        bump = player_escrow.bump,
        has_one = player,
    )]
    pub player_escrow: Account<'info, PlayerEscrow>,

    #[account(mut)]
    pub player: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(amount: u64, player: Pubkey)]
pub struct DeductEscrowOperated<'info> {
    #[account(
        seeds = [MARKET_CONFIG_SEED],
        bump = market_config.bump,
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        mut,
        seeds = [PLAYER_ESCROW_SEED, player.as_ref()],
        bump = player_escrow.bump,
    )]
    pub player_escrow: Account<'info, PlayerEscrow>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(amount: u64, player: Pubkey)]
pub struct CreditEscrowOperated<'info> {
    #[account(
        seeds = [MARKET_CONFIG_SEED],
        bump = market_config.bump,
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        mut,
        seeds = [PLAYER_ESCROW_SEED, player.as_ref()],
        bump = player_escrow.bump,
    )]
    pub player_escrow: Account<'info, PlayerEscrow>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(listing_id: u64, bid_amount: u64, bidder: Pubkey)]
pub struct PlaceNftBidEscrow<'info> {
    #[account(
        seeds = [MARKET_CONFIG_SEED],
        bump = market_config.bump,
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        mut,
        seeds = [NFT_LISTING_SEED, &listing_id.to_le_bytes()],
        bump = nft_listing.bump,
    )]
    pub nft_listing: Account<'info, NftListing>,

    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [PLAYER_ESCROW_SEED, bidder.as_ref()],
        bump = bidder_escrow.bump,
    )]
    pub bidder_escrow: Account<'info, PlayerEscrow>,

    /// Previous bidder's escrow for refund. When there is no previous bidder,
    /// pass the bidder_escrow account again (it won't be modified in that case).
    /// When there IS a previous bidder, this must be their escrow PDA.
    #[account(mut)]
    pub prev_bidder_escrow: Account<'info, PlayerEscrow>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(listing_id: u64, buyer: Pubkey)]
pub struct BuyoutNftEscrow<'info> {
    #[account(
        seeds = [MARKET_CONFIG_SEED],
        bump = market_config.bump,
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        mut,
        seeds = [NFT_LISTING_SEED, &listing_id.to_le_bytes()],
        bump = nft_listing.bump,
    )]
    pub nft_listing: Account<'info, NftListing>,

    #[account(mut)]
    pub authority: Signer<'info>,

    /// CHECK: Treasury for fees. Validated against market_config.treasury.
    #[account(
        mut,
        constraint = treasury.key() == market_config.treasury,
    )]
    pub treasury: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [PLAYER_ESCROW_SEED, buyer.as_ref()],
        bump = buyer_escrow.bump,
    )]
    pub buyer_escrow: Account<'info, PlayerEscrow>,

    /// CHECK: The buyer's wallet that will receive the NFT.
    pub buyer_wallet: UncheckedAccount<'info>,

    /// CHECK: The Metaplex Core asset. Must match nft_listing.asset.
    #[account(
        mut,
        constraint = asset.key() == nft_listing.asset,
    )]
    pub asset: UncheckedAccount<'info>,

    /// CHECK: The Metaplex Core collection. Required for collection asset transfers.
    #[account(mut)]
    pub collection: UncheckedAccount<'info>,

    /// CHECK: Metaplex Core program.
    pub mpl_core_program: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

// ---------------------------------------------------------------------------
// Non-Operated NFT Instruction Account Contexts
// ---------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(buyout_price: u64, duration_hours: u16, item_id: u32)]
pub struct CreateNftListing<'info> {
    #[account(
        seeds = [MARKET_CONFIG_SEED],
        bump = market_config.bump,
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        mut,
        seeds = [NFT_CONFIG_SEED],
        bump = nft_config.bump,
    )]
    pub nft_config: Account<'info, NftConfig>,

    #[account(
        init,
        payer = seller,
        space = 8 + NftListing::INIT_SPACE,
        seeds = [NFT_LISTING_SEED, &nft_config.listing_count.to_le_bytes()],
        bump,
    )]
    pub nft_listing: Account<'info, NftListing>,

    /// CHECK: The Metaplex Core asset to be listed. Verified via mpl-core CPI.
    #[account(mut)]
    pub asset: UncheckedAccount<'info>,

    #[account(mut)]
    pub seller: Signer<'info>,

    /// CHECK: The Metaplex Core collection. Required for collection asset transfers.
    #[account(mut)]
    pub collection: UncheckedAccount<'info>,

    /// CHECK: Metaplex Core program.
    pub mpl_core_program: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(listing_id: u64)]
pub struct PlaceNftBid<'info> {
    #[account(
        seeds = [MARKET_CONFIG_SEED],
        bump = market_config.bump,
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        mut,
        seeds = [NFT_LISTING_SEED, &listing_id.to_le_bytes()],
        bump = nft_listing.bump,
    )]
    pub nft_listing: Account<'info, NftListing>,

    #[account(mut)]
    pub bidder: Signer<'info>,

    /// CHECK: Previous bidder to refund. Must match nft_listing.current_bidder
    /// (or be any account if there is no previous bidder).
    #[account(
        mut,
        constraint = nft_listing.current_bidder == Pubkey::default()
            || previous_bidder.key() == nft_listing.current_bidder
    )]
    pub previous_bidder: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(listing_id: u64)]
pub struct BuyoutNft<'info> {
    #[account(
        seeds = [MARKET_CONFIG_SEED],
        bump = market_config.bump,
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        mut,
        seeds = [NFT_LISTING_SEED, &listing_id.to_le_bytes()],
        bump = nft_listing.bump,
    )]
    pub nft_listing: Account<'info, NftListing>,

    #[account(mut)]
    pub buyer: Signer<'info>,

    /// CHECK: The seller who receives proceeds. Must match nft_listing.seller.
    #[account(
        mut,
        constraint = seller.key() == nft_listing.seller,
    )]
    pub seller: UncheckedAccount<'info>,

    /// CHECK: Treasury that receives the fee. Must match market_config.treasury.
    #[account(
        mut,
        constraint = treasury.key() == market_config.treasury,
    )]
    pub treasury: UncheckedAccount<'info>,

    /// CHECK: Previous bidder to refund. Must match nft_listing.current_bidder
    /// (or be any account if there is no previous bidder).
    #[account(
        mut,
        constraint = nft_listing.current_bidder == Pubkey::default()
            || previous_bidder.key() == nft_listing.current_bidder
    )]
    pub previous_bidder: UncheckedAccount<'info>,

    /// CHECK: The Metaplex Core asset. Must match nft_listing.asset.
    #[account(
        mut,
        constraint = asset.key() == nft_listing.asset,
    )]
    pub asset: UncheckedAccount<'info>,

    /// CHECK: The Metaplex Core collection. Required for collection asset transfers.
    #[account(mut)]
    pub collection: UncheckedAccount<'info>,

    /// CHECK: Metaplex Core program.
    pub mpl_core_program: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(listing_id: u64)]
pub struct CancelNftListing<'info> {
    #[account(
        mut,
        seeds = [NFT_LISTING_SEED, &listing_id.to_le_bytes()],
        bump = nft_listing.bump,
    )]
    pub nft_listing: Account<'info, NftListing>,

    #[account(mut)]
    pub seller: Signer<'info>,

    /// CHECK: The Metaplex Core asset. Must match nft_listing.asset.
    #[account(
        mut,
        constraint = asset.key() == nft_listing.asset,
    )]
    pub asset: UncheckedAccount<'info>,

    /// CHECK: The Metaplex Core collection. Required for collection asset transfers.
    #[account(mut)]
    pub collection: UncheckedAccount<'info>,

    /// CHECK: Metaplex Core program.
    pub mpl_core_program: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[event]
pub struct ItemRegistered {
    pub item_id: u32,
    pub mint: Pubkey,
}

#[event]
pub struct ItemMinted {
    pub item_id: u32,
    pub recipient: Pubkey,
    pub amount: u64,
}

#[event]
pub struct ItemBurned {
    pub item_id: u32,
    pub owner: Pubkey,
    pub amount: u64,
}

#[event]
pub struct ListingCreated {
    pub listing_id: u64,
    pub seller: Pubkey,
    pub item_mint: Pubkey,
    pub amount: u64,
    pub buyout_price: u64,
}

#[event]
pub struct BidPlaced {
    pub listing_id: u64,
    pub bidder: Pubkey,
    pub amount: u64,
}

#[event]
pub struct ListingSold {
    pub listing_id: u64,
    pub buyer: Pubkey,
    pub price: u64,
    pub fee: u64,
}

#[event]
pub struct ListingCancelled {
    pub listing_id: u64,
}

#[event]
pub struct ListingExpired {
    pub listing_id: u64,
    pub had_bids: bool,
}

#[event]
pub struct NftListingCreated {
    pub listing_id: u64,
    pub seller: Pubkey,
    pub asset: Pubkey,
    pub item_id: u32,
    pub buyout_price: u64,
}

#[event]
pub struct NftBidPlaced {
    pub listing_id: u64,
    pub bidder: Pubkey,
    pub amount: u64,
}

#[event]
pub struct NftListingSold {
    pub listing_id: u64,
    pub buyer: Pubkey,
    pub price: u64,
    pub fee: u64,
}

#[event]
pub struct NftListingCancelled {
    pub listing_id: u64,
}

#[event]
pub struct NftListingExpired {
    pub listing_id: u64,
    pub had_bids: bool,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[error_code]
pub enum MarketError {
    #[msg("Signer is not the market authority")]
    NotAuthority,
    #[msg("Marketplace is paused")]
    MarketPaused,
    #[msg("Item is not tradeable")]
    ItemNotTradeable,
    #[msg("Insufficient token balance")]
    InsufficientTokens,
    #[msg("Listing is not in Active state")]
    ListingNotActive,
    #[msg("Listing has expired")]
    ListingExpiredError,
    #[msg("Bid must be higher than current bid")]
    BidTooLow,
    #[msg("Bid must be less than buyout price; use buyout instead")]
    BidExceedsBuyout,
    #[msg("Cannot cancel a listing that has bids")]
    HasBids,
    #[msg("Only the seller can perform this action")]
    NotSeller,
    #[msg("Duration must be between 1 and 48 hours")]
    InvalidDuration,
    #[msg("Price must be greater than zero")]
    InvalidPrice,

    #[msg("Insufficient escrow balance")]
    InsufficientEscrowBalance,

    #[msg("Player escrow account not found")]
    EscrowNotFound,
}
