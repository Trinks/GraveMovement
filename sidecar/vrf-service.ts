import { PublicKey, SystemProgram, SYSVAR_SLOT_HASHES_PUBKEY } from "@solana/web3.js";
import { BN } from "@coral-xyz/anchor";
import { createHash } from "crypto";
import { config } from "../config";
import {
  getLootVrfProgram,
  getAuthority,
  getConnection,
} from "./solana";

// ── MagicBlock VRF Constants (devnet) ──

const VRF_PROGRAM_ID = new PublicKey("Vrf1RNUjXmQGjmQrQLvJHs9SNkvDJEsRVFPkfSQUwGz");
const DEFAULT_QUEUE = new PublicKey("Cuj97ggrhhidhbu39TijNVqE74xvKJ69gDervRUXAxGh");

// ── PDA helpers ──

function getLootVrfConfigPda(): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("loot_vrf_config")],
    config.lootVrfProgramId
  );
}

function getVrfRequestPda(requestId: BN): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("vrf_request"), requestId.toArrayLike(Buffer, "le", 8)],
    config.lootVrfProgramId
  );
}

function getLootReceiptPda(receiptId: BN): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("loot_receipt"), receiptId.toArrayLike(Buffer, "le", 8)],
    config.lootVrfProgramId
  );
}

/** Derive our program's identity PDA (used by #[vrf] macro for CPI signing). */
function getProgramIdentityPda(): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("identity")],
    config.lootVrfProgramId
  );
}

// ── Service functions ──

export async function initialize(): Promise<{ tx: string; lootVrfConfig: string }> {
  const program = getLootVrfProgram();
  const authority = getAuthority();
  const [lootVrfConfigPda] = getLootVrfConfigPda();

  const tx = await program.methods
    .initialize(DEFAULT_QUEUE) // MagicBlock oracle queue
    .accounts({
      lootVrfConfig: lootVrfConfigPda,
      authority: authority.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  return { tx, lootVrfConfig: lootVrfConfigPda.toBase58() };
}

export async function requestVrf(
  creatureId: number,
  creatureName: string,
  eligiblePlayers: number[]
): Promise<{ tx: string; requestId: string; vrfRequest: string }> {
  const program = getLootVrfProgram();
  const authority = getAuthority();
  const [lootVrfConfigPda] = getLootVrfConfigPda();
  const [programIdentityPda] = getProgramIdentityPda();

  // Get current request count for next request ID
  const configAcct = await (program.account as any).lootVrfConfig.fetch(lootVrfConfigPda);
  const requestId = configAcct.requestCount as BN;
  const [vrfRequestPda] = getVrfRequestPda(requestId);

  const creatureNameHash = Array.from(
    createHash("sha256").update(creatureName).digest()
  );

  // Convert player IDs to BN array
  const eligiblePlayersBn = eligiblePlayers.map((id) => new BN(id));

  const tx = await program.methods
    .requestVrf(creatureId, creatureNameHash, eligiblePlayersBn)
    .accounts({
      lootVrfConfig: lootVrfConfigPda,
      vrfRequest: vrfRequestPda,
      authority: authority.publicKey,
      oracleQueue: DEFAULT_QUEUE,
      systemProgram: SystemProgram.programId,
      programIdentity: programIdentityPda,
      vrfProgram: VRF_PROGRAM_ID,
      slotHashes: SYSVAR_SLOT_HASHES_PUBKEY,
    })
    .signers([authority])
    .rpc();

  return {
    tx,
    requestId: requestId.toString(),
    vrfRequest: vrfRequestPda.toBase58(),
  };
}

/**
 * Poll the VrfRequest account until it transitions to Fulfilled state.
 * MagicBlock's oracle typically responds within 1-2 seconds.
 */
export async function waitForFulfillment(
  requestId: number,
  timeoutMs: number = 30000
): Promise<{ fulfilled: boolean; randomness: number[] | null; elapsedMs: number }> {
  const program = getLootVrfProgram();
  const requestIdBn = new BN(requestId);
  const [vrfRequestPda] = getVrfRequestPda(requestIdBn);

  const startTime = Date.now();
  const pollIntervalMs = 1000;

  while (Date.now() - startTime < timeoutMs) {
    try {
      const req = await (program.account as any).vrfRequest.fetch(vrfRequestPda);
      // VrfRequestState: Pending=0, Fulfilled=1, Used=2, Expired=3
      // Anchor deserializes enums as objects like { fulfilled: {} }
      const state = req.state;
      if (state.fulfilled || state.used) {
        return {
          fulfilled: true,
          randomness: Array.from(req.randomness as number[]),
          elapsedMs: Date.now() - startTime,
        };
      }
    } catch {
      // Account may not exist yet, keep polling
    }

    await new Promise((resolve) => setTimeout(resolve, pollIntervalMs));
  }

  return { fulfilled: false, randomness: null, elapsedMs: Date.now() - startTime };
}

/**
 * Request VRF and wait for MagicBlock oracle callback.
 * Replaces the old requestAndFulfillVrf mock flow.
 */
export async function requestVrfAndWait(
  creatureId: number,
  creatureName: string,
  eligiblePlayers: number[],
  timeoutMs: number = 30000
): Promise<{
  requestTx: string;
  requestId: string;
  fulfilled: boolean;
  randomness: number[] | null;
  seedHex: string | null;
  elapsedMs: number;
}> {
  const reqResult = await requestVrf(creatureId, creatureName, eligiblePlayers);
  const waitResult = await waitForFulfillment(
    parseInt(reqResult.requestId, 10),
    timeoutMs
  );

  const seedHex = waitResult.randomness
    ? Buffer.from(waitResult.randomness).toString("hex")
    : null;

  return {
    requestTx: reqResult.tx,
    requestId: reqResult.requestId,
    fulfilled: waitResult.fulfilled,
    randomness: waitResult.randomness,
    seedHex,
    elapsedMs: waitResult.elapsedMs,
  };
}

export interface LootItemRecord {
  itemId: number;
  quantity: number;
  poolIndex: number;
  rollValue: number;
}

export interface LootAssignment {
  characterId: number;
  itemId: number;
  quantity: number;
  reason: number;
}

export async function publishLootReceipt(
  requestId: number,
  lootTableHash: number[],
  generatedLootHash: number[],
  items: LootItemRecord[],
  coins: number,
  distributionMode: number,
  assignments: LootAssignment[],
  isFallbackRng: boolean
): Promise<{ tx: string; receiptId: string; lootReceipt: string }> {
  const program = getLootVrfProgram();
  const authority = getAuthority();
  const requestIdBn = new BN(requestId);
  const [lootVrfConfigPda] = getLootVrfConfigPda();
  const [vrfRequestPda] = getVrfRequestPda(requestIdBn);

  // Get current receipt count for next receipt ID
  const configAcct = await (program.account as any).lootVrfConfig.fetch(lootVrfConfigPda);
  const receiptId = configAcct.receiptCount as BN;
  const [lootReceiptPda] = getLootReceiptPda(receiptId);

  // Convert items to program format
  const programItems = items.map((item) => ({
    itemId: item.itemId,
    quantity: item.quantity,
    poolIndex: item.poolIndex,
    rollValue: item.rollValue,
  }));

  // Convert assignments to program format
  const programAssignments = assignments.map((a) => ({
    characterId: new BN(a.characterId),
    itemId: a.itemId,
    quantity: a.quantity,
    reason: a.reason,
  }));

  const tx = await program.methods
    .publishLootReceipt(
      requestIdBn,
      Buffer.from(lootTableHash),
      Buffer.from(generatedLootHash),
      programItems,
      new BN(coins),
      distributionMode,
      programAssignments,
      isFallbackRng
    )
    .accounts({
      lootVrfConfig: lootVrfConfigPda,
      vrfRequest: vrfRequestPda,
      lootReceipt: lootReceiptPda,
      authority: authority.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  return {
    tx,
    receiptId: receiptId.toString(),
    lootReceipt: lootReceiptPda.toBase58(),
  };
}

export async function getVrfRequest(requestId: number): Promise<any> {
  const program = getLootVrfProgram();
  const requestIdBn = new BN(requestId);
  const [vrfRequestPda] = getVrfRequestPda(requestIdBn);

  try {
    const req = await (program.account as any).vrfRequest.fetch(vrfRequestPda);
    return {
      requestId: (req.requestId as BN).toString(),
      creatureId: req.creatureId,
      creatureNameHash: Array.from(req.creatureNameHash as number[]),
      requester: (req.requester as PublicKey).toBase58(),
      state: req.state,
      randomness: Array.from(req.randomness as number[]),
      eligiblePlayerCount: req.eligiblePlayerCount,
      eligiblePlayers: (req.eligiblePlayers as BN[])
        .slice(0, req.eligiblePlayerCount as number)
        .map((p) => p.toString()),
      requestedAt: (req.requestedAt as BN).toString(),
      fulfilledAt: (req.fulfilledAt as BN).toString(),
    };
  } catch {
    return null;
  }
}

export async function getLootReceipt(receiptId: number): Promise<any> {
  const program = getLootVrfProgram();
  const receiptIdBn = new BN(receiptId);
  const [lootReceiptPda] = getLootReceiptPda(receiptIdBn);

  try {
    const receipt = await (program.account as any).lootReceipt.fetch(lootReceiptPda);
    const itemCount = receipt.itemCount as number;
    const assignmentCount = receipt.assignmentCount as number;

    return {
      receiptId: (receipt.receiptId as BN).toString(),
      vrfRequestId: (receipt.vrfRequestId as BN).toString(),
      creatureId: receipt.creatureId,
      vrfSeed: Array.from(receipt.vrfSeed as number[]),
      lootTableHash: Array.from(receipt.lootTableHash as number[]),
      generatedLootHash: Array.from(receipt.generatedLootHash as number[]),
      itemCount,
      items: (receipt.items as any[]).slice(0, itemCount).map((item) => ({
        itemId: item.itemId,
        quantity: item.quantity,
        poolIndex: item.poolIndex,
        rollValue: item.rollValue,
      })),
      coinsGenerated: (receipt.coinsGenerated as BN).toString(),
      distributionMode: receipt.distributionMode,
      assignmentCount,
      assignments: (receipt.assignments as any[]).slice(0, assignmentCount).map((a) => ({
        characterId: (a.characterId as BN).toString(),
        itemId: a.itemId,
        quantity: a.quantity,
        reason: a.reason,
      })),
      isFallbackRng: receipt.isFallbackRng,
      publishedAt: (receipt.publishedAt as BN).toString(),
    };
  } catch {
    return null;
  }
}

export async function getLootVrfConfig(): Promise<any> {
  const program = getLootVrfProgram();
  const [lootVrfConfigPda] = getLootVrfConfigPda();

  try {
    const cfg = await (program.account as any).lootVrfConfig.fetch(lootVrfConfigPda);
    return {
      authority: (cfg.authority as PublicKey).toBase58(),
      oracleQueue: (cfg.oracleQueue as PublicKey).toBase58(),
      requestCount: (cfg.requestCount as BN).toString(),
      receiptCount: (cfg.receiptCount as BN).toString(),
    };
  } catch {
    return null;
  }
}
