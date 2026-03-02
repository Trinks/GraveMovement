# Alerith Online — Sponsor Integration: MagicBlock VRF

## Sponsor: MagicBlock — Verifiable Random Function (VRF)

We integrated MagicBlock's VRF oracle to bring **provably fair boss loot drops** to our MMORPG. This document describes exactly how MagicBlock VRF is used, with full source code for every layer of the integration.

---

## Why VRF?

In traditional MMOs, players have no way to verify that loot drops are fair. The server rolls random numbers internally and players must trust it. With MagicBlock's VRF:

1. **Neither the server nor the player controls the randomness** — it comes from MagicBlock's on-chain oracle
2. **Every loot roll is recorded on-chain** with the VRF seed, loot table hash, and results
3. **Anyone can independently verify** that the loot distributed matches what the VRF seed dictates
4. **Cryptographic proof of fairness** — if the server lies about drops, the math won't match

---

## Architecture Overview

```
[Boss Dies in Game]
       |
[Unity Game Server] ──HTTP──> [Express.js Sidecar] ──Anchor CPI──> [VRF Loot Program on Solana]
       |                              |                                        |
       |                              |                              [MagicBlock VRF Oracle]
       |                              |                              (fulfills randomness)
       |                              |                                        |
       |                      (polls for fulfillment)                          |
       |                              |<───────────callback_fulfill_vrf────────┘
       |                              |
       |<──────VRF seed (32 bytes)────┘
       |
[SeededRandom: xoshiro256**]
       |
[Roll loot against boss loot table]
       |
[Publish LootReceipt PDA on-chain]
       |
[Immutable audit trail — anyone can verify]
```

### Flow Summary

1. Boss dies → game server calls sidecar `/api/v1/vrf/request-and-wait`
2. Sidecar sends `request_vrf` instruction → CPI into MagicBlock VRF program
3. MagicBlock oracle fulfills with 32-byte random seed via `callback_fulfill_vrf`
4. Sidecar polls VrfRequest PDA until state = Fulfilled, returns seed to game server
5. Game server creates `SeededRandom(vrfSeed)` — a deterministic xoshiro256** PRNG
6. Loot is rolled using this seeded PRNG against the boss's loot table
7. Game server publishes an immutable `LootReceipt` PDA containing: VRF seed, loot table hash, every item rolled, coin amounts, player assignments
8. **Verification**: Anyone downloads the receipt, runs xoshiro256** with the same seed, and confirms the results match

---

## Program IDs (Devnet)

| Program | Address |
|---------|---------|
| VRF Loot | `ENJmHMGDHpa83QvakPL99hPkY18s3KwvMTPrcHGfAStc` |
| Arena (PvP Wagering) | `29ZHkMATNJ9kZoeFNhExSiskwk8BL1W6roqaiEQQneYF` |
| Marketplace (NFT Trading) | `Beva7XHsfKZM7zTZUz4dgXqCxfDM3Xc4wVSx9swYWf3F` |

---

## Code Structure

All Solana integration code is included in this submission:

```
GraveyardSubmission/
├── SUBMISSION.md                    # Hackathon submission form
├── SPONSOR_INTEGRATION.md           # This document
├── VIDEO_SCRIPT.md                  # Demo video script
│
├── anchor-program/
│   └── alerith_loot_vrf.rs         # Full Anchor program (Rust) — VRF Loot
│
├── sidecar/
│   ├── vrf-service.ts              # Express sidecar VRF service (TypeScript)
│   ├── vrf-routes.ts               # REST API routes for VRF
│   └── vault-service.ts            # NFT mint/burn/query service (Metaplex Core)
│
└── game-server/
    ├── VrfLootProvider.cs           # Static delegate bridge (Core assembly)
    ├── SolanaLootService.cs         # Server-side VRF bridge (registers delegates)
    └── SeededRandom.cs              # xoshiro256** PRNG (deterministic, cross-platform)
```

---

## Layer 1: Anchor Program (Rust)

**File: `anchor-program/alerith_loot_vrf.rs`**

The on-chain program with 5 instructions:

| Instruction | Description |
|---|---|
| `initialize` | Create the global `LootVrfConfig` PDA (authority, oracle queue, counters) |
| `request_vrf` | CPI into MagicBlock VRF to request randomness for a boss kill |
| `callback_fulfill_vrf` | MagicBlock oracle callback — delivers verified 32-byte random seed |
| `publish_loot_receipt` | Publish an immutable `LootReceipt` PDA with full loot audit trail |
| `verify_loot` | Permissionless verification — anyone can check a receipt hash matches |

### Key MagicBlock Integration Points

**VRF Request (CPI into MagicBlock):**
```rust
// Build callback so MagicBlock knows where to deliver randomness
let callback_discriminator = sha256("global:callback_fulfill_vrf")[..8];
let callback_accounts = vec![SerializableAccountMeta {
    pubkey: vrf_request.key(),
    is_signer: false,
    is_writable: true,
}];

let ix = create_request_regular_randomness_ix(RequestRandomnessParams {
    payer: authority.key(),
    oracle_queue: oracle_queue.key(),
    callback_program_id: crate::ID,
    callback_discriminator,
    caller_seed,
    accounts_metas: Some(callback_accounts),
    ..Default::default()
});

// The #[vrf] macro generates invoke_signed_vrf() for the CPI
ctx.accounts.invoke_signed_vrf(&authority, &ix)?;
```

**VRF Callback (oracle delivers randomness):**
```rust
pub fn callback_fulfill_vrf(ctx: Context<CallbackFulfillVrf>, randomness: [u8; 32]) -> Result<()> {
    let vrf_request = &mut ctx.accounts.vrf_request;
    vrf_request.randomness = randomness;
    vrf_request.state = VrfRequestState::Fulfilled;
    // ...
}
```

**Callback Security:**
```rust
#[derive(Accounts)]
pub struct CallbackFulfillVrf<'info> {
    // Only MagicBlock's VRF program can sign this PDA
    #[account(address = ephemeral_vrf_sdk::consts::VRF_PROGRAM_IDENTITY)]
    pub vrf_program_identity: Signer<'info>,
    #[account(mut)]
    pub vrf_request: Account<'info, VrfRequest>,
}
```

**Loot Receipt (immutable on-chain audit):**
```rust
pub struct LootReceipt {
    pub vrf_seed: [u8; 32],           // The VRF randomness used
    pub loot_table_hash: [u8; 32],    // SHA-256 of loot table config
    pub generated_loot_hash: [u8; 32],// SHA-256 of actual results
    pub items: [LootItemRecord; 16],  // Every item rolled
    pub coins_generated: u64,         // Total coins
    pub assignments: [LootAssignment; 16], // Who got what
    pub is_fallback_rng: bool,        // True if VRF timed out
    // ...
}
```

---

## Layer 2: Express.js Sidecar (TypeScript)

**File: `sidecar/vrf-service.ts`**

The sidecar handles Anchor instruction building, signing, and VRF polling:

```typescript
// Request VRF and wait for MagicBlock oracle callback
export async function requestVrfAndWait(creatureId, creatureName, eligiblePlayers, timeoutMs = 30000) {
    // 1. Send request_vrf instruction (CPI into MagicBlock)
    const reqResult = await requestVrf(creatureId, creatureName, eligiblePlayers);

    // 2. Poll VrfRequest PDA until fulfilled (oracle typically responds in 1-2 seconds)
    const waitResult = await waitForFulfillment(parseInt(reqResult.requestId), timeoutMs);

    return { requestId, fulfilled, randomness, seedHex, elapsedMs };
}
```

**File: `sidecar/vrf-routes.ts`**

REST endpoints called by the Unity game server:
- `POST /api/v1/vrf/request-and-wait` — Request VRF + wait for oracle
- `POST /api/v1/vrf/publish-receipt` — Publish loot receipt on-chain
- `GET /api/v1/vrf/receipt/:id` — Query a loot receipt

---

## Layer 3: Unity Game Server (C#)

**File: `game-server/VrfLootProvider.cs`**

Static delegate bridge between the Core assembly (where loot generation lives) and the Server assembly (where Solana integration lives). This pattern avoids assembly dependency cycles:

```csharp
public static class VrfLootProvider
{
    // Set by Server assembly's SolanaLootService on startup
    public static RequestVrfSeedDelegate RequestVrfSeed;
    public static PublishReceiptDelegate PublishReceipt;

    public static bool IsAvailable => RequestVrfSeed != null;
}
```

**File: `game-server/SolanaLootService.cs`**

Registers the VRF delegates and bridges async HTTP calls to the sidecar:

```csharp
// On server startup:
VrfLootProvider.RequestVrfSeed = HandleRequestVrfSeed;
VrfLootProvider.PublishReceipt = HandlePublishReceipt;

// When a boss dies:
// 1. Call sidecar /api/v1/vrf/request-and-wait
// 2. Get 32-byte VRF seed back
// 3. Dispatch to main thread via MainThreadDispatcher
// 4. Core assembly creates SeededRandom(seed) and rolls loot
// 5. Publish receipt back to sidecar (fire-and-forget)
```

**File: `game-server/SeededRandom.cs`**

The deterministic PRNG that makes verification possible:

```csharp
// xoshiro256** — identical output in C#, Rust, Python, JavaScript
public sealed class SeededRandom
{
    private ulong _s0, _s1, _s2, _s3; // 256-bit state from VRF seed

    public SeededRandom(byte[] vrfSeed) { /* init from 32-byte seed */ }
    public float Value()               { /* [0, 1) uniform float */ }
    public int Range(int min, int max) { /* inclusive, unbiased */ }
}
```

The key property: given the same 32-byte VRF seed, `SeededRandom` produces **identical** results on any platform. This means anyone can download a `LootReceipt` from Solana, reconstruct the PRNG, replay the loot rolls, and verify the server was honest.

---

## Verification Flow

For any loot receipt on-chain:

1. **Read the `LootReceipt` PDA** — get `vrf_seed`, `loot_table_hash`, `items`, `assignments`
2. **Download the loot table** — verify its SHA-256 matches `loot_table_hash`
3. **Create `SeededRandom(vrf_seed)`** — initialize xoshiro256** with the VRF output
4. **Replay the rolls** — walk the loot table pools, call `Range()` for each roll
5. **Compare** — the replayed items must exactly match `items` and `assignments` in the receipt
6. **If they don't match** — the server lied about the drops (cryptographic proof of cheating)

The `verify_loot` instruction on-chain also allows permissionless hash verification without needing to replay the full PRNG — just compare the `generated_loot_hash`.

---

## Dependencies

| Dependency | Version | Purpose |
|---|---|---|
| `ephemeral-vrf-sdk` | latest | MagicBlock VRF CPI helpers and `#[vrf]` macro |
| `anchor-lang` | 0.30.1 | Solana program framework |
| `@coral-xyz/anchor` | 0.30.1 | TypeScript client for Anchor programs |
| `@solana/web3.js` | 1.x | Solana JavaScript SDK |

---

## What MagicBlock Enables

Without MagicBlock's VRF oracle, provably fair loot is **impossible**:

- **Server-generated randomness** → server can rig results
- **Client-generated randomness** → player can rig results
- **Block hash as randomness** → miners/validators can influence
- **MagicBlock VRF** → neither party controls the randomness, cryptographically verified on-chain

MagicBlock VRF is the **trust anchor** of our entire fairness guarantee. It provides the one thing we can't produce ourselves: a source of randomness that is both unpredictable and verifiable.
