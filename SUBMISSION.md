# Graveyard Hackathon Submission

## Project Name
**Alerith** 

## One-Line Description
A fully playable browser-based MMORPG with on-chain PvP wagering, a provably fair VRF loot system, and an NFT item marketplace 

## GitHub Repo
https://github.com/Trinks/GraveMovement

## Demo / Pitch Video
[Link to demo video]

## Relevant Links
- **X Handle:** [@PlayAlerith](https://x.com/PlayAlerith)
- **Telegram Handle:** [@Alerith](https://t.me/Alerith)
- **Website:** (https://alerith.com)

## Sponsors Applying For
- **MagicBlock** — VRF (Verifiable Random Function) for provably fair boss loot drops

## Next on the Roadmap
- Compressed NFTs (cNFTs via Bubblegum) to reduce minting costs at scale
- Session key auto-approval framework to eliminate remaining wallet popups
- Mainnet deployment with real economic incentives
- SOAR leaderboard integration for ranked PvP seasons

## Telegram Handle
xTrinks

---

## Project Description

### What is Alerith?

Alerith is a fully playable, MMORPG built in Unity with Mirror networking. It features tick-based combat for a duel arena while featuring ability style combat for open world, 16 trainable skills, a quest system, crafting, gathering, and a full creature/NPC ecosystem. On top of this complete game foundation, we integrated three custom Solana programs to bring trustless economics to the MMO genre without sacrificing gameplay or adding latency.

The core philosophy: **the game server stays authoritative for game logic** (combat, movement, skills) while **Solana handles economic settlement** (wagering, item ownership, loot provenance). Players get the responsiveness of a traditional MMO with the trustlessness of on-chain verification.

---

### Solana Integration: Three Anchor Programs

We built and deployed three Anchor programs on Solana devnet, each handling a distinct economic pillar of the game:

#### 1. Arena Program — On-Chain PvP Wagering
**Program ID:** `29ZHkMATNJ9kZoeFNhExSiskwk8BL1W6roqaiEQQneYF`

Players can stake real SOL on PvP duels. The flow:
- Both players deposit their wager into an on-chain escrow PDA
- Tick-based combat runs on the Mirror game server
- When the duel ends, the server signs the result with an Ed25519 oracle key and submits a combat log hash
- The winner automatically receives 2x their wager minus a small treasury fee
- A dispute window exists for contested results

This removes the #1 trust problem in competitive gaming: "did the server rig the match?" The combat hash is on-chain and auditable.

#### 2. Marketplace Program — NFT Item Trading
**Program ID:** `Beva7XHsfKZM7zTZUz4dgXqCxfDM3Xc4wVSx9swYWf3F`

In-game items that drop from bosses can be withdrawn to a player's Solana wallet as Metaplex Core NFTs. From there, they can be listed on our on-chain marketplace:
- Sellers create listings with a buyout price in SOL
- Buyers purchase atomically (SOL to seller, NFT to buyer, fee to treasury — single transaction)
- Items can be deposited back into the game inventory at any time
- NFT metadata encodes full item stats: attack power, quality tier, required level, rarity

The "Dark Bank" UI window lets players seamlessly move items between their in-game inventory and their Solana wallet. The "Dark Auction House" provides a full marketplace experience (browse, filter, list, buy).

#### 3. VRF Loot Program — Provably Fair Boss Drops (MagicBlock)
**Program ID:** `ENJmHMGDHpa83QvakPL99hPkY18s3KwvMTPrcHGfAStc`

**This is where MagicBlock comes in.** We integrated MagicBlock's VRF oracle to bring verifiable randomness to boss loot drops:

1. When a boss dies, the server publishes a `VrfRequest` PDA on Solana
2. MagicBlock's VRF oracle fulfills the request with a 32-byte random seed
3. The server feeds this seed into our `SeededRandom` implementation (xoshiro256** PRNG — deterministic, cross-platform)
4. Loot is rolled using the VRF seed against the boss's loot table
5. An immutable `LootReceipt` PDA is published containing: the VRF seed, a SHA256 hash of the loot table, every item rolled, coin amounts, and player assignments

**The key property:** Any third party can download the `LootReceipt`, take the VRF seed, run xoshiro256** locally, and independently verify that the loot the server distributed matches what the randomness dictated. If the server lied about drops, the receipt won't match. This is cryptographic proof of fair loot — something no traditional MMO can offer.

This is how we used **MagicBlock**: their VRF oracle provides the trusted randomness seed that makes the entire verification chain possible. Without an external source of randomness that neither the server nor the player controls, provably fair loot is impossible. MagicBlock's VRF is the anchor (pun intended) of our fairness guarantee.

---

### Architecture

```
[Browser - WebGL Client]
    |
    | Mirror Networking (WebSocket)
    |
[Unity Dedicated Server]
    |                          |
    | HTTP (localhost:3003)    | Mirror RPCs
    |                          |
[Express.js Sidecar]       [Game Logic]
    |                       (Combat, Skills,
    | @coral-xyz/anchor     Quests, Crafting)
    |
[Solana Devnet]
    |-- Arena Program (escrow, settlement)
    |-- Marketplace Program (NFT listings, buyouts)
    |-- VRF Loot Program (seed requests, receipts)
    |
[MagicBlock VRF Oracle]
    |-- Fulfills randomness requests
```

The sidecar pattern keeps Solana transaction construction out of the Unity server (no Rust/JS dependency in C#). The server communicates with the sidecar via HTTP, and the sidecar handles all Anchor instruction building, signing, and submission.

**Authority-operated transactions** let the server sign routine operations (funding matches, claiming winnings) without requiring wallet popups for every action. Players only need Phantom for initial authentication and high-value operations like creating marketplace listings.

---

### Technical Highlights

- **Fully playable MMO**: 16 skills, tick-based combat, quests, crafting, gathering, parties, mounts — this isn't a demo, it's a game
- **WebGL + Phantom**: Runs in the browser with native Phantom wallet integration via JavaScript interop
- **Three specialized programs**: Each Anchor program is independently deployable and composable
- **xoshiro256** PRNG**: Deterministic across C#, Rust, Python, and JS — anyone can verify loot rolls in any language
- **Zero gameplay latency**: Solana transactions happen asynchronously; combat and movement stay server-authoritative at 600ms ticks
- **Immutable audit trail**: Every duel result, marketplace sale, and loot drop is permanently recorded on-chain

---

### Program IDs (Devnet)
| Program | Address |
|---------|---------|
| Arena | `29ZHkMATNJ9kZoeFNhExSiskwk8BL1W6roqaiEQQneYF` |
| Marketplace | `Beva7XHsfKZM7zTZUz4dgXqCxfDM3Xc4wVSx9swYWf3F` |
| VRF Loot | `ENJmHMGDHpa83QvakPL99hPkY18s3KwvMTPrcHGfAStc` |
