use anchor_lang::prelude::*;
use ephemeral_vrf_sdk::anchor::vrf;
use ephemeral_vrf_sdk::instructions::{
    create_request_regular_randomness_ix, RequestRandomnessParams,
};
use ephemeral_vrf_sdk::types::SerializableAccountMeta;

declare_id!("ENJmHMGDHpa83QvakPL99hPkY18s3KwvMTPrcHGfAStc");

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAX_PLAYERS: usize = 8;
const MAX_ITEMS: usize = 16;
const MAX_ASSIGNMENTS: usize = 16;

// ---------------------------------------------------------------------------
// Data structs (fixed-size, used inside accounts)
// ---------------------------------------------------------------------------

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Default, InitSpace, PartialEq, Eq)]
pub struct LootItemRecord {
    /// Game item ID (matches Unity ItemStruct.itemId)
    pub item_id: u32,
    /// Stack quantity
    pub quantity: u16,
    /// Which LootPool index generated this item
    pub pool_index: u8,
    /// The raw random roll value used (for deterministic replay)
    pub roll_value: u32,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Default, InitSpace, PartialEq, Eq)]
pub struct LootAssignment {
    /// Game character ID that received the item
    pub character_id: u64,
    /// Item ID assigned
    pub item_id: u32,
    /// Quantity assigned
    pub quantity: u16,
    /// Reason: 0=Solo, 1=NeedWin, 2=GreedWin, 3=RoundRobin, 4=MasterLoot
    pub reason: u8,
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Default, InitSpace, PartialEq, Eq)]
#[repr(u8)]
pub enum VrfRequestState {
    #[default]
    Pending = 0,
    Fulfilled = 1,
    Used = 2,
    Expired = 3,
}

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

#[account]
#[derive(InitSpace)]
pub struct LootVrfConfig {
    /// Authority that can request VRF and publish receipts (game server)
    pub authority: Pubkey,
    /// MagicBlock oracle queue address (DEFAULT_QUEUE on devnet)
    pub oracle_queue: Pubkey,
    /// Monotonically increasing VRF request counter
    pub request_count: u64,
    /// Monotonically increasing receipt counter
    pub receipt_count: u64,
    /// PDA bump
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct VrfRequest {
    /// Unique request identifier
    pub request_id: u64,
    /// Game creature ID (CreatureInfo.Id)
    pub creature_id: u32,
    /// SHA-256 hash of creature name for off-chain identification
    pub creature_name_hash: [u8; 32],
    /// Pubkey that created the request (must be authority)
    pub requester: Pubkey,
    /// Current state of the request
    pub state: VrfRequestState,
    /// VRF output randomness (all zeros until fulfilled)
    pub randomness: [u8; 32],
    /// Number of eligible players (1..=8)
    pub eligible_player_count: u8,
    /// Fixed array of character IDs (unused slots are 0)
    pub eligible_players: [u64; 8],
    /// Unix timestamp when request was created
    pub requested_at: i64,
    /// Unix timestamp when VRF was fulfilled (0 until fulfilled)
    pub fulfilled_at: i64,
    /// PDA bump
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct LootReceipt {
    /// Unique receipt identifier
    pub receipt_id: u64,
    /// The VRF request this receipt is based on
    pub vrf_request_id: u64,
    /// Game creature ID
    pub creature_id: u32,
    /// The VRF randomness seed used for loot generation
    pub vrf_seed: [u8; 32],
    /// SHA-256 of the serialized loot table config at generation time
    pub loot_table_hash: [u8; 32],
    /// SHA-256 of the actual generated loot results
    pub generated_loot_hash: [u8; 32],
    /// Number of items in the items array
    pub item_count: u8,
    /// Fixed array of loot items (unused slots are zeroed)
    pub items: [LootItemRecord; 16],
    /// Total coins generated
    pub coins_generated: u64,
    /// Distribution mode: 0=FFA, 1=Personal, 2=RoundRobin, 3=NeedGreed, 4=MasterLoot
    pub distribution_mode: u8,
    /// Number of assignments
    pub assignment_count: u8,
    /// Fixed array of item assignments (unused slots are zeroed)
    pub assignments: [LootAssignment; 16],
    /// True if VRF timed out and Unity.Random was used instead
    pub is_fallback_rng: bool,
    /// Unix timestamp when receipt was published on-chain
    pub published_at: i64,
    /// PDA bump
    pub bump: u8,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[event]
pub struct VrfRequested {
    pub request_id: u64,
    pub creature_id: u32,
    pub eligible_player_count: u8,
}

#[event]
pub struct VrfFulfilled {
    pub request_id: u64,
    /// First 4 bytes of randomness for compact logging
    pub randomness_preview: [u8; 4],
}

#[event]
pub struct LootPublished {
    pub receipt_id: u64,
    pub request_id: u64,
    pub creature_id: u32,
    pub item_count: u8,
    pub coins: u64,
    pub is_fallback: bool,
}

#[event]
pub struct LootVerified {
    pub receipt_id: u64,
    pub is_valid: bool,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[error_code]
pub enum LootVrfError {
    #[msg("Signer is not the config authority")]
    NotAuthority,
    #[msg("VRF request is not in Pending state")]
    RequestNotPending,
    #[msg("VRF request is not in Fulfilled state")]
    RequestNotFulfilled,
    #[msg("Too many players (max 8)")]
    TooManyPlayers,
    #[msg("Too many items (max 16)")]
    TooManyItems,
    #[msg("Too many assignments (max 16)")]
    TooManyAssignments,
    #[msg("Invalid distribution mode (must be 0-4)")]
    InvalidDistributionMode,
    #[msg("VRF request has already been used")]
    RequestAlreadyUsed,
}

// ---------------------------------------------------------------------------
// Program
// ---------------------------------------------------------------------------

#[program]
pub mod alerith_loot_vrf {
    use super::*;

    /// Initialize the global LootVrfConfig PDA.
    pub fn initialize(ctx: Context<Initialize>, oracle_queue: Pubkey) -> Result<()> {
        let config = &mut ctx.accounts.loot_vrf_config;
        config.authority = ctx.accounts.authority.key();
        config.oracle_queue = oracle_queue;
        config.request_count = 0;
        config.receipt_count = 0;
        config.bump = ctx.bumps.loot_vrf_config;
        Ok(())
    }

    /// Update the oracle queue address (authority only).
    pub fn update_config(ctx: Context<UpdateConfig>, new_oracle_queue: Pubkey) -> Result<()> {
        let config = &mut ctx.accounts.loot_vrf_config;
        require!(
            ctx.accounts.authority.key() == config.authority,
            LootVrfError::NotAuthority
        );
        config.oracle_queue = new_oracle_queue;
        Ok(())
    }

    /// Request VRF randomness for a boss kill event.
    /// CPIs into MagicBlock's VRF program; oracle will callback with randomness.
    pub fn request_vrf(
        ctx: Context<RequestVrf>,
        creature_id: u32,
        creature_name_hash: [u8; 32],
        eligible_players: Vec<u64>,
    ) -> Result<()> {
        let player_count = eligible_players.len();
        require!(player_count <= MAX_PLAYERS, LootVrfError::TooManyPlayers);

        let config = &mut ctx.accounts.loot_vrf_config;

        // Verify signer is the authority
        require!(
            ctx.accounts.authority.key() == config.authority,
            LootVrfError::NotAuthority
        );

        let request_id = config.request_count;
        let clock = Clock::get()?;

        // Populate the VRF request account
        let vrf_request = &mut ctx.accounts.vrf_request;
        vrf_request.request_id = request_id;
        vrf_request.creature_id = creature_id;
        vrf_request.creature_name_hash = creature_name_hash;
        vrf_request.requester = ctx.accounts.authority.key();
        vrf_request.state = VrfRequestState::Pending;
        vrf_request.randomness = [0u8; 32];
        vrf_request.eligible_player_count = player_count as u8;

        // Copy eligible players into fixed array, zero-fill remaining
        let mut players_array = [0u64; MAX_PLAYERS];
        for (i, pid) in eligible_players.iter().enumerate() {
            players_array[i] = *pid;
        }
        vrf_request.eligible_players = players_array;

        vrf_request.requested_at = clock.unix_timestamp;
        vrf_request.fulfilled_at = 0;
        vrf_request.bump = ctx.bumps.vrf_request;

        // Increment the global counter
        config.request_count = config.request_count.checked_add(1).unwrap();

        // --- CPI into MagicBlock VRF ---
        // Build caller_seed from request_id for deterministic linking
        let mut caller_seed = [0u8; 32];
        caller_seed[..8].copy_from_slice(&request_id.to_le_bytes());

        // Compute callback discriminator: first 8 bytes of sha256("global:callback_fulfill_vrf")
        let disc_hash =
            anchor_lang::solana_program::hash::hash(b"global:callback_fulfill_vrf");
        let callback_discriminator = disc_hash.to_bytes()[..8].to_vec();

        // The callback needs the VrfRequest PDA as a writable account
        let callback_accounts = vec![SerializableAccountMeta {
            pubkey: ctx.accounts.vrf_request.key(),
            is_signer: false,
            is_writable: true,
        }];

        let ix = create_request_regular_randomness_ix(RequestRandomnessParams {
            payer: ctx.accounts.authority.key(),
            oracle_queue: ctx.accounts.oracle_queue.key(),
            callback_program_id: crate::ID,
            callback_discriminator,
            caller_seed,
            accounts_metas: Some(callback_accounts),
            ..Default::default()
        });

        // invoke_signed_vrf is generated by the #[vrf] macro on RequestVrf
        ctx.accounts
            .invoke_signed_vrf(&ctx.accounts.authority.to_account_info(), &ix)?;

        emit!(VrfRequested {
            request_id,
            creature_id,
            eligible_player_count: player_count as u8,
        });

        Ok(())
    }

    /// MagicBlock VRF callback — oracle delivers verified randomness.
    /// The VRF program has already verified the cryptographic proof on-chain.
    pub fn callback_fulfill_vrf(
        ctx: Context<CallbackFulfillVrf>,
        randomness: [u8; 32],
    ) -> Result<()> {
        let vrf_request = &mut ctx.accounts.vrf_request;

        // Must be in Pending state
        require!(
            vrf_request.state == VrfRequestState::Pending,
            LootVrfError::RequestNotPending
        );

        let clock = Clock::get()?;

        vrf_request.randomness = randomness;
        vrf_request.state = VrfRequestState::Fulfilled;
        vrf_request.fulfilled_at = clock.unix_timestamp;

        // Emit event with first 4 bytes of randomness for compact logs
        let mut preview = [0u8; 4];
        preview.copy_from_slice(&randomness[..4]);

        emit!(VrfFulfilled {
            request_id: vrf_request.request_id,
            randomness_preview: preview,
        });

        Ok(())
    }

    /// Game server publishes the deterministic loot results on-chain.
    pub fn publish_loot_receipt(
        ctx: Context<PublishLootReceipt>,
        _request_id: u64,
        loot_table_hash: [u8; 32],
        generated_loot_hash: [u8; 32],
        items: Vec<LootItemRecord>,
        coins: u64,
        distribution_mode: u8,
        assignments: Vec<LootAssignment>,
        is_fallback_rng: bool,
    ) -> Result<()> {
        require!(items.len() <= MAX_ITEMS, LootVrfError::TooManyItems);
        require!(
            assignments.len() <= MAX_ASSIGNMENTS,
            LootVrfError::TooManyAssignments
        );
        require!(distribution_mode <= 4, LootVrfError::InvalidDistributionMode);

        let config = &mut ctx.accounts.loot_vrf_config;

        // Verify signer is the authority
        require!(
            ctx.accounts.authority.key() == config.authority,
            LootVrfError::NotAuthority
        );

        let vrf_request = &mut ctx.accounts.vrf_request;

        // Guard against reuse
        require!(
            vrf_request.state != VrfRequestState::Used,
            LootVrfError::RequestAlreadyUsed
        );

        // If fallback RNG, allow Pending state (VRF timed out).
        // Otherwise require Fulfilled.
        if is_fallback_rng {
            require!(
                vrf_request.state == VrfRequestState::Pending
                    || vrf_request.state == VrfRequestState::Fulfilled,
                LootVrfError::RequestNotFulfilled
            );
        } else {
            require!(
                vrf_request.state == VrfRequestState::Fulfilled,
                LootVrfError::RequestNotFulfilled
            );
        }

        let receipt_id = config.receipt_count;
        let clock = Clock::get()?;

        // Build fixed-size items array
        let mut items_array = [LootItemRecord::default(); MAX_ITEMS];
        for (i, item) in items.iter().enumerate() {
            items_array[i] = *item;
        }

        // Build fixed-size assignments array
        let mut assignments_array = [LootAssignment::default(); MAX_ASSIGNMENTS];
        for (i, assignment) in assignments.iter().enumerate() {
            assignments_array[i] = *assignment;
        }

        // Populate receipt
        let receipt = &mut ctx.accounts.loot_receipt;
        receipt.receipt_id = receipt_id;
        receipt.vrf_request_id = vrf_request.request_id;
        receipt.creature_id = vrf_request.creature_id;
        receipt.vrf_seed = vrf_request.randomness;
        receipt.loot_table_hash = loot_table_hash;
        receipt.generated_loot_hash = generated_loot_hash;
        receipt.item_count = items.len() as u8;
        receipt.items = items_array;
        receipt.coins_generated = coins;
        receipt.distribution_mode = distribution_mode;
        receipt.assignment_count = assignments.len() as u8;
        receipt.assignments = assignments_array;
        receipt.is_fallback_rng = is_fallback_rng;
        receipt.published_at = clock.unix_timestamp;
        receipt.bump = ctx.bumps.loot_receipt;

        // Mark the VRF request as used
        vrf_request.state = VrfRequestState::Used;

        // Increment receipt counter
        config.receipt_count = config.receipt_count.checked_add(1).unwrap();

        emit!(LootPublished {
            receipt_id,
            request_id: vrf_request.request_id,
            creature_id: vrf_request.creature_id,
            item_count: receipt.item_count,
            coins,
            is_fallback: is_fallback_rng,
        });

        Ok(())
    }

    /// Permissionless verification: anyone can check a loot receipt hash.
    /// Does NOT fail on mismatch -- emits an event with is_valid = false instead.
    pub fn verify_loot(
        ctx: Context<VerifyLoot>,
        _receipt_id: u64,
        expected_loot_hash: [u8; 32],
    ) -> Result<()> {
        let receipt = &ctx.accounts.loot_receipt;
        let is_valid = receipt.generated_loot_hash == expected_loot_hash;

        emit!(LootVerified {
            receipt_id: receipt.receipt_id,
            is_valid,
        });

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Instruction account structs
// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + LootVrfConfig::INIT_SPACE,
        seeds = [b"loot_vrf_config"],
        bump,
    )]
    pub loot_vrf_config: Account<'info, LootVrfConfig>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdateConfig<'info> {
    #[account(
        mut,
        seeds = [b"loot_vrf_config"],
        bump = loot_vrf_config.bump,
    )]
    pub loot_vrf_config: Account<'info, LootVrfConfig>,

    #[account(mut)]
    pub authority: Signer<'info>,
}

/// RequestVrf context — the #[vrf] macro auto-injects:
///   - program_identity: PDA [b"identity"] for our program (signer in CPI)
///   - vrf_program: MagicBlock's VRF program reference
///   - slot_hashes: SlotHashes sysvar
/// It also generates invoke_signed_vrf() which does the CPI.
#[vrf]
#[derive(Accounts)]
pub struct RequestVrf<'info> {
    #[account(
        mut,
        seeds = [b"loot_vrf_config"],
        bump = loot_vrf_config.bump,
    )]
    pub loot_vrf_config: Account<'info, LootVrfConfig>,

    #[account(
        init,
        payer = authority,
        space = 8 + VrfRequest::INIT_SPACE,
        seeds = [b"vrf_request", loot_vrf_config.request_count.to_le_bytes().as_ref()],
        bump,
    )]
    pub vrf_request: Account<'info, VrfRequest>,

    #[account(mut)]
    pub authority: Signer<'info>,

    /// CHECK: MagicBlock oracle queue account, validated by VRF program
    #[account(mut)]
    pub oracle_queue: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

/// Callback context — only the MagicBlock VRF program can call this.
/// The vrf_program_identity PDA is signed by the VRF program via CPI,
/// so no unauthorized caller can invoke this instruction.
#[derive(Accounts)]
pub struct CallbackFulfillVrf<'info> {
    /// MagicBlock VRF program identity — MUST be a signer.
    /// This PDA can only be signed by the VRF program itself.
    #[account(address = ephemeral_vrf_sdk::consts::VRF_PROGRAM_IDENTITY)]
    pub vrf_program_identity: Signer<'info>,

    /// The VRF request to fulfill (passed via accounts_metas in the request).
    #[account(mut)]
    pub vrf_request: Account<'info, VrfRequest>,
}

#[derive(Accounts)]
#[instruction(_request_id: u64)]
pub struct PublishLootReceipt<'info> {
    #[account(
        mut,
        seeds = [b"loot_vrf_config"],
        bump = loot_vrf_config.bump,
    )]
    pub loot_vrf_config: Account<'info, LootVrfConfig>,

    #[account(
        mut,
        seeds = [b"vrf_request", _request_id.to_le_bytes().as_ref()],
        bump = vrf_request.bump,
    )]
    pub vrf_request: Account<'info, VrfRequest>,

    #[account(
        init,
        payer = authority,
        space = 8 + LootReceipt::INIT_SPACE,
        seeds = [b"loot_receipt", loot_vrf_config.receipt_count.to_le_bytes().as_ref()],
        bump,
    )]
    pub loot_receipt: Account<'info, LootReceipt>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(_receipt_id: u64)]
pub struct VerifyLoot<'info> {
    #[account(
        seeds = [b"loot_receipt", _receipt_id.to_le_bytes().as_ref()],
        bump = loot_receipt.bump,
    )]
    pub loot_receipt: Account<'info, LootReceipt>,
}
