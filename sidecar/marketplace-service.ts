import {
  PublicKey,
  SystemProgram,
  Keypair,
  SYSVAR_RENT_PUBKEY,
} from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddress,
} from "@solana/spl-token";
import { BN } from "@coral-xyz/anchor";
import { config } from "../config";
import {
  getMarketplaceProgram,
  getAuthority,
  getTreasury,
  getConnection,
} from "./solana";
import { createHash } from "crypto";

// ── PDA helpers ──

function getMarketConfigPda(): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("market_config")],
    config.marketplaceProgramId
  );
}

function getRegisteredItemPda(itemId: number): [PublicKey, number] {
  const buf = Buffer.alloc(4);
  buf.writeUInt32LE(itemId);
  return PublicKey.findProgramAddressSync(
    [Buffer.from("registered_item"), buf],
    config.marketplaceProgramId
  );
}

function getItemMintPda(itemId: number): [PublicKey, number] {
  const buf = Buffer.alloc(4);
  buf.writeUInt32LE(itemId);
  return PublicKey.findProgramAddressSync(
    [Buffer.from("item_mint"), buf],
    config.marketplaceProgramId
  );
}

function getListingPda(listingId: BN): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("listing"), listingId.toArrayLike(Buffer, "le", 8)],
    config.marketplaceProgramId
  );
}

// ── Service functions ──

export async function initialize(
  listingFeeBps: number,
  saleFeeBps: number
): Promise<{ tx: string; marketConfig: string }> {
  const program = getMarketplaceProgram();
  const authority = getAuthority();
  const treasury = getTreasury();
  const [marketConfigPda] = getMarketConfigPda();

  const tx = await program.methods
    .initialize(treasury, listingFeeBps, saleFeeBps)
    .accounts({
      marketConfig: marketConfigPda,
      authority: authority.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  return { tx, marketConfig: marketConfigPda.toBase58() };
}

export async function registerItem(
  itemId: number,
  itemName: string,
  isTradeable: boolean
): Promise<{ tx: string; registeredItem: string; itemMint: string }> {
  const program = getMarketplaceProgram();
  const authority = getAuthority();
  const [marketConfigPda] = getMarketConfigPda();
  const [registeredItemPda] = getRegisteredItemPda(itemId);
  const [itemMintPda] = getItemMintPda(itemId);

  const nameHash = Array.from(
    createHash("sha256").update(itemName).digest()
  );

  const tx = await program.methods
    .registerItem(itemId, nameHash, isTradeable)
    .accounts({
      marketConfig: marketConfigPda,
      registeredItem: registeredItemPda,
      itemMint: itemMintPda,
      authority: authority.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
      rent: SYSVAR_RENT_PUBKEY,
    })
    .signers([authority])
    .rpc();

  return {
    tx,
    registeredItem: registeredItemPda.toBase58(),
    itemMint: itemMintPda.toBase58(),
  };
}

export async function mintItem(
  itemId: number,
  recipient: string,
  amount: number
): Promise<{ tx: string }> {
  const program = getMarketplaceProgram();
  const authority = getAuthority();
  const [marketConfigPda] = getMarketConfigPda();
  const [itemMintPda] = getItemMintPda(itemId);
  const recipientPk = new PublicKey(recipient);
  const recipientAta = await getAssociatedTokenAddress(itemMintPda, recipientPk);

  const tx = await program.methods
    .mintItem(itemId, new BN(amount))
    .accounts({
      marketConfig: marketConfigPda,
      itemMint: itemMintPda,
      recipient: recipientPk,
      recipientAta: recipientAta,
      authority: authority.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  return { tx };
}

export async function burnItem(
  ownerKeypair: Keypair,
  itemId: number,
  amount: number
): Promise<{ tx: string }> {
  const program = getMarketplaceProgram();
  const [itemMintPda] = getItemMintPda(itemId);
  const ownerAta = await getAssociatedTokenAddress(itemMintPda, ownerKeypair.publicKey);

  const tx = await program.methods
    .burnItem(itemId, new BN(amount))
    .accounts({
      itemMint: itemMintPda,
      ownerAta: ownerAta,
      owner: ownerKeypair.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
    })
    .signers([ownerKeypair])
    .rpc();

  return { tx };
}

export async function createListing(
  sellerKeypair: Keypair,
  itemId: number,
  buyoutPrice: number,
  durationHours: number
): Promise<{ tx: string; listingId: string; listing: string }> {
  const program = getMarketplaceProgram();
  const [marketConfigPda] = getMarketConfigPda();
  const [registeredItemPda] = getRegisteredItemPda(itemId);
  const [itemMintPda] = getItemMintPda(itemId);

  // Get current listing count for the next listing ID
  const configAcct = await (program.account as any).marketConfig.fetch(marketConfigPda);
  const listingId = configAcct.listingCount as BN;
  const [listingPda] = getListingPda(listingId);

  const sellerAta = await getAssociatedTokenAddress(itemMintPda, sellerKeypair.publicKey);
  const escrowAta = await getAssociatedTokenAddress(itemMintPda, listingPda, true);

  const tx = await program.methods
    .createListing(new BN(buyoutPrice), durationHours)
    .accounts({
      marketConfig: marketConfigPda,
      registeredItem: registeredItemPda,
      itemMint: itemMintPda,
      listing: listingPda,
      sellerAta: sellerAta,
      escrowAta: escrowAta,
      seller: sellerKeypair.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .signers([sellerKeypair])
    .rpc();

  return {
    tx,
    listingId: listingId.toString(),
    listing: listingPda.toBase58(),
  };
}

export async function placeBid(
  bidderKeypair: Keypair,
  listingId: number,
  bidAmount: number
): Promise<{ tx: string }> {
  const program = getMarketplaceProgram();
  const listingIdBn = new BN(listingId);
  const [marketConfigPda] = getMarketConfigPda();
  const [listingPda] = getListingPda(listingIdBn);

  // Fetch listing to get previous bidder
  const listing = await (program.account as any).listing.fetch(listingPda);
  const previousBidder = listing.currentBidder as PublicKey;

  const tx = await program.methods
    .placeBid(listingIdBn, new BN(bidAmount))
    .accounts({
      marketConfig: marketConfigPda,
      listing: listingPda,
      bidder: bidderKeypair.publicKey,
      previousBidder: previousBidder,
      systemProgram: SystemProgram.programId,
    })
    .signers([bidderKeypair])
    .rpc();

  return { tx };
}

export async function buyout(
  buyerKeypair: Keypair,
  listingId: number
): Promise<{ tx: string }> {
  const program = getMarketplaceProgram();
  const listingIdBn = new BN(listingId);
  const [marketConfigPda] = getMarketConfigPda();
  const [listingPda] = getListingPda(listingIdBn);
  const treasury = getTreasury();

  // Fetch listing to get seller, previous bidder, item mint
  const listing = await (program.account as any).listing.fetch(listingPda);
  const seller = listing.seller as PublicKey;
  const previousBidder = listing.currentBidder as PublicKey;
  const itemMint = listing.itemMint as PublicKey;

  const escrowAta = await getAssociatedTokenAddress(itemMint, listingPda, true);
  const buyerAta = await getAssociatedTokenAddress(itemMint, buyerKeypair.publicKey);

  const tx = await program.methods
    .buyout(listingIdBn)
    .accounts({
      marketConfig: marketConfigPda,
      listing: listingPda,
      buyer: buyerKeypair.publicKey,
      seller: seller,
      treasury: treasury,
      previousBidder: previousBidder,
      escrowAta: escrowAta,
      buyerAta: buyerAta,
      itemMint: itemMint,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .signers([buyerKeypair])
    .rpc();

  return { tx };
}

export async function cancelListing(
  sellerKeypair: Keypair,
  listingId: number
): Promise<{ tx: string }> {
  const program = getMarketplaceProgram();
  const listingIdBn = new BN(listingId);
  const [listingPda] = getListingPda(listingIdBn);

  const listing = await (program.account as any).listing.fetch(listingPda);
  const itemMint = listing.itemMint as PublicKey;
  const escrowAta = await getAssociatedTokenAddress(itemMint, listingPda, true);
  const sellerAta = await getAssociatedTokenAddress(itemMint, sellerKeypair.publicKey);

  const tx = await program.methods
    .cancelListing(listingIdBn)
    .accounts({
      listing: listingPda,
      escrowAta: escrowAta,
      sellerAta: sellerAta,
      itemMint: itemMint,
      seller: sellerKeypair.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .signers([sellerKeypair])
    .rpc();

  return { tx };
}

// ── Authority-operated functions ──

export async function createListingOperated(
  itemId: number,
  buyoutPrice: number,
  durationHours: number,
  seller: string,
  itemAmount: number
): Promise<{ tx: string; listingId: string; listing: string }> {
  const program = getMarketplaceProgram();
  const authority = getAuthority();
  const [marketConfigPda] = getMarketConfigPda();
  const [registeredItemPda] = getRegisteredItemPda(itemId);
  const [itemMintPda] = getItemMintPda(itemId);

  const configAcct = await (program.account as any).marketConfig.fetch(marketConfigPda);
  const listingId = configAcct.listingCount as BN;
  const [listingPda] = getListingPda(listingId);

  const escrowAta = await getAssociatedTokenAddress(itemMintPda, listingPda, true);

  const tx = await program.methods
    .createListingOperated(
      new BN(buyoutPrice),
      durationHours,
      new PublicKey(seller),
      new BN(itemAmount)
    )
    .accounts({
      marketConfig: marketConfigPda,
      registeredItem: registeredItemPda,
      itemMint: itemMintPda,
      listing: listingPda,
      escrowAta: escrowAta,
      authority: authority.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  return {
    tx,
    listingId: listingId.toString(),
    listing: listingPda.toBase58(),
  };
}

export async function placeBidOperated(
  listingId: number,
  bidAmount: number,
  bidder: string
): Promise<{ tx: string }> {
  const program = getMarketplaceProgram();
  const authority = getAuthority();
  const listingIdBn = new BN(listingId);
  const [marketConfigPda] = getMarketConfigPda();
  const [listingPda] = getListingPda(listingIdBn);

  const tx = await program.methods
    .placeBidOperated(listingIdBn, new BN(bidAmount), new PublicKey(bidder))
    .accounts({
      marketConfig: marketConfigPda,
      listing: listingPda,
      authority: authority.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  return { tx };
}

export async function buyoutOperated(
  listingId: number,
  buyer: string
): Promise<{ tx: string }> {
  const program = getMarketplaceProgram();
  const authority = getAuthority();
  const listingIdBn = new BN(listingId);
  const [marketConfigPda] = getMarketConfigPda();
  const [listingPda] = getListingPda(listingIdBn);
  const treasury = getTreasury();

  const listing = await (program.account as any).listing.fetch(listingPda);
  const itemMint = listing.itemMint as PublicKey;
  const escrowAta = await getAssociatedTokenAddress(itemMint, listingPda, true);

  const tx = await program.methods
    .buyoutOperated(listingIdBn, new PublicKey(buyer))
    .accounts({
      marketConfig: marketConfigPda,
      listing: listingPda,
      authority: authority.publicKey,
      treasury: treasury,
      escrowAta: escrowAta,
      itemMint: itemMint,
      tokenProgram: TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  return { tx };
}

export async function cancelListingOperated(
  listingId: number
): Promise<{ tx: string }> {
  const program = getMarketplaceProgram();
  const authority = getAuthority();
  const listingIdBn = new BN(listingId);
  const [marketConfigPda] = getMarketConfigPda();
  const [listingPda] = getListingPda(listingIdBn);

  const listing = await (program.account as any).listing.fetch(listingPda);
  const itemMint = listing.itemMint as PublicKey;
  const escrowAta = await getAssociatedTokenAddress(itemMint, listingPda, true);

  const tx = await program.methods
    .cancelListingOperated(listingIdBn)
    .accounts({
      marketConfig: marketConfigPda,
      listing: listingPda,
      escrowAta: escrowAta,
      itemMint: itemMint,
      authority: authority.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  return { tx };
}

export async function claimExpired(
  payerKeypair: Keypair,
  listingId: number
): Promise<{ tx: string }> {
  const program = getMarketplaceProgram();
  const listingIdBn = new BN(listingId);
  const [marketConfigPda] = getMarketConfigPda();
  const [listingPda] = getListingPda(listingIdBn);
  const treasury = getTreasury();

  const listing = await (program.account as any).listing.fetch(listingPda);
  const seller = listing.seller as PublicKey;
  const itemMint = listing.itemMint as PublicKey;
  const currentBidder = listing.currentBidder as PublicKey;
  const hasBids = (listing.currentBid as BN).toNumber() > 0;

  // If there are bids, items go to bidder; otherwise back to seller
  const itemRecipient = hasBids ? currentBidder : seller;
  const escrowAta = await getAssociatedTokenAddress(itemMint, listingPda, true);
  const bidderOrSellerAta = await getAssociatedTokenAddress(itemMint, itemRecipient);

  const tx = await program.methods
    .claimExpired(listingIdBn)
    .accounts({
      marketConfig: marketConfigPda,
      listing: listingPda,
      seller: seller,
      treasury: treasury,
      escrowAta: escrowAta,
      bidderOrSellerAta: bidderOrSellerAta,
      itemRecipient: itemRecipient,
      itemMint: itemMint,
      payer: payerKeypair.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .signers([payerKeypair])
    .rpc();

  return { tx };
}

export async function getListing(listingId: number): Promise<any> {
  const program = getMarketplaceProgram();
  const listingIdBn = new BN(listingId);
  const [listingPda] = getListingPda(listingIdBn);

  try {
    const listing = await (program.account as any).listing.fetch(listingPda);
    return {
      listingId: (listing.listingId as BN).toString(),
      seller: (listing.seller as PublicKey).toBase58(),
      itemMint: (listing.itemMint as PublicKey).toBase58(),
      itemAmount: (listing.itemAmount as BN).toString(),
      buyoutPrice: (listing.buyoutPrice as BN).toString(),
      currentBid: (listing.currentBid as BN).toString(),
      currentBidder: (listing.currentBidder as PublicKey).toBase58(),
      depositAmount: (listing.depositAmount as BN).toString(),
      createdAt: (listing.createdAt as BN).toString(),
      expiresAt: (listing.expiresAt as BN).toString(),
      state: listing.state,
    };
  } catch {
    return null;
  }
}

export async function getRegisteredItem(itemId: number): Promise<any> {
  const program = getMarketplaceProgram();
  const [registeredItemPda] = getRegisteredItemPda(itemId);

  try {
    const item = await (program.account as any).registeredItem.fetch(registeredItemPda);
    return {
      itemId: item.itemId,
      mint: (item.mint as PublicKey).toBase58(),
      isTradeable: item.isTradeable,
      nameHash: Array.from(item.nameHash as number[]),
    };
  } catch {
    return null;
  }
}

export async function getMarketConfig(): Promise<any> {
  const program = getMarketplaceProgram();
  const [marketConfigPda] = getMarketConfigPda();

  try {
    const cfg = await (program.account as any).marketConfig.fetch(marketConfigPda);
    return {
      authority: (cfg.authority as PublicKey).toBase58(),
      treasury: (cfg.treasury as PublicKey).toBase58(),
      listingFeeBps: cfg.listingFeeBps,
      saleFeeBps: cfg.saleFeeBps,
      listingCount: (cfg.listingCount as BN).toString(),
      paused: cfg.paused,
    };
  } catch {
    return null;
  }
}
