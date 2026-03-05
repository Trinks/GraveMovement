import { Router, Request, Response } from "express";
import * as vrfService from "../services/vrf";
import * as arenaService from "../services/arena";
import { SeededRandom } from "../services/seededRandom";

const router = Router();

// Public verification routes — no requireInternalSecret.
// Add CORS headers for browser access on every response.
router.use((_req, res, next) => {
  res.header("Access-Control-Allow-Origin", "*");
  res.header("Access-Control-Allow-Methods", "GET, POST, OPTIONS");
  res.header("Access-Control-Allow-Headers", "Content-Type");
  next();
});

// ── GET /api/v1/verify/vrf/:requestId ──
// Fetch a VRF request from chain.
router.get("/vrf/:requestId", async (req: Request, res: Response) => {
  try {
    const requestId = parseInt(req.params.requestId, 10);
    if (isNaN(requestId)) {
      res.status(400).json({ success: false, error: "Invalid requestId" });
      return;
    }

    const vrfRequest = await vrfService.getVrfRequest(requestId);
    if (!vrfRequest) {
      res.status(404).json({ success: false, error: "VRF request not found" });
      return;
    }

    // Convert randomness byte array to hex for readability
    const randomnessHex = Buffer.from(vrfRequest.randomness).toString("hex");

    res.json({
      success: true,
      vrfRequest: {
        requestId: vrfRequest.requestId,
        creatureId: vrfRequest.creatureId,
        state: vrfRequest.state,
        randomness: vrfRequest.randomness,
        randomnessHex,
        requestedAt: vrfRequest.requestedAt,
        fulfilledAt: vrfRequest.fulfilledAt,
        eligiblePlayerCount: vrfRequest.eligiblePlayerCount,
        eligiblePlayers: vrfRequest.eligiblePlayers,
      },
    });
  } catch (err: any) {
    console.error("[verify] vrf error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// ── GET /api/v1/verify/loot/:receiptId ──
// Fetch a loot receipt AND its associated VRF request.
router.get("/loot/:receiptId", async (req: Request, res: Response) => {
  try {
    const receiptId = parseInt(req.params.receiptId, 10);
    if (isNaN(receiptId)) {
      res.status(400).json({ success: false, error: "Invalid receiptId" });
      return;
    }

    const receipt = await vrfService.getLootReceipt(receiptId);
    if (!receipt) {
      res.status(404).json({ success: false, error: "Loot receipt not found" });
      return;
    }

    // Also fetch the associated VRF request for provenance
    const vrfRequestId = parseInt(receipt.vrfRequestId, 10);
    const vrfRequest = await vrfService.getVrfRequest(vrfRequestId);

    // Convert byte arrays to hex for readability
    const vrfSeedHex = Buffer.from(receipt.vrfSeed).toString("hex");
    const lootTableHashHex = Buffer.from(receipt.lootTableHash).toString("hex");
    const generatedLootHashHex = Buffer.from(receipt.generatedLootHash).toString("hex");

    res.json({
      success: true,
      receipt: {
        receiptId: receipt.receiptId,
        vrfRequestId: receipt.vrfRequestId,
        creatureId: receipt.creatureId,
        vrfSeed: receipt.vrfSeed,
        vrfSeedHex,
        lootTableHash: receipt.lootTableHash,
        lootTableHashHex,
        generatedLootHash: receipt.generatedLootHash,
        generatedLootHashHex,
        items: receipt.items,
        coinsGenerated: receipt.coinsGenerated,
        distributionMode: receipt.distributionMode,
        assignments: receipt.assignments,
        isFallbackRng: receipt.isFallbackRng,
        publishedAt: receipt.publishedAt,
      },
      vrfRequest: vrfRequest || null,
    });
  } catch (err: any) {
    console.error("[verify] loot error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// ── GET /api/v1/verify/duel/:matchId ──
// Fetch a duel match from the arena program.
router.get("/duel/:matchId", async (req: Request, res: Response) => {
  try {
    const matchId = parseInt(req.params.matchId, 10);
    if (isNaN(matchId)) {
      res.status(400).json({ success: false, error: "Invalid matchId" });
      return;
    }

    const match = await arenaService.getMatch(matchId);
    if (!match) {
      res.status(404).json({ success: false, error: "Match not found" });
      return;
    }

    // Convert combatHash byte array to hex
    const combatHashHex = Buffer.from(match.combatHash).toString("hex");

    res.json({
      success: true,
      match: {
        matchId: match.matchId,
        arenaType: match.arenaType,
        state: match.state,
        playerA: match.playerA,
        playerB: match.playerB,
        wagerAmount: match.wagerAmount,
        winner: match.winner,
        combatHash: match.combatHash,
        combatHashHex,
        createdAt: match.createdAt,
        settledAt: match.settledAt,
      },
    });
  } catch (err: any) {
    console.error("[verify] duel error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// ── POST /api/v1/verify/replay-loot ──
// Replay the PRNG with a VRF seed and roll sequence to verify loot results.
router.post("/replay-loot", async (req: Request, res: Response) => {
  try {
    const { vrfSeedHex, rolls } = req.body;

    if (!vrfSeedHex || typeof vrfSeedHex !== "string") {
      res.status(400).json({ success: false, error: "Missing or invalid vrfSeedHex" });
      return;
    }
    if (!Array.isArray(rolls) || rolls.length === 0) {
      res.status(400).json({ success: false, error: "Missing or empty rolls array" });
      return;
    }
    if (vrfSeedHex.length !== 64) {
      res.status(400).json({ success: false, error: "vrfSeedHex must be 64 hex characters (32 bytes)" });
      return;
    }

    const rng = new SeededRandom(vrfSeedHex);
    const results: {
      poolIndex: number;
      rollType: string;
      min?: number;
      max?: number;
      result: number;
    }[] = [];

    for (const roll of rolls) {
      const { poolIndex, rollType, min, max } = roll;

      if (rollType === "value") {
        results.push({
          poolIndex,
          rollType: "value",
          result: rng.value(),
        });
      } else if (rollType === "range") {
        if (min === undefined || max === undefined) {
          res.status(400).json({
            success: false,
            error: `Roll at poolIndex ${poolIndex}: range type requires min and max`,
          });
          return;
        }
        results.push({
          poolIndex,
          rollType: "range",
          min,
          max,
          result: rng.range(min, max),
        });
      } else {
        res.status(400).json({
          success: false,
          error: `Invalid rollType "${rollType}" at poolIndex ${poolIndex}. Must be "value" or "range".`,
        });
        return;
      }
    }

    res.json({
      success: true,
      vrfSeedHex,
      rollCount: results.length,
      callCount: rng.callCount,
      results,
    });
  } catch (err: any) {
    console.error("[verify] replay-loot error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// ── POST /api/v1/verify/replay-combat ──
// Replay the PRNG with a VRF seed and combat roll sequence.
router.post("/replay-combat", async (req: Request, res: Response) => {
  try {
    const { vrfSeedHex, rolls } = req.body;

    if (!vrfSeedHex || typeof vrfSeedHex !== "string") {
      res.status(400).json({ success: false, error: "Missing or invalid vrfSeedHex" });
      return;
    }
    if (!Array.isArray(rolls) || rolls.length === 0) {
      res.status(400).json({ success: false, error: "Missing or empty rolls array" });
      return;
    }
    if (vrfSeedHex.length !== 64) {
      res.status(400).json({ success: false, error: "vrfSeedHex must be 64 hex characters (32 bytes)" });
      return;
    }

    const rng = new SeededRandom(vrfSeedHex);
    const results: {
      index: number;
      type: string;
      min?: number;
      max?: number;
      result: number;
    }[] = [];

    for (let i = 0; i < rolls.length; i++) {
      const roll = rolls[i];
      const { type, min, max } = roll;

      if (type === "value") {
        results.push({
          index: i,
          type: "value",
          result: rng.value(),
        });
      } else if (type === "range") {
        if (min === undefined || max === undefined) {
          res.status(400).json({
            success: false,
            error: `Roll at index ${i}: range type requires min and max`,
          });
          return;
        }
        results.push({
          index: i,
          type: "range",
          min,
          max,
          result: rng.range(min, max),
        });
      } else {
        res.status(400).json({
          success: false,
          error: `Invalid roll type "${type}" at index ${i}. Must be "value" or "range".`,
        });
        return;
      }
    }

    res.json({
      success: true,
      vrfSeedHex,
      rollCount: results.length,
      callCount: rng.callCount,
      results,
    });
  } catch (err: any) {
    console.error("[verify] replay-combat error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

export default router;
