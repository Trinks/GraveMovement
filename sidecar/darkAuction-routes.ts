import { Router, Request, Response } from "express";
import { requireInternalSecret } from "../middleware/auth";
import * as darkAuctionService from "../services/darkAuction";

const router = Router();
router.use(requireInternalSecret);

// POST /api/v1/dark-auction/initialize
router.post("/initialize", async (_req: Request, res: Response) => {
  try {
    const result = await darkAuctionService.initializeNftConfig();
    res.json({ success: true, ...result });
  } catch (err: any) {
    console.error("[darkAuction] initialize error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/dark-auction/create-listing
// Body: { seller, itemId, itemName, vrfRequestId, metadata, buyoutPrice, durationHours }
router.post("/create-listing", async (req: Request, res: Response) => {
  try {
    const {
      seller,
      itemId,
      itemName,
      vrfRequestId,
      metadata = {},
      buyoutPrice,
      durationHours = 24,
    } = req.body;

    if (!seller || !itemId || !itemName || !buyoutPrice) {
      res.status(400).json({
        success: false,
        error: "Missing required fields: seller, itemId, itemName, buyoutPrice",
      });
      return;
    }

    const result = await darkAuctionService.createNftListing(
      seller,
      itemId,
      itemName,
      vrfRequestId || 0,
      metadata,
      buyoutPrice,
      durationHours
    );

    res.json({
      success: true,
      listingId: parseInt(result.listingId),
      assetAddress: result.assetAddress,
      tx: result.tx,
    });
  } catch (err: any) {
    console.error("[darkAuction] create-listing error:", err.message);
    let errorMsg = err.message;
    if (errorMsg.includes("insufficient lamports")) {
      errorMsg =
        "Authority wallet has insufficient SOL for the listing deposit. Try a lower buyout price.";
    }
    res.status(500).json({ success: false, error: errorMsg });
  }
});

// POST /api/v1/dark-auction/place-bid
// Body: { listingId, bidder, bidAmount }
router.post("/place-bid", async (req: Request, res: Response) => {
  try {
    const { listingId, bidder, bidAmount } = req.body;

    if (listingId === undefined || !bidder || !bidAmount) {
      res.status(400).json({
        success: false,
        error: "Missing required fields: listingId, bidder, bidAmount",
      });
      return;
    }

    const result = await darkAuctionService.placeNftBid(
      listingId,
      bidder,
      bidAmount
    );

    res.json({
      success: true,
      newBid: parseInt(result.newBid),
      tx: result.tx,
    });
  } catch (err: any) {
    console.error("[darkAuction] place-bid error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/dark-auction/buyout
// Body: { listingId, buyer }
router.post("/buyout", async (req: Request, res: Response) => {
  try {
    const { listingId, buyer } = req.body;

    if (listingId === undefined || !buyer) {
      res.status(400).json({
        success: false,
        error: "Missing required fields: listingId, buyer",
      });
      return;
    }

    const result = await darkAuctionService.buyoutNft(listingId, buyer);

    res.json({
      success: true,
      buyerWallet: result.buyerWallet,
      tx: result.tx,
    });
  } catch (err: any) {
    console.error("[darkAuction] buyout error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/dark-auction/cancel-listing
// Body: { listingId }
router.post("/cancel-listing", async (req: Request, res: Response) => {
  try {
    const { listingId } = req.body;

    if (listingId === undefined) {
      res.status(400).json({
        success: false,
        error: "Missing required field: listingId",
      });
      return;
    }

    const result = await darkAuctionService.cancelNftListing(listingId);
    res.json({ success: true, tx: result.tx });
  } catch (err: any) {
    console.error("[darkAuction] cancel-listing error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/dark-auction/claim-expired
// Body: { listingId }
router.post("/claim-expired", async (req: Request, res: Response) => {
  try {
    const { listingId } = req.body;

    if (listingId === undefined) {
      res.status(400).json({
        success: false,
        error: "Missing required field: listingId",
      });
      return;
    }

    const result = await darkAuctionService.claimNftExpired(listingId);
    res.json({
      success: true,
      recipientWallet: result.recipientWallet,
      tx: result.tx,
    });
  } catch (err: any) {
    console.error("[darkAuction] claim-expired error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// GET /api/v1/dark-auction/active-listings
router.get("/active-listings", async (_req: Request, res: Response) => {
  try {
    const listings = await darkAuctionService.getActiveNftListings();
    res.json({ success: true, listings });
  } catch (err: any) {
    console.error("[darkAuction] active-listings error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// GET /api/v1/dark-auction/listing/:listingId
router.get("/listing/:listingId", async (req: Request, res: Response) => {
  try {
    const listingId = parseInt(req.params.listingId, 10);
    const listing = await darkAuctionService.getNftListing(listingId);

    if (!listing) {
      res.status(404).json({ success: false, error: "Listing not found" });
      return;
    }

    res.json({ success: true, listing });
  } catch (err: any) {
    console.error("[darkAuction] get-listing error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// GET /api/v1/dark-auction/my-listings/:sellerWallet
router.get(
  "/my-listings/:sellerWallet",
  async (req: Request, res: Response) => {
    try {
      const { sellerWallet } = req.params;
      const listings = await darkAuctionService.getMyNftListings(sellerWallet);
      res.json({ success: true, listings });
    } catch (err: any) {
      console.error("[darkAuction] my-listings error:", err.message);
      res.status(500).json({ success: false, error: err.message });
    }
  }
);

// GET /api/v1/dark-auction/config
router.get("/config", async (_req: Request, res: Response) => {
  try {
    const cfg = await darkAuctionService.getNftConfig();
    if (!cfg) {
      res.status(404).json({
        success: false,
        error: "NFT config not initialized",
      });
      return;
    }
    res.json({ success: true, config: cfg });
  } catch (err: any) {
    console.error("[darkAuction] get-config error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// ── Escrow management (client-signed transactions) ──

// POST /api/v1/dark-auction/build-deposit-tx
// Body: { playerWallet, amount }
router.post("/build-deposit-tx", async (req: Request, res: Response) => {
  try {
    const { playerWallet, amount } = req.body;

    if (!playerWallet || !amount) {
      res.status(400).json({
        success: false,
        error: "Missing required fields: playerWallet, amount",
      });
      return;
    }

    const result = await darkAuctionService.buildDepositEscrowTx(
      playerWallet,
      amount
    );
    res.json({ success: true, ...result });
  } catch (err: any) {
    console.error("[darkAuction] build-deposit-tx error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/dark-auction/build-withdraw-tx
// Body: { playerWallet, amount }
router.post("/build-withdraw-tx", async (req: Request, res: Response) => {
  try {
    const { playerWallet, amount } = req.body;

    if (!playerWallet || !amount) {
      res.status(400).json({
        success: false,
        error: "Missing required fields: playerWallet, amount",
      });
      return;
    }

    const result = await darkAuctionService.buildWithdrawEscrowTx(
      playerWallet,
      amount
    );
    res.json({ success: true, ...result });
  } catch (err: any) {
    console.error("[darkAuction] build-withdraw-tx error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// GET /api/v1/dark-auction/escrow-balance/:wallet
router.get(
  "/escrow-balance/:wallet",
  async (req: Request, res: Response) => {
    try {
      const result = await darkAuctionService.getEscrowBalance(
        req.params.wallet
      );
      res.json({ success: true, ...result });
    } catch (err: any) {
      console.error("[darkAuction] escrow-balance error:", err.message);
      res.status(500).json({ success: false, error: err.message });
    }
  }
);

// ── Escrow-funded operations (game server calls these, authority signs) ──

// POST /api/v1/dark-auction/bid-escrow
// Body: { characterId, listingId, bidderWallet, bidAmount }
router.post("/bid-escrow", async (req: Request, res: Response) => {
  try {
    const { characterId, listingId, bidderWallet, bidAmount } = req.body;

    if (
      characterId === undefined ||
      listingId === undefined ||
      !bidderWallet ||
      !bidAmount
    ) {
      res.status(400).json({
        success: false,
        error:
          "Missing required fields: characterId, listingId, bidderWallet, bidAmount",
      });
      return;
    }

    const result = await darkAuctionService.placeNftBidFromEscrow(
      characterId,
      listingId,
      bidderWallet,
      bidAmount
    );

    res.json({
      success: true,
      newBid: parseInt(result.newBid),
      tx: result.tx,
    });
  } catch (err: any) {
    console.error("[darkAuction] bid-escrow error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/dark-auction/buyout-escrow
// Body: { characterId, listingId, buyerWallet }
router.post("/buyout-escrow", async (req: Request, res: Response) => {
  try {
    const { characterId, listingId, buyerWallet } = req.body;

    if (characterId === undefined || listingId === undefined || !buyerWallet) {
      res.status(400).json({
        success: false,
        error:
          "Missing required fields: characterId, listingId, buyerWallet",
      });
      return;
    }

    const result = await darkAuctionService.buyoutNftFromEscrow(
      characterId,
      listingId,
      buyerWallet
    );

    res.json({
      success: true,
      buyerWallet: result.buyerWallet,
      tx: result.tx,
    });
  } catch (err: any) {
    console.error("[darkAuction] buyout-escrow error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

export default router;
