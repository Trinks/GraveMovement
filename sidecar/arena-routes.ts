import { Router, Request, Response } from "express";
import { Keypair } from "@solana/web3.js";
import { requireInternalSecret } from "../middleware/auth";
import * as arenaService from "../services/arena";

const router = Router();
router.use(requireInternalSecret);

// POST /api/v1/arena/initialize
router.post("/initialize", async (req: Request, res: Response) => {
  try {
    const { feeBps = 200, minWager = 10_000_000, maxWager = 100_000_000_000 } = req.body;
    const result = await arenaService.initialize(feeBps, minWager, maxWager);
    res.json({ success: true, ...result });
  } catch (err: any) {
    console.error("[arena] initialize error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/arena/create-match
// Body: { playerSecretKey?: number[], arenaType: number, wagerAmountLamports: number, useAuthority?: boolean, playerA?: string }
router.post("/create-match", async (req: Request, res: Response) => {
  try {
    const { playerSecretKey, arenaType, wagerAmountLamports, useAuthority, playerA } = req.body;
    if (useAuthority) {
      // Authority operates on behalf of player; playerA is a pubkey string
      const playerPubkey = playerA || "11111111111111111111111111111111";
      const result = await arenaService.createMatchOperated(arenaType, wagerAmountLamports, playerPubkey);
      res.json({ success: true, ...result });
    } else {
      const playerKeypair = Keypair.fromSecretKey(Uint8Array.from(playerSecretKey));
      const result = await arenaService.createMatch(playerKeypair, arenaType, wagerAmountLamports);
      res.json({ success: true, ...result });
    }
  } catch (err: any) {
    console.error("[arena] create-match error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/arena/join-match
// Body: { playerSecretKey?: number[], matchId: number, useAuthority?: boolean, playerB?: string }
router.post("/join-match", async (req: Request, res: Response) => {
  try {
    const { playerSecretKey, matchId, useAuthority, playerB } = req.body;
    if (useAuthority) {
      const playerPubkey = playerB || "11111111111111111111111111111111";
      const result = await arenaService.joinMatchOperated(matchId, playerPubkey);
      res.json({ success: true, ...result });
    } else {
      const playerKeypair = Keypair.fromSecretKey(Uint8Array.from(playerSecretKey));
      const result = await arenaService.joinMatch(playerKeypair, matchId);
      res.json({ success: true, ...result });
    }
  } catch (err: any) {
    console.error("[arena] join-match error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/arena/fund-match
// Body: { playerSecretKey?: number[], matchId: number, useAuthority?: boolean }
router.post("/fund-match", async (req: Request, res: Response) => {
  try {
    const { playerSecretKey, matchId, useAuthority } = req.body;
    if (useAuthority) {
      const result = await arenaService.fundMatchOperated(matchId);
      res.json({ success: true, ...result });
    } else {
      const playerKeypair = Keypair.fromSecretKey(Uint8Array.from(playerSecretKey));
      const result = await arenaService.fundMatch(playerKeypair, matchId);
      res.json({ success: true, ...result });
    }
  } catch (err: any) {
    console.error("[arena] fund-match error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/arena/activate-match
// Body: { matchId: number }
router.post("/activate-match", async (req: Request, res: Response) => {
  try {
    const { matchId } = req.body;
    const result = await arenaService.activateMatch(matchId);
    res.json({ success: true, ...result });
  } catch (err: any) {
    console.error("[arena] activate-match error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/arena/submit-result
// Body: { matchId: number, winner: string, combatHash: number[32] }
router.post("/submit-result", async (req: Request, res: Response) => {
  try {
    const { matchId, winner, combatHash } = req.body;
    const result = await arenaService.submitResult(matchId, winner, combatHash);
    res.json({ success: true, ...result });
  } catch (err: any) {
    console.error("[arena] submit-result error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/arena/claim-winnings
// Body: { winnerSecretKey?: number[], matchId: number, useAuthority?: boolean }
router.post("/claim-winnings", async (req: Request, res: Response) => {
  try {
    const { winnerSecretKey, matchId, useAuthority } = req.body;
    if (useAuthority) {
      const result = await arenaService.claimWinningsOperated(matchId);
      res.json({ success: true, ...result });
    } else {
      const winnerKeypair = Keypair.fromSecretKey(Uint8Array.from(winnerSecretKey));
      const result = await arenaService.claimWinnings(winnerKeypair, matchId);
      res.json({ success: true, ...result });
    }
  } catch (err: any) {
    console.error("[arena] claim-winnings error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/arena/settle-and-pay
// Body: { matchId: number, winner: string, combatHash: number[32] }
// Submits result, claims winnings to authority, transfers payout to winner wallet.
router.post("/settle-and-pay", async (req: Request, res: Response) => {
  try {
    const { matchId, winner, combatHash } = req.body;
    if (matchId === undefined || !winner || !combatHash) {
      res.status(400).json({
        success: false,
        error: "Missing required fields: matchId, winner, combatHash",
      });
      return;
    }
    const result = await arenaService.settleAndPay(matchId, winner, combatHash);
    res.json({ success: true, ...result });
  } catch (err: any) {
    console.error("[arena] settle-and-pay error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/arena/cancel-match
// Body: { matchId: number, playerA: string, playerB: string }
router.post("/cancel-match", async (req: Request, res: Response) => {
  try {
    const { matchId, playerA, playerB } = req.body;
    const result = await arenaService.cancelMatch(matchId, playerA, playerB);
    res.json({ success: true, ...result });
  } catch (err: any) {
    console.error("[arena] cancel-match error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/arena/fund-match-escrow
// Body: { matchId: number, playerAWallet: string, playerBWallet: string, wagerAmountLamports: number }
router.post("/fund-match-escrow", async (req: Request, res: Response) => {
  try {
    const { matchId, playerAWallet, playerBWallet, wagerAmountLamports } = req.body;
    if (matchId === undefined || !playerAWallet || !playerBWallet || !wagerAmountLamports) {
      res.status(400).json({
        success: false,
        error: "Missing required fields: matchId, playerAWallet, playerBWallet, wagerAmountLamports",
      });
      return;
    }
    const result = await arenaService.fundMatchFromEscrow(
      matchId, playerAWallet, playerBWallet, wagerAmountLamports
    );
    res.json({ success: true, ...result });
  } catch (err: any) {
    console.error("[arena] fund-match-escrow error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/arena/settle-and-pay-escrow
// Body: { matchId: number, winner: string, combatHash: number[32] }
router.post("/settle-and-pay-escrow", async (req: Request, res: Response) => {
  try {
    const { matchId, winner, combatHash } = req.body;
    if (matchId === undefined || !winner || !combatHash) {
      res.status(400).json({
        success: false,
        error: "Missing required fields: matchId, winner, combatHash",
      });
      return;
    }
    const result = await arenaService.settleAndPayToEscrow(matchId, winner, combatHash);
    res.json({ success: true, ...result });
  } catch (err: any) {
    console.error("[arena] settle-and-pay-escrow error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/arena/cancel-and-refund-escrow
// Body: { matchId: number, playerAWallet: string, playerBWallet: string, wagerAmountLamports: number }
router.post("/cancel-and-refund-escrow", async (req: Request, res: Response) => {
  try {
    const { matchId, playerAWallet, playerBWallet, wagerAmountLamports } = req.body;
    if (matchId === undefined || !playerAWallet || !playerBWallet || !wagerAmountLamports) {
      res.status(400).json({
        success: false,
        error: "Missing required fields: matchId, playerAWallet, playerBWallet, wagerAmountLamports",
      });
      return;
    }
    const result = await arenaService.cancelAndRefundToEscrow(
      matchId, playerAWallet, playerBWallet, wagerAmountLamports
    );
    res.json({ success: true, ...result });
  } catch (err: any) {
    console.error("[arena] cancel-and-refund-escrow error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// GET /api/v1/arena/escrow-balance/:wallet
router.get("/escrow-balance/:wallet", async (req: Request, res: Response) => {
  try {
    const { wallet } = req.params;
    const result = await arenaService.getEscrowBalance(wallet);
    res.json({ success: true, balanceLamports: result.balance });
  } catch (err: any) {
    console.error("[arena] escrow-balance error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// GET /api/v1/arena/match/:matchId
router.get("/match/:matchId", async (req: Request, res: Response) => {
  try {
    const matchId = parseInt(req.params.matchId, 10);
    const match = await arenaService.getMatch(matchId);
    if (!match) {
      res.status(404).json({ success: false, error: "Match not found" });
      return;
    }
    res.json({ success: true, match });
  } catch (err: any) {
    console.error("[arena] get-match error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// GET /api/v1/arena/config
router.get("/config", async (_req: Request, res: Response) => {
  try {
    const cfg = await arenaService.getArenaConfig();
    if (!cfg) {
      res.status(404).json({ success: false, error: "Arena not initialized" });
      return;
    }
    res.json({ success: true, config: cfg });
  } catch (err: any) {
    console.error("[arena] get-config error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

export default router;
