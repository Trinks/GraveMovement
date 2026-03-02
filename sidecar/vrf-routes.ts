import { Router, Request, Response } from "express";
import { requireInternalSecret } from "../middleware/auth";
import * as vrfService from "../services/vrf";

const router = Router();
router.use(requireInternalSecret);

// POST /api/v1/vrf/initialize
router.post("/initialize", async (_req: Request, res: Response) => {
  try {
    const result = await vrfService.initialize();
    res.json({ success: true, ...result });
  } catch (err: any) {
    console.error("[vrf] initialize error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/vrf/request
// Body: { creatureId: number, creatureName: string, eligiblePlayers: number[] }
router.post("/request", async (req: Request, res: Response) => {
  try {
    const { creatureId, creatureName, eligiblePlayers } = req.body;
    const result = await vrfService.requestVrf(creatureId, creatureName, eligiblePlayers);
    res.json({ success: true, ...result });
  } catch (err: any) {
    console.error("[vrf] request error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/vrf/request-and-wait
// Request VRF and wait for MagicBlock oracle callback (replaces request-and-fulfill)
// Body: { creatureId: number, creatureName: string, eligiblePlayers: number[], timeoutMs?: number }
router.post("/request-and-wait", async (req: Request, res: Response) => {
  try {
    const { creatureId, creatureName, eligiblePlayers, timeoutMs } = req.body;
    const result = await vrfService.requestVrfAndWait(
      creatureId,
      creatureName,
      eligiblePlayers,
      timeoutMs
    );
    res.json({ success: true, ...result });
  } catch (err: any) {
    console.error("[vrf] request-and-wait error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// POST /api/v1/vrf/publish-receipt
// Body: { requestId, lootTableHash, generatedLootHash, items, coins, distributionMode, assignments, isFallbackRng }
router.post("/publish-receipt", async (req: Request, res: Response) => {
  try {
    const {
      requestId,
      lootTableHash,
      generatedLootHash,
      items,
      coins,
      distributionMode,
      assignments,
      isFallbackRng = false,
    } = req.body;
    const result = await vrfService.publishLootReceipt(
      requestId,
      lootTableHash,
      generatedLootHash,
      items,
      coins,
      distributionMode,
      assignments,
      isFallbackRng
    );
    res.json({ success: true, ...result });
  } catch (err: any) {
    console.error("[vrf] publish-receipt error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// GET /api/v1/vrf/request/:requestId
router.get("/request/:requestId", async (req: Request, res: Response) => {
  try {
    const requestId = parseInt(req.params.requestId, 10);
    const vrfRequest = await vrfService.getVrfRequest(requestId);
    if (!vrfRequest) {
      res.status(404).json({ success: false, error: "VRF request not found" });
      return;
    }
    res.json({ success: true, vrfRequest });
  } catch (err: any) {
    console.error("[vrf] get-request error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// GET /api/v1/vrf/receipt/:receiptId
router.get("/receipt/:receiptId", async (req: Request, res: Response) => {
  try {
    const receiptId = parseInt(req.params.receiptId, 10);
    const receipt = await vrfService.getLootReceipt(receiptId);
    if (!receipt) {
      res.status(404).json({ success: false, error: "Loot receipt not found" });
      return;
    }
    res.json({ success: true, receipt });
  } catch (err: any) {
    console.error("[vrf] get-receipt error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

// GET /api/v1/vrf/config
router.get("/config", async (_req: Request, res: Response) => {
  try {
    const cfg = await vrfService.getLootVrfConfig();
    if (!cfg) {
      res.status(404).json({ success: false, error: "VRF config not initialized" });
      return;
    }
    res.json({ success: true, config: cfg });
  } catch (err: any) {
    console.error("[vrf] get-config error:", err.message);
    res.status(500).json({ success: false, error: err.message });
  }
});

export default router;
