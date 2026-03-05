import { PublicKey, SystemProgram, Transaction } from "@solana/web3.js";
import { BN } from "@coral-xyz/anchor";
import { createUmi } from "@metaplex-foundation/umi-bundle-defaults";
import {
  keypairIdentity,
  publicKey,
  generateSigner,
} from "@metaplex-foundation/umi";
import {
  create,
  fetchCollection,
  transferV1,
} from "@metaplex-foundation/mpl-core";
import { config, loadKeypairFromFile } from "../config";
import {
  getMarketplaceProgram,
  getAuthority,
  getTreasury,
  getConnection,
} from "./solana";

// Metaplex Core program ID
const MPL_CORE_PROGRAM_ID = new PublicKey(
  "CoREENxT6tW1HoK8ypY1SxRMZTcVPm7R94rH4PZNhX7d"
);

// ── Umi instance (lazy-initialized) ──

let _umi: ReturnType<typeof createUmi> | null = null;

function getUmi() {
  if (_umi) return _umi;

  const authority = getAuthority();
  const umi = createUmi(config.solanaRpcUrl);

  const umiKeypair = umi.eddsa.createKeypairFromSecretKey(authority.secretKey);
  umi.use(keypairIdentity(umiKeypair));

  _umi = umi;
  return _umi;
}

function getCollectionAddress(): string {
  if (!config.gameNftCollection) {
    throw new Error(
      "Game NFT collection not initialized. Set GAME_NFT_COLLECTION env var."
    );
  }
  return config.gameNftCollection;
}

// ── PDA helpers ──

export function getNftConfigPda(): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("nft_config")],
    config.marketplaceProgramId
  );
}

export function getNftListingPda(listingId: BN): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [
      Buffer.from("nft_listing"),
      listingId.toArrayLike(Buffer, "le", 8),
    ],
    config.marketplaceProgramId
  );
}

function getMarketConfigPda(): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("market_config")],
    config.marketplaceProgramId
  );
}

export function getPlayerEscrowPda(playerWallet: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("player_escrow"), playerWallet.toBuffer()],
    config.marketplaceProgramId
  );
}

// ── Interfaces ──

export interface NftItemMetadata {
  rarity?: string;
  category?: string;
  description?: string;
  dropTimestamp?: number;
}

export interface NftListingInfo {
  listingId: number;
  seller: string;
  asset: string;
  buyoutPrice: number;
  currentBid: number;
  currentBidder: string;
  depositAmount: number;
  createdAt: number;
  expiresAt: number;
  state: string;
  itemId: number;
}

// ── Service functions ──

/**
 * Initialize the NftConfig PDA (one-time setup, requires existing MarketConfig).
 */
export async function initializeNftConfig(): Promise<{
  tx: string;
  nftConfig: string;
}> {
  const program = getMarketplaceProgram();
  const authority = getAuthority();
  const [marketConfigPda] = getMarketConfigPda();
  const [nftConfigPda] = getNftConfigPda();

  const tx = await program.methods
    .initializeNftConfig()
    .accounts({
      marketConfig: marketConfigPda,
      nftConfig: nftConfigPda,
      authority: authority.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  console.log(`[darkAuction] NftConfig initialized: ${nftConfigPda.toBase58()}`);
  return { tx, nftConfig: nftConfigPda.toBase58() };
}

/**
 * Create an NFT listing on the Dark Auction House.
 *
 * Flow:
 * 1. Mint a Metaplex Core NFT with owner = authority
 * 2. Call create_nft_listing_operated which transfers NFT to listing PDA
 */
export async function createNftListing(
  seller: string,
  itemId: number,
  itemName: string,
  vrfRequestId: number,
  metadata: NftItemMetadata,
  buyoutPrice: number,
  durationHours: number
): Promise<{
  tx: string;
  listingId: string;
  listing: string;
  assetAddress: string;
}> {
  const program = getMarketplaceProgram();
  const authority = getAuthority();
  const umi = getUmi();
  const collectionAddress = getCollectionAddress();
  const [marketConfigPda] = getMarketConfigPda();
  const [nftConfigPda] = getNftConfigPda();

  // 1. Fetch current listing count for PDA derivation
  const nftConfig = await (program.account as any).nftConfig.fetch(nftConfigPda);
  const listingId = nftConfig.listingCount as BN;
  const [nftListingPda] = getNftListingPda(listingId);

  // 2. Mint Metaplex Core NFT with owner = authority (will be transferred to listing PDA by instruction)
  const collection = await fetchCollection(
    umi,
    publicKey(collectionAddress)
  );
  const assetSigner = generateSigner(umi);

  await create(umi, {
    asset: assetSigner,
    name: itemName,
    uri: "",
    collection: collection,
    owner: publicKey(authority.publicKey.toBase58()),
    plugins: [
      {
        type: "Attributes",
        attributeList: [
          { key: "itemId", value: String(itemId) },
          { key: "itemName", value: itemName },
          { key: "vrfRequestId", value: String(vrfRequestId) },
          { key: "rarity", value: metadata.rarity || "Common" },
          { key: "category", value: metadata.category || "Misc" },
          {
            key: "dropTimestamp",
            value: String(
              metadata.dropTimestamp || Math.floor(Date.now() / 1000)
            ),
          },
        ],
      },
    ],
  }).sendAndConfirm(umi);

  const assetAddress = assetSigner.publicKey.toString();
  const assetPubkey = new PublicKey(assetAddress);

  console.log(
    `[darkAuction] NFT minted: ${assetAddress} for listing ${listingId.toString()}`
  );

  // 3. Call create_nft_listing_operated
  const tx = await program.methods
    .createNftListingOperated(
      new BN(buyoutPrice),
      durationHours,
      new PublicKey(seller),
      itemId
    )
    .accounts({
      marketConfig: marketConfigPda,
      nftConfig: nftConfigPda,
      nftListing: nftListingPda,
      asset: assetPubkey,
      authority: authority.publicKey,
      collection: new PublicKey(collectionAddress),
      mplCoreProgram: MPL_CORE_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  console.log(
    `[darkAuction] Listing created: id=${listingId.toString()}, asset=${assetAddress}, seller=${seller}`
  );

  return {
    tx,
    listingId: listingId.toString(),
    listing: nftListingPda.toBase58(),
    assetAddress,
  };
}

/**
 * Place a bid on an NFT listing (authority-operated).
 */
export async function placeNftBid(
  listingId: number,
  bidder: string,
  bidAmount: number
): Promise<{ tx: string; newBid: string }> {
  const program = getMarketplaceProgram();
  const authority = getAuthority();
  const listingIdBn = new BN(listingId);
  const [marketConfigPda] = getMarketConfigPda();
  const [nftListingPda] = getNftListingPda(listingIdBn);

  const tx = await program.methods
    .placeNftBidOperated(listingIdBn, new BN(bidAmount), new PublicKey(bidder))
    .accounts({
      marketConfig: marketConfigPda,
      nftListing: nftListingPda,
      authority: authority.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  console.log(
    `[darkAuction] Bid placed: listing=${listingId}, bidder=${bidder}, amount=${bidAmount}`
  );

  return { tx, newBid: String(bidAmount) };
}

/**
 * Buyout an NFT listing (authority-operated).
 * NFT transferred to buyer wallet, SOL distributed.
 */
export async function buyoutNft(
  listingId: number,
  buyer: string
): Promise<{ tx: string; buyerWallet: string }> {
  const program = getMarketplaceProgram();
  const authority = getAuthority();
  const treasury = getTreasury();
  const listingIdBn = new BN(listingId);
  const [marketConfigPda] = getMarketConfigPda();
  const [nftListingPda] = getNftListingPda(listingIdBn);

  // Fetch listing to get asset address
  const listing = await (program.account as any).nftListing.fetch(nftListingPda);
  const assetPubkey = listing.asset as PublicKey;

  const tx = await program.methods
    .buyoutNftOperated(listingIdBn, new PublicKey(buyer))
    .accounts({
      marketConfig: marketConfigPda,
      nftListing: nftListingPda,
      authority: authority.publicKey,
      treasury: treasury,
      buyerWallet: new PublicKey(buyer),
      asset: assetPubkey,
      collection: new PublicKey(getCollectionAddress()),
      mplCoreProgram: MPL_CORE_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  console.log(
    `[darkAuction] Buyout: listing=${listingId}, buyer=${buyer}`
  );

  return { tx, buyerWallet: buyer };
}

/**
 * Cancel an NFT listing (authority-operated, no bids).
 * NFT returned to seller wallet.
 */
export async function cancelNftListing(
  listingId: number
): Promise<{ tx: string }> {
  const program = getMarketplaceProgram();
  const authority = getAuthority();
  const listingIdBn = new BN(listingId);
  const [marketConfigPda] = getMarketConfigPda();
  const [nftListingPda] = getNftListingPda(listingIdBn);

  // Fetch listing to get seller and asset
  const listing = await (program.account as any).nftListing.fetch(nftListingPda);
  const sellerPubkey = listing.seller as PublicKey;
  const assetPubkey = listing.asset as PublicKey;

  const tx = await program.methods
    .cancelNftListingOperated(listingIdBn)
    .accounts({
      marketConfig: marketConfigPda,
      nftListing: nftListingPda,
      authority: authority.publicKey,
      sellerWallet: sellerPubkey,
      asset: assetPubkey,
      collection: new PublicKey(getCollectionAddress()),
      mplCoreProgram: MPL_CORE_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  console.log(`[darkAuction] Listing cancelled: ${listingId}`);

  return { tx };
}

/**
 * Claim an expired NFT listing.
 * If bids: NFT → highest bidder, SOL → authority. If no bids: NFT → seller.
 */
export async function claimNftExpired(
  listingId: number
): Promise<{ tx: string; recipientWallet: string }> {
  const program = getMarketplaceProgram();
  const authority = getAuthority();
  const treasury = getTreasury();
  const listingIdBn = new BN(listingId);
  const [marketConfigPda] = getMarketConfigPda();
  const [nftListingPda] = getNftListingPda(listingIdBn);

  // Fetch listing to determine recipient
  const listing = await (program.account as any).nftListing.fetch(nftListingPda);
  const sellerPubkey = listing.seller as PublicKey;
  const currentBidder = listing.currentBidder as PublicKey;
  const hasBids = !currentBidder.equals(PublicKey.default);
  const nftRecipient = hasBids ? currentBidder : sellerPubkey;
  const assetPubkey = listing.asset as PublicKey;

  const tx = await program.methods
    .claimNftExpired(listingIdBn)
    .accounts({
      marketConfig: marketConfigPda,
      nftListing: nftListingPda,
      authority: authority.publicKey,
      treasury: treasury,
      nftRecipient: nftRecipient,
      asset: assetPubkey,
      collection: new PublicKey(getCollectionAddress()),
      mplCoreProgram: MPL_CORE_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  const recipientWallet = nftRecipient.toBase58();
  console.log(
    `[darkAuction] Expired listing claimed: ${listingId}, recipient=${recipientWallet}`
  );

  return { tx, recipientWallet };
}

// ── Query functions ──

function parseListingState(state: any): string {
  if (state.active !== undefined) return "active";
  if (state.sold !== undefined) return "sold";
  if (state.cancelled !== undefined) return "cancelled";
  if (state.expired !== undefined) return "expired";
  return "unknown";
}

function rawListingToInfo(listing: any): NftListingInfo {
  return {
    listingId: (listing.listingId as BN).toNumber(),
    seller: (listing.seller as PublicKey).toBase58(),
    asset: (listing.asset as PublicKey).toBase58(),
    buyoutPrice: (listing.buyoutPrice as BN).toNumber(),
    currentBid: (listing.currentBid as BN).toNumber(),
    currentBidder: (listing.currentBidder as PublicKey).toBase58(),
    depositAmount: (listing.depositAmount as BN).toNumber(),
    createdAt: (listing.createdAt as BN).toNumber(),
    expiresAt: (listing.expiresAt as BN).toNumber(),
    state: parseListingState(listing.state),
    itemId: listing.itemId,
  };
}

/**
 * Fetch a single NFT listing by ID.
 */
export async function getNftListing(
  listingId: number
): Promise<NftListingInfo | null> {
  const program = getMarketplaceProgram();
  const listingIdBn = new BN(listingId);
  const [nftListingPda] = getNftListingPda(listingIdBn);

  try {
    const listing = await (program.account as any).nftListing.fetch(
      nftListingPda
    );
    return rawListingToInfo(listing);
  } catch {
    return null;
  }
}

/**
 * Fetch all active NFT listings (state=Active and not expired).
 */
export async function getActiveNftListings(): Promise<NftListingInfo[]> {
  const program = getMarketplaceProgram();
  const now = Math.floor(Date.now() / 1000);

  try {
    const allListings = await (program.account as any).nftListing.all();
    const results: NftListingInfo[] = [];

    for (const { account } of allListings) {
      const state = parseListingState(account.state);
      if (state !== "active") continue;

      const expiresAt = (account.expiresAt as BN).toNumber();
      if (expiresAt <= now) continue;

      results.push(rawListingToInfo(account));
    }

    console.log(`[darkAuction] Active listings: ${results.length}`);
    return results;
  } catch (err: any) {
    console.error(`[darkAuction] getActiveNftListings error: ${err.message}`);
    return [];
  }
}

/**
 * Fetch NFT listings for a specific seller wallet.
 */
export async function getMyNftListings(
  sellerWallet: string
): Promise<NftListingInfo[]> {
  const program = getMarketplaceProgram();

  try {
    const allListings = await (program.account as any).nftListing.all();
    const results: NftListingInfo[] = [];

    for (const { account } of allListings) {
      const seller = (account.seller as PublicKey).toBase58();
      if (seller !== sellerWallet) continue;
      results.push(rawListingToInfo(account));
    }

    console.log(
      `[darkAuction] My listings for ${sellerWallet}: ${results.length}`
    );
    return results;
  } catch (err: any) {
    console.error(`[darkAuction] getMyNftListings error: ${err.message}`);
    return [];
  }
}

/**
 * Fetch the NftConfig state.
 */
export async function getNftConfig(): Promise<{
  listingCount: string;
} | null> {
  const program = getMarketplaceProgram();
  const [nftConfigPda] = getNftConfigPda();

  try {
    const cfg = await (program.account as any).nftConfig.fetch(nftConfigPda);
    return {
      listingCount: (cfg.listingCount as BN).toString(),
    };
  } catch {
    return null;
  }
}

// ── Escrow functions ──

/**
 * Build a deposit_escrow transaction for the player to sign client-side.
 * Returns the serialized unsigned transaction as base64.
 */
export async function buildDepositEscrowTx(
  playerWallet: string,
  amount: number
): Promise<{ transaction: string; escrowPda: string }> {
  const program = getMarketplaceProgram();
  const connection = getConnection();
  const playerPubkey = new PublicKey(playerWallet);
  const [escrowPda] = getPlayerEscrowPda(playerPubkey);

  const ix = await program.methods
    .depositEscrow(new BN(amount))
    .accounts({
      playerEscrow: escrowPda,
      player: playerPubkey,
      systemProgram: SystemProgram.programId,
    })
    .instruction();

  const tx = new Transaction().add(ix);
  tx.feePayer = playerPubkey;
  tx.recentBlockhash = (await connection.getLatestBlockhash()).blockhash;

  const serialized = tx
    .serialize({ requireAllSignatures: false })
    .toString("base64");

  console.log(
    `[darkAuction] Built deposit escrow tx: player=${playerWallet}, amount=${amount}`
  );

  return { transaction: serialized, escrowPda: escrowPda.toBase58() };
}

/**
 * Build a withdraw_escrow transaction for the player to sign client-side.
 * Returns the serialized unsigned transaction as base64.
 */
export async function buildWithdrawEscrowTx(
  playerWallet: string,
  amount: number
): Promise<{ transaction: string; escrowPda: string }> {
  const program = getMarketplaceProgram();
  const connection = getConnection();
  const playerPubkey = new PublicKey(playerWallet);
  const [escrowPda] = getPlayerEscrowPda(playerPubkey);

  const ix = await program.methods
    .withdrawEscrow(new BN(amount))
    .accounts({
      playerEscrow: escrowPda,
      player: playerPubkey,
      systemProgram: SystemProgram.programId,
    })
    .instruction();

  const tx = new Transaction().add(ix);
  tx.feePayer = playerPubkey;
  tx.recentBlockhash = (await connection.getLatestBlockhash()).blockhash;

  const serialized = tx
    .serialize({ requireAllSignatures: false })
    .toString("base64");

  console.log(
    `[darkAuction] Built withdraw escrow tx: player=${playerWallet}, amount=${amount}`
  );

  return { transaction: serialized, escrowPda: escrowPda.toBase58() };
}

/**
 * Fetch a player's escrow balance.
 * Returns 0 if the escrow account does not exist yet.
 */
export async function getEscrowBalance(
  playerWallet: string
): Promise<{ balance: number }> {
  const program = getMarketplaceProgram();
  const playerPubkey = new PublicKey(playerWallet);
  const [escrowPda] = getPlayerEscrowPda(playerPubkey);

  try {
    const account = await (program.account as any).playerEscrow.fetch(escrowPda);
    return { balance: (account.balance as BN).toNumber() };
  } catch {
    // Account doesn't exist yet — balance is zero
    return { balance: 0 };
  }
}

/**
 * Place an NFT bid using the bidder's escrow balance (authority-operated).
 * The on-chain program debits the bidder's escrow and credits the previous bidder's escrow.
 */
export async function placeNftBidFromEscrow(
  characterId: number,
  listingId: number,
  bidderWallet: string,
  bidAmount: number
): Promise<{ tx: string; newBid: string }> {
  const program = getMarketplaceProgram();
  const authority = getAuthority();
  const listingIdBn = new BN(listingId);
  const [marketConfigPda] = getMarketConfigPda();
  const [nftListingPda] = getNftListingPda(listingIdBn);

  const bidderPubkey = new PublicKey(bidderWallet);
  const [bidderEscrowPda] = getPlayerEscrowPda(bidderPubkey);

  // Fetch listing to get the previous bidder
  const listing = await (program.account as any).nftListing.fetch(nftListingPda);
  const currentBidder = listing.currentBidder as PublicKey;
  const hasPreviousBidder = !currentBidder.equals(PublicKey.default);

  // Derive prev bidder escrow PDA (use bidder's own if no previous bidder)
  const [prevBidderEscrowPda] = hasPreviousBidder
    ? getPlayerEscrowPda(currentBidder)
    : getPlayerEscrowPda(bidderPubkey);

  const tx = await program.methods
    .placeNftBidEscrow(listingIdBn, new BN(bidAmount), bidderPubkey)
    .accounts({
      marketConfig: marketConfigPda,
      nftListing: nftListingPda,
      authority: authority.publicKey,
      bidderEscrow: bidderEscrowPda,
      prevBidderEscrow: prevBidderEscrowPda,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  console.log(
    `[darkAuction] Escrow bid placed: listing=${listingId}, bidder=${bidderWallet}, amount=${bidAmount}, char=${characterId}`
  );

  return { tx, newBid: String(bidAmount) };
}

/**
 * Buyout an NFT listing using the buyer's escrow balance (authority-operated).
 * NFT transferred to buyer wallet, SOL distributed from escrow.
 */
export async function buyoutNftFromEscrow(
  characterId: number,
  listingId: number,
  buyerWallet: string
): Promise<{ tx: string; buyerWallet: string }> {
  const program = getMarketplaceProgram();
  const authority = getAuthority();
  const treasury = getTreasury();
  const listingIdBn = new BN(listingId);
  const [marketConfigPda] = getMarketConfigPda();
  const [nftListingPda] = getNftListingPda(listingIdBn);

  const buyerPubkey = new PublicKey(buyerWallet);
  const [buyerEscrowPda] = getPlayerEscrowPda(buyerPubkey);

  // Fetch listing to get asset and collection
  const listing = await (program.account as any).nftListing.fetch(nftListingPda);
  const assetPubkey = listing.asset as PublicKey;

  const tx = await program.methods
    .buyoutNftEscrow(listingIdBn, buyerPubkey)
    .accounts({
      marketConfig: marketConfigPda,
      nftListing: nftListingPda,
      authority: authority.publicKey,
      treasury: treasury,
      buyerEscrow: buyerEscrowPda,
      buyerWallet: buyerPubkey,
      asset: assetPubkey,
      collection: new PublicKey(getCollectionAddress()),
      mplCoreProgram: MPL_CORE_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  console.log(
    `[darkAuction] Escrow buyout: listing=${listingId}, buyer=${buyerWallet}, char=${characterId}`
  );

  return { tx, buyerWallet };
}
