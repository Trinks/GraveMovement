use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("29ZHkMATNJ9kZoeFNhExSiskwk8BL1W6roqaiEQQneYF");

// ---------------------------------------------------------------------------
// Program
// ---------------------------------------------------------------------------

#[program]
pub mod alerith_arena {
    use super::*;

    /// Create the global ArenaConfig PDA. Only callable once.
    pub fn initialize(
        ctx: Context<Initialize>,
        oracle: Pubkey,
        treasury: Pubkey,
        fee_bps: u16,
        min_wager: u64,
        max_wager: u64,
    ) -> Result<()> {
        let config = &mut ctx.accounts.arena_config;
        config.authority = ctx.accounts.authority.key();
        config.oracle = oracle;
        config.treasury = treasury;
        config.fee_bps = fee_bps;
        config.min_wager = min_wager;
        config.max_wager = max_wager;
        config.match_count = 0;
        config.paused = false;
        Ok(())
    }

    /// Create a new MatchEscrow. Caller becomes player_a.
    pub fn create_match(
        ctx: Context<CreateMatch>,
        arena_type: u8,
        wager_amount: u64,
    ) -> Result<()> {
        let config = &mut ctx.accounts.arena_config;
        require!(!config.paused, ArenaError::ArenaPaused);
        require!(arena_type <= 3, ArenaError::InvalidArenaType);
        require!(wager_amount >= config.min_wager, ArenaError::WagerTooLow);
        require!(wager_amount <= config.max_wager, ArenaError::WagerTooHigh);

        let match_id = config.match_count;
        config.match_count = config.match_count.checked_add(1).unwrap();

        let escrow = &mut ctx.accounts.match_escrow;
        escrow.match_id = match_id;
        escrow.arena_type = arena_type;
        escrow.state = MatchState::Created;
        escrow.player_a = ctx.accounts.player.key();
        escrow.player_b = Pubkey::default();
        escrow.wager_amount = wager_amount;
        escrow.created_at = Clock::get()?.unix_timestamp;
        escrow.settled_at = 0;
        escrow.winner = Pubkey::default();
        escrow.combat_hash = [0u8; 32];
        escrow.bump = ctx.bumps.match_escrow;

        emit!(MatchCreated {
            match_id,
            arena_type,
            player_a: escrow.player_a,
            wager_amount,
        });

        Ok(())
    }

    /// Second player joins an existing match.
    pub fn join_match(ctx: Context<JoinMatch>, _match_id: u64) -> Result<()> {
        let escrow = &mut ctx.accounts.match_escrow;
        require!(
            escrow.state == MatchState::Created,
            ArenaError::MatchNotCreated
        );
        require!(
            escrow.player_b == Pubkey::default(),
            ArenaError::AlreadyJoined
        );
        // Prevent player_a from joining their own match as player_b.
        require!(
            ctx.accounts.player.key() != escrow.player_a,
            ArenaError::AlreadyJoined
        );

        escrow.player_b = ctx.accounts.player.key();

        emit!(MatchJoined {
            match_id: escrow.match_id,
            player_b: escrow.player_b,
        });

        Ok(())
    }

    /// Player deposits their wager into the escrow PDA.
    pub fn fund_match(ctx: Context<FundMatch>, _match_id: u64) -> Result<()> {
        let escrow = &ctx.accounts.match_escrow;
        let signer_key = ctx.accounts.player.key();

        // Must be one of the two players.
        require!(
            signer_key == escrow.player_a || signer_key == escrow.player_b,
            ArenaError::NotAPlayer
        );

        // Match must be in Created state (both players assigned but not yet fully funded)
        // or still awaiting second fund while in Created.
        require!(
            escrow.state == MatchState::Created,
            ArenaError::MatchNotCreated
        );

        // Ensure player_b has actually joined.
        require!(
            escrow.player_b != Pubkey::default(),
            ArenaError::MatchNotCreated
        );

        // Prevent double-funding: check the escrow balance to see if this player
        // has already deposited. Before any funding the escrow holds only rent.
        // After one deposit it holds rent + wager_amount.
        // After two deposits it holds rent + 2 * wager_amount.
        let escrow_info = ctx.accounts.match_escrow.to_account_info();
        let escrow_lamports = escrow_info.lamports();
        let rent = Rent::get()?;
        let escrow_data_len = escrow_info.data_len();
        let rent_exempt_min = rent.minimum_balance(escrow_data_len);

        let deposited_so_far = escrow_lamports.saturating_sub(rent_exempt_min);
        let wager = escrow.wager_amount;

        // If the full 2x wager is already deposited, both have funded.
        require!(
            deposited_so_far < wager.checked_mul(2).unwrap(),
            ArenaError::AlreadyFunded
        );

        // Transfer wager from player to escrow PDA via system_program.
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.player.to_account_info(),
                    to: ctx.accounts.match_escrow.to_account_info(),
                },
            ),
            wager,
        )?;

        // Re-check balance after transfer.
        let new_balance = ctx.accounts.match_escrow.to_account_info().lamports();
        let new_deposited = new_balance.saturating_sub(rent_exempt_min);

        if new_deposited >= wager.checked_mul(2).unwrap() {
            // Both players have funded -- advance state.
            let escrow_mut = &mut ctx.accounts.match_escrow;
            escrow_mut.state = MatchState::Funded;

            emit!(MatchFunded {
                match_id: escrow_mut.match_id,
            });
        }

        Ok(())
    }

    /// Oracle marks the match as active (combat started on game server).
    pub fn activate_match(ctx: Context<ActivateMatch>, _match_id: u64) -> Result<()> {
        let config = &ctx.accounts.arena_config;
        require!(
            ctx.accounts.oracle.key() == config.oracle,
            ArenaError::NotOracle
        );

        let escrow = &mut ctx.accounts.match_escrow;
        require!(
            escrow.state == MatchState::Funded,
            ArenaError::MatchNotFunded
        );

        escrow.state = MatchState::Active;
        escrow.created_at = Clock::get()?.unix_timestamp; // update timestamp for activation

        emit!(MatchActivated {
            match_id: escrow.match_id,
        });

        Ok(())
    }

    /// Oracle submits the combat result.
    pub fn submit_result(
        ctx: Context<SubmitResult>,
        _match_id: u64,
        winner: Pubkey,
        combat_hash: [u8; 32],
    ) -> Result<()> {
        let config = &ctx.accounts.arena_config;
        require!(
            ctx.accounts.oracle.key() == config.oracle,
            ArenaError::NotOracle
        );

        let escrow = &mut ctx.accounts.match_escrow;
        require!(
            escrow.state == MatchState::Active,
            ArenaError::MatchNotActive
        );
        require!(
            winner == escrow.player_a || winner == escrow.player_b,
            ArenaError::InvalidWinner
        );

        escrow.winner = winner;
        escrow.combat_hash = combat_hash;
        escrow.state = MatchState::Settled;
        escrow.settled_at = Clock::get()?.unix_timestamp;

        emit!(MatchSettled {
            match_id: escrow.match_id,
            winner,
            combat_hash,
        });

        Ok(())
    }

    /// Winner claims the pot minus platform fee. Closes the escrow account.
    pub fn claim_winnings(ctx: Context<ClaimWinnings>, _match_id: u64) -> Result<()> {
        let escrow = &ctx.accounts.match_escrow;
        require!(
            escrow.state == MatchState::Settled,
            ArenaError::MatchNotSettled
        );
        require!(
            ctx.accounts.winner.key() == escrow.winner,
            ArenaError::NotWinner
        );

        let wager = escrow.wager_amount;
        let total_pot = wager.checked_mul(2).unwrap();
        let fee_bps = ctx.accounts.arena_config.fee_bps as u64;
        let fee = total_pot
            .checked_mul(fee_bps)
            .unwrap()
            .checked_div(10_000)
            .unwrap();
        let payout = total_pot.checked_sub(fee).unwrap();

        // Transfer fee to treasury by directly manipulating lamports.
        let escrow_info = ctx.accounts.match_escrow.to_account_info();
        let treasury_info = ctx.accounts.treasury.to_account_info();
        let winner_info = ctx.accounts.winner.to_account_info();

        // Transfer fee to treasury.
        **escrow_info.try_borrow_mut_lamports()? -= fee;
        **treasury_info.try_borrow_mut_lamports()? += fee;

        // Transfer remaining lamports (payout + rent) to winner, effectively closing
        // the escrow account.
        let remaining = escrow_info.lamports();
        **escrow_info.try_borrow_mut_lamports()? -= remaining;
        **winner_info.try_borrow_mut_lamports()? += remaining;

        emit!(WinningsClaimed {
            match_id: escrow.match_id,
            winner: escrow.winner,
            amount: payout,
            fee,
        });

        Ok(())
    }

    // =====================================================================
    // Authority-operated instructions (server signs on behalf of players)
    // =====================================================================

    /// Authority creates a match on behalf of player_a.
    pub fn create_match_operated(
        ctx: Context<CreateMatchOperated>,
        arena_type: u8,
        wager_amount: u64,
        player_a: Pubkey,
    ) -> Result<()> {
        let config = &mut ctx.accounts.arena_config;
        require!(!config.paused, ArenaError::ArenaPaused);
        require!(
            ctx.accounts.authority.key() == config.authority,
            ArenaError::NotAuthority
        );
        require!(arena_type <= 3, ArenaError::InvalidArenaType);
        require!(wager_amount >= config.min_wager, ArenaError::WagerTooLow);
        require!(wager_amount <= config.max_wager, ArenaError::WagerTooHigh);

        let match_id = config.match_count;
        config.match_count = config.match_count.checked_add(1).unwrap();

        let escrow = &mut ctx.accounts.match_escrow;
        escrow.match_id = match_id;
        escrow.arena_type = arena_type;
        escrow.state = MatchState::Created;
        escrow.player_a = player_a;
        escrow.player_b = Pubkey::default();
        escrow.wager_amount = wager_amount;
        escrow.created_at = Clock::get()?.unix_timestamp;
        escrow.settled_at = 0;
        escrow.winner = Pubkey::default();
        escrow.combat_hash = [0u8; 32];
        escrow.bump = ctx.bumps.match_escrow;

        emit!(MatchCreated {
            match_id,
            arena_type,
            player_a,
            wager_amount,
        });

        Ok(())
    }

    /// Authority joins a player_b to an existing match.
    pub fn join_match_operated(
        ctx: Context<JoinMatchOperated>,
        _match_id: u64,
        player_b: Pubkey,
    ) -> Result<()> {
        require!(
            ctx.accounts.authority.key() == ctx.accounts.arena_config.authority,
            ArenaError::NotAuthority
        );

        let escrow = &mut ctx.accounts.match_escrow;
        require!(
            escrow.state == MatchState::Created,
            ArenaError::MatchNotCreated
        );
        require!(
            escrow.player_b == Pubkey::default(),
            ArenaError::AlreadyJoined
        );
        require!(player_b != escrow.player_a, ArenaError::AlreadyJoined);

        escrow.player_b = player_b;

        emit!(MatchJoined {
            match_id: escrow.match_id,
            player_b,
        });

        Ok(())
    }

    /// Authority funds the match escrow (full 2x wager at once).
    pub fn fund_match_operated(
        ctx: Context<FundMatchOperated>,
        _match_id: u64,
    ) -> Result<()> {
        require!(
            ctx.accounts.authority.key() == ctx.accounts.arena_config.authority,
            ArenaError::NotAuthority
        );

        let escrow = &ctx.accounts.match_escrow;
        require!(
            escrow.state == MatchState::Created,
            ArenaError::MatchNotCreated
        );
        require!(
            escrow.player_b != Pubkey::default(),
            ArenaError::MatchNotCreated
        );

        let total_wager = escrow.wager_amount.checked_mul(2).unwrap();

        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.authority.to_account_info(),
                    to: ctx.accounts.match_escrow.to_account_info(),
                },
            ),
            total_wager,
        )?;

        let escrow_mut = &mut ctx.accounts.match_escrow;
        escrow_mut.state = MatchState::Funded;

        emit!(MatchFunded {
            match_id: escrow_mut.match_id,
        });

        Ok(())
    }

    /// Authority claims winnings, sending all lamports to authority.
    pub fn claim_winnings_operated(
        ctx: Context<ClaimWinningsOperated>,
        _match_id: u64,
    ) -> Result<()> {
        let escrow = &ctx.accounts.match_escrow;
        require!(
            escrow.state == MatchState::Settled,
            ArenaError::MatchNotSettled
        );
        require!(
            ctx.accounts.authority.key() == ctx.accounts.arena_config.authority,
            ArenaError::NotAuthority
        );

        let wager = escrow.wager_amount;
        let total_pot = wager.checked_mul(2).unwrap();
        let fee_bps = ctx.accounts.arena_config.fee_bps as u64;
        let fee = total_pot
            .checked_mul(fee_bps)
            .unwrap()
            .checked_div(10_000)
            .unwrap();
        let payout = total_pot.checked_sub(fee).unwrap();

        let escrow_info = ctx.accounts.match_escrow.to_account_info();
        let treasury_info = ctx.accounts.treasury.to_account_info();
        let authority_info = ctx.accounts.authority.to_account_info();

        // Transfer fee to treasury.
        **escrow_info.try_borrow_mut_lamports()? -= fee;
        **treasury_info.try_borrow_mut_lamports()? += fee;

        // Transfer remaining (payout + rent) to authority.
        let remaining = escrow_info.lamports();
        **escrow_info.try_borrow_mut_lamports()? -= remaining;
        **authority_info.try_borrow_mut_lamports()? += remaining;

        emit!(WinningsClaimed {
            match_id: escrow.match_id,
            winner: escrow.winner,
            amount: payout,
            fee,
        });

        Ok(())
    }

    /// Cancel a match that is Created or Funded. Refunds any deposited lamports.
    pub fn cancel_match(ctx: Context<CancelMatch>, _match_id: u64) -> Result<()> {
        let escrow = &ctx.accounts.match_escrow;
        let signer_key = ctx.accounts.authority.key();

        // Must be oracle or player_a.
        require!(
            signer_key == ctx.accounts.arena_config.oracle || signer_key == escrow.player_a,
            ArenaError::NotOracle
        );

        require!(
            escrow.state == MatchState::Created || escrow.state == MatchState::Funded,
            ArenaError::MatchNotCreated
        );

        let escrow_info = ctx.accounts.match_escrow.to_account_info();
        let rent = Rent::get()?;
        let rent_exempt_min = rent.minimum_balance(escrow_info.data_len());
        let deposited = escrow_info.lamports().saturating_sub(rent_exempt_min);
        let wager = escrow.wager_amount;

        // Refund player_a if they deposited.
        if deposited > 0 {
            let refund_a = std::cmp::min(deposited, wager);
            if refund_a > 0 {
                let player_a_info = ctx.accounts.player_a.to_account_info();
                **escrow_info.try_borrow_mut_lamports()? -= refund_a;
                **player_a_info.try_borrow_mut_lamports()? += refund_a;
            }

            // Refund player_b if they also deposited.
            let remaining_deposit = deposited.saturating_sub(refund_a);
            if remaining_deposit > 0 && escrow.player_b != Pubkey::default() {
                let refund_b = std::cmp::min(remaining_deposit, wager);
                if refund_b > 0 {
                    let player_b_info = ctx.accounts.player_b.to_account_info();
                    **escrow_info.try_borrow_mut_lamports()? -= refund_b;
                    **player_b_info.try_borrow_mut_lamports()? += refund_b;
                }
            }
        }

        // Close the escrow account: send any remaining rent to player_a.
        let leftover = escrow_info.lamports();
        if leftover > 0 {
            let player_a_info = ctx.accounts.player_a.to_account_info();
            **escrow_info.try_borrow_mut_lamports()? -= leftover;
            **player_a_info.try_borrow_mut_lamports()? += leftover;
        }

        // Mark as cancelled (account data will be zeroed by runtime on close,
        // but we set it for the event / any trailing reads).
        let escrow_mut = &mut ctx.accounts.match_escrow;
        escrow_mut.state = MatchState::Cancelled;

        emit!(MatchCancelled {
            match_id: escrow_mut.match_id,
        });

        Ok(())
    }

    /// Cancel a match (authority-operated). All lamports go to authority
    /// instead of player wallets. Used when matches are funded from escrow —
    /// the sidecar refunds players' escrow balances separately.
    pub fn cancel_match_operated(
        ctx: Context<CancelMatchOperated>,
        _match_id: u64,
    ) -> Result<()> {
        let config = &ctx.accounts.arena_config;
        require!(
            ctx.accounts.authority.key() == config.authority
                || ctx.accounts.authority.key() == config.oracle,
            ArenaError::NotAuthority
        );

        let escrow = &ctx.accounts.match_escrow;
        require!(
            escrow.state == MatchState::Created || escrow.state == MatchState::Funded,
            ArenaError::MatchNotCreated
        );

        // Transfer ALL lamports (deposits + rent) to authority.
        let escrow_info = ctx.accounts.match_escrow.to_account_info();
        let authority_info = ctx.accounts.authority.to_account_info();
        let remaining = escrow_info.lamports();

        **escrow_info.try_borrow_mut_lamports()? = escrow_info
            .lamports()
            .checked_sub(remaining)
            .unwrap();
        **authority_info.try_borrow_mut_lamports()? = authority_info
            .lamports()
            .checked_add(remaining)
            .unwrap();

        let escrow_mut = &mut ctx.accounts.match_escrow;
        escrow_mut.state = MatchState::Cancelled;

        emit!(MatchCancelled {
            match_id: escrow_mut.match_id,
        });

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Accounts (state)
// ---------------------------------------------------------------------------

#[account]
#[derive(InitSpace)]
pub struct ArenaConfig {
    /// The admin who can update config.
    pub authority: Pubkey,
    /// Server oracle signing key for match results.
    pub oracle: Pubkey,
    /// Treasury wallet that receives platform fees.
    pub treasury: Pubkey,
    /// Fee in basis points (e.g. 500 = 5%).
    pub fee_bps: u16,
    /// Minimum wager in lamports.
    pub min_wager: u64,
    /// Maximum wager in lamports.
    pub max_wager: u64,
    /// Incrementing match counter used to derive MatchEscrow PDAs.
    pub match_count: u64,
    /// Emergency pause flag.
    pub paused: bool,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace)]
#[repr(u8)]
pub enum MatchState {
    Created = 0,
    Funded = 1,
    Active = 2,
    Settled = 3,
    Cancelled = 4,
}

#[account]
#[derive(InitSpace)]
pub struct MatchEscrow {
    /// Unique match identifier.
    pub match_id: u64,
    /// Arena type: 0 = 1v1, 1 = 2v2, 2 = 3v3, 3 = 5v5.
    pub arena_type: u8,
    /// Current match state.
    pub state: MatchState,
    /// First player (match creator).
    pub player_a: Pubkey,
    /// Second player (joiner). Pubkey::default() until someone joins.
    pub player_b: Pubkey,
    /// Wager amount each player must deposit (in lamports).
    pub wager_amount: u64,
    /// Unix timestamp when match was created (updated to activation time on activate).
    pub created_at: i64,
    /// Unix timestamp when match was settled.
    pub settled_at: i64,
    /// Winner pubkey. Pubkey::default() until settled.
    pub winner: Pubkey,
    /// SHA-256 hash of the combat log for verification.
    pub combat_hash: [u8; 32],
    /// PDA bump seed.
    pub bump: u8,
}

// ---------------------------------------------------------------------------
// Instruction account structs
// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + ArenaConfig::INIT_SPACE,
        seeds = [b"arena_config"],
        bump,
    )]
    pub arena_config: Account<'info, ArenaConfig>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CreateMatch<'info> {
    #[account(
        mut,
        seeds = [b"arena_config"],
        bump,
    )]
    pub arena_config: Account<'info, ArenaConfig>,

    #[account(
        init,
        payer = player,
        space = 8 + MatchEscrow::INIT_SPACE,
        seeds = [b"match_escrow", arena_config.match_count.to_le_bytes().as_ref()],
        bump,
    )]
    pub match_escrow: Account<'info, MatchEscrow>,

    #[account(mut)]
    pub player: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(match_id: u64)]
pub struct JoinMatch<'info> {
    #[account(
        mut,
        seeds = [b"match_escrow", match_id.to_le_bytes().as_ref()],
        bump = match_escrow.bump,
    )]
    pub match_escrow: Account<'info, MatchEscrow>,

    #[account(mut)]
    pub player: Signer<'info>,
}

#[derive(Accounts)]
#[instruction(match_id: u64)]
pub struct FundMatch<'info> {
    #[account(
        mut,
        seeds = [b"match_escrow", match_id.to_le_bytes().as_ref()],
        bump = match_escrow.bump,
    )]
    pub match_escrow: Account<'info, MatchEscrow>,

    #[account(mut)]
    pub player: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(match_id: u64)]
pub struct ActivateMatch<'info> {
    #[account(
        seeds = [b"arena_config"],
        bump,
    )]
    pub arena_config: Account<'info, ArenaConfig>,

    #[account(
        mut,
        seeds = [b"match_escrow", match_id.to_le_bytes().as_ref()],
        bump = match_escrow.bump,
    )]
    pub match_escrow: Account<'info, MatchEscrow>,

    #[account(mut)]
    pub oracle: Signer<'info>,
}

#[derive(Accounts)]
#[instruction(match_id: u64)]
pub struct SubmitResult<'info> {
    #[account(
        seeds = [b"arena_config"],
        bump,
    )]
    pub arena_config: Account<'info, ArenaConfig>,

    #[account(
        mut,
        seeds = [b"match_escrow", match_id.to_le_bytes().as_ref()],
        bump = match_escrow.bump,
    )]
    pub match_escrow: Account<'info, MatchEscrow>,

    #[account(mut)]
    pub oracle: Signer<'info>,
}

#[derive(Accounts)]
#[instruction(match_id: u64)]
pub struct ClaimWinnings<'info> {
    #[account(
        seeds = [b"arena_config"],
        bump,
    )]
    pub arena_config: Account<'info, ArenaConfig>,

    #[account(
        mut,
        seeds = [b"match_escrow", match_id.to_le_bytes().as_ref()],
        bump = match_escrow.bump,
    )]
    pub match_escrow: Account<'info, MatchEscrow>,

    /// The winner claiming their winnings.
    #[account(mut)]
    pub winner: Signer<'info>,

    /// Treasury account to receive the platform fee.
    /// CHECK: Validated against arena_config.treasury.
    #[account(
        mut,
        constraint = treasury.key() == arena_config.treasury @ ArenaError::NotOracle
    )]
    pub treasury: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(match_id: u64)]
pub struct CancelMatch<'info> {
    #[account(
        seeds = [b"arena_config"],
        bump,
    )]
    pub arena_config: Account<'info, ArenaConfig>,

    #[account(
        mut,
        seeds = [b"match_escrow", match_id.to_le_bytes().as_ref()],
        bump = match_escrow.bump,
    )]
    pub match_escrow: Account<'info, MatchEscrow>,

    /// Oracle or player_a -- validated in handler logic.
    #[account(mut)]
    pub authority: Signer<'info>,

    /// CHECK: player_a account for refunds. Validated against match_escrow.player_a.
    #[account(
        mut,
        constraint = player_a.key() == match_escrow.player_a @ ArenaError::NotAPlayer
    )]
    pub player_a: AccountInfo<'info>,

    /// CHECK: player_b account for refunds. May be Pubkey::default() if no one joined yet.
    #[account(mut)]
    pub player_b: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

// ---------------------------------------------------------------------------
// Authority-operated instruction account structs
// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct CreateMatchOperated<'info> {
    #[account(
        mut,
        seeds = [b"arena_config"],
        bump,
    )]
    pub arena_config: Account<'info, ArenaConfig>,

    #[account(
        init,
        payer = authority,
        space = 8 + MatchEscrow::INIT_SPACE,
        seeds = [b"match_escrow", arena_config.match_count.to_le_bytes().as_ref()],
        bump,
    )]
    pub match_escrow: Account<'info, MatchEscrow>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(match_id: u64)]
pub struct JoinMatchOperated<'info> {
    #[account(
        seeds = [b"arena_config"],
        bump,
    )]
    pub arena_config: Account<'info, ArenaConfig>,

    #[account(
        mut,
        seeds = [b"match_escrow", match_id.to_le_bytes().as_ref()],
        bump = match_escrow.bump,
    )]
    pub match_escrow: Account<'info, MatchEscrow>,

    #[account(mut)]
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
#[instruction(match_id: u64)]
pub struct FundMatchOperated<'info> {
    #[account(
        seeds = [b"arena_config"],
        bump,
    )]
    pub arena_config: Account<'info, ArenaConfig>,

    #[account(
        mut,
        seeds = [b"match_escrow", match_id.to_le_bytes().as_ref()],
        bump = match_escrow.bump,
    )]
    pub match_escrow: Account<'info, MatchEscrow>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(match_id: u64)]
pub struct ClaimWinningsOperated<'info> {
    #[account(
        seeds = [b"arena_config"],
        bump,
    )]
    pub arena_config: Account<'info, ArenaConfig>,

    #[account(
        mut,
        seeds = [b"match_escrow", match_id.to_le_bytes().as_ref()],
        bump = match_escrow.bump,
    )]
    pub match_escrow: Account<'info, MatchEscrow>,

    #[account(mut)]
    pub authority: Signer<'info>,

    /// CHECK: Treasury account. Validated against arena_config.treasury.
    #[account(
        mut,
        constraint = treasury.key() == arena_config.treasury @ ArenaError::NotOracle
    )]
    pub treasury: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(_match_id: u64)]
pub struct CancelMatchOperated<'info> {
    #[account(
        seeds = [b"arena_config"],
        bump,
    )]
    pub arena_config: Account<'info, ArenaConfig>,

    #[account(
        mut,
        seeds = [b"match_escrow", _match_id.to_le_bytes().as_ref()],
        bump = match_escrow.bump,
    )]
    pub match_escrow: Account<'info, MatchEscrow>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[event]
pub struct MatchCreated {
    pub match_id: u64,
    pub arena_type: u8,
    pub player_a: Pubkey,
    pub wager_amount: u64,
}

#[event]
pub struct MatchJoined {
    pub match_id: u64,
    pub player_b: Pubkey,
}

#[event]
pub struct MatchFunded {
    pub match_id: u64,
}

#[event]
pub struct MatchActivated {
    pub match_id: u64,
}

#[event]
pub struct MatchSettled {
    pub match_id: u64,
    pub winner: Pubkey,
    pub combat_hash: [u8; 32],
}

#[event]
pub struct WinningsClaimed {
    pub match_id: u64,
    pub winner: Pubkey,
    pub amount: u64,
    pub fee: u64,
}

#[event]
pub struct MatchCancelled {
    pub match_id: u64,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[error_code]
pub enum ArenaError {
    #[msg("Invalid arena type. Must be 0 (1v1), 1 (2v2), 2 (3v3), or 3 (5v5).")]
    InvalidArenaType,

    #[msg("Wager amount is below the minimum.")]
    WagerTooLow,

    #[msg("Wager amount exceeds the maximum.")]
    WagerTooHigh,

    #[msg("Match is not in the Created state.")]
    MatchNotCreated,

    #[msg("Match is not in the Funded state.")]
    MatchNotFunded,

    #[msg("Match is not in the Active state.")]
    MatchNotActive,

    #[msg("Match is not in the Settled state.")]
    MatchNotSettled,

    #[msg("A second player has already joined this match.")]
    AlreadyJoined,

    #[msg("Signer is not a player in this match.")]
    NotAPlayer,

    #[msg("Signer is not the match winner.")]
    NotWinner,

    #[msg("Signer is not the authorized oracle.")]
    NotOracle,

    #[msg("The arena is currently paused.")]
    ArenaPaused,

    #[msg("Specified winner is not a participant in this match.")]
    InvalidWinner,

    #[msg("This player has already funded their wager.")]
    AlreadyFunded,

    #[msg("Signer is not the arena authority.")]
    NotAuthority,
}
