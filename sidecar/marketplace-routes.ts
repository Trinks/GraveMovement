import { Router, Request, Response } from "express";
import { Keypair } from "@solana/web3.js";
import { requireInternalSecret } from "../middleware/auth";
import * as marketplaceService from "../services/marketplace";

const router = Router();
router.use(requireInternalSecret);

// POST /api/v1/marketplace/initialize
router.post("/initialize", async (req: Request, res: Response) => {
  try {
    const { listingFeeBps = 200, saleFeeBps = 200 } = req.body;
    const result = await marketplaceService.initialize(listingFeeBps, saleFeeBps);
    res.json({ success: true, ...result });
  } catch (err: any) {
    console.error("[marketplace] initialize error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/marketplace/register-item
// Body: { itemId: number, itemName: string, isTradeable: boolean }
router.post("/register-item", async (req: Request, res: Response) => {
  try {
    const { itemId, itemName, isTradeable = true } = req.body;
    const result = await marketplaceService.registerItem(itemId, itemName, isTradeable);
    res.json({ success: true, ...result });
  } catch (err: any) {
    console.error("[marketplace] register-item error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/marketplace/mint-item
// Body: { itemId: number, recipient: string (pubkey), amount: number }
router.post("/mint-item", async (req: Request, res: Response) => {
  try {
    const { itemId, recipient, amount } = req.body;
    const result = await marketplaceService.mintItem(itemId, recipient, amount);
    res.json({ success: true, ...result });
  } catch (err: any) {
    console.error("[marketplace] mint-item error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/marketplace/burn-item
// Body: { ownerSecretKey: number[], itemId: number, amount: number }
router.post("/burn-item", async (req: Request, res: Response) => {
  try {
    const { ownerSecretKey, itemId, amount } = req.body;
    const ownerKeypair = Keypair.fromSecretKey(Uint8Array.from(ownerSecretKey));
    const result = await marketplaceService.burnItem(ownerKeypair, itemId, amount);
    res.json({ success: true, ...result });
  } catch (err: any) {
    console.error("[marketplace] burn-item error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/marketplace/create-listing
// Body: { sellerSecretKey?: number[], itemId: number, buyoutPrice: number, durationHours: number, useAuthority?: boolean, seller?: string, itemAmount?: number }
router.post("/create-listing", async (req: Request, res: Response) => {
  try {
    const { sellerSecretKey, itemId, buyoutPrice, durationHours = 24, useAuthority, seller, itemAmount = 1 } = req.body;
    if (useAuthority) {
      const sellerPubkey = seller || "11111111111111111111111111111111";
      const result = await marketplaceService.createListingOperated(
        itemId, buyoutPrice, durationHours, sellerPubkey, itemAmount
      );
      res.json({ success: true, ...result });
    } else {
      const sellerKeypair = Keypair.fromSecretKey(Uint8Array.from(sellerSecretKey));
      const result = await marketplaceService.createListing(
        sellerKeypair, itemId, buyoutPrice, durationHours
      );
      res.json({ success: true, ...result });
    }
  } catch (err: any) {
    console.error("[marketplace] create-listing error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/marketplace/place-bid
// Body: { bidderSecretKey?: number[], listingId: number, bidAmount: number, useAuthority?: boolean, bidder?: string }
router.post("/place-bid", async (req: Request, res: Response) => {
  try {
    const { bidderSecretKey, listingId, bidAmount, useAuthority, bidder } = req.body;
    if (useAuthority) {
      const bidderPubkey = bidder || "11111111111111111111111111111111";
      const result = await marketplaceService.placeBidOperated(listingId, bidAmount, bidderPubkey);
      res.json({ success: true, ...result });
    } else {
      const bidderKeypair = Keypair.fromSecretKey(Uint8Array.from(bidderSecretKey));
      const result = await marketplaceService.placeBid(bidderKeypair, listingId, bidAmount);
      res.json({ success: true, ...result });
    }
  } catch (err: any) {
    console.error("[marketplace] place-bid error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/marketplace/buyout
// Body: { buyerSecretKey?: number[], listingId: number, useAuthority?: boolean, buyer?: string }
router.post("/buyout", async (req: Request, res: Response) => {
  try {
    const { buyerSecretKey, listingId, useAuthority, buyer } = req.body;
    if (useAuthority) {
      const buyerPubkey = buyer || "11111111111111111111111111111111";
      const result = await marketplaceService.buyoutOperated(listingId, buyerPubkey);
      res.json({ success: true, ...result });
    } else {
      const buyerKeypair = Keypair.fromSecretKey(Uint8Array.from(buyerSecretKey));
      const result = await marketplaceService.buyout(buyerKeypair, listingId);
      res.json({ success: true, ...result });
    }
  } catch (err: any) {
    console.error("[marketplace] buyout error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/marketplace/cancel-listing
// Body: { sellerSecretKey?: number[], listingId: number, useAuthority?: boolean }
router.post("/cancel-listing", async (req: Request, res: Response) => {
  try {
    const { sellerSecretKey, listingId, useAuthority } = req.body;
    if (useAuthority) {
      const result = await marketplaceService.cancelListingOperated(listingId);
      res.json({ success: true, ...result });
    } else {
      const sellerKeypair = Keypair.fromSecretKey(Uint8Array.from(sellerSecretKey));
      const result = await marketplaceService.cancelListing(sellerKeypair, listingId);
      res.json({ success: true, ...result });
    }
  } catch (err: any) {
    console.error("[marketplace] cancel-listing error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/marketplace/claim-expired
// Body: { payerSecretKey?: number[], listingId: number }
router.post("/claim-expired", async (req: Request, res: Response) => {
  try {
    const { payerSecretKey, listingId } = req.body;
    // If no payer provided, use authority
    const { getAuthority } = await import("../services/solana");
    const payerKeypair = payerSecretKey
      ? Keypair.fromSecretKey(Uint8Array.from(payerSecretKey))
      : getAuthority();
    const result = await marketplaceService.claimExpired(payerKeypair, listingId);
    res.json({ success: true, ...result });
  } catch (err: any) {
    console.error("[marketplace] claim-expired error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// GET /api/v1/marketplace/listing/:listingId
router.get("/listing/:listingId", async (req: Request, res: Response) => {
  try {
    const listingId = parseInt(req.params.listingId, 10);
    const listing = await marketplaceService.getListing(listingId);
    if (!listing) {
      res.status(404).json({ success: false, error: "Listing not found" });
      return;
    }
    res.json({ success: true, listing });
  } catch (err: any) {
    console.error("[marketplace] get-listing error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// GET /api/v1/marketplace/item/:itemId
router.get("/item/:itemId", async (req: Request, res: Response) => {
  try {
    const itemId = parseInt(req.params.itemId, 10);
    const item = await marketplaceService.getRegisteredItem(itemId);
    if (!item) {
      res.status(404).json({ success: false, error: "Item not found" });
      return;
    }
    res.json({ success: true, item });
  } catch (err: any) {
    console.error("[marketplace] get-item error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// GET /api/v1/marketplace/config
router.get("/config", async (_req: Request, res: Response) => {
  try {
    const cfg = await marketplaceService.getMarketConfig();
    if (!cfg) {
      res.status(404).json({ success: false, error: "Marketplace not initialized" });
      return;
    }
    res.json({ success: true, config: cfg });
  } catch (err: any) {
    console.error("[marketplace] get-config error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

export default router;
