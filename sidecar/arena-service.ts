import { PublicKey, SystemProgram, Keypair, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { BN } from "@coral-xyz/anchor";
import { config } from "../config";
import {
  getArenaProgram,
  getMarketplaceProgram,
  getAuthority,
  getTreasury,
  getConnection,
  getProvider,
} from "./solana";
import { getPlayerEscrowPda, getEscrowBalance } from "./darkAuction";

// ── PDA helpers ──

function getArenaConfigPda(): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("arena_config")],
    config.arenaProgramId
  );
}

function getMatchEscrowPda(matchId: BN): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("match_escrow"), matchId.toArrayLike(Buffer, "le", 8)],
    config.arenaProgramId
  );
}

// ── Service functions ──

export async function initialize(
  feeBps: number,
  minWager: number,
  maxWager: number
): Promise<{ tx: string; arenaConfig: string }> {
  const program = getArenaProgram();
  const authority = getAuthority();
  const treasury = getTreasury();
  const [arenaConfigPda] = getArenaConfigPda();

  const tx = await program.methods
    .initialize(
      authority.publicKey,   // oracle
      treasury,              // treasury
      feeBps,                // feeBps
      new BN(minWager),      // minWager
      new BN(maxWager)       // maxWager
    )
    .accounts({
      arenaConfig: arenaConfigPda,
      authority: authority.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  return { tx, arenaConfig: arenaConfigPda.toBase58() };
}

export async function createMatch(
  playerKeypair: Keypair,
  arenaType: number,
  wagerAmount: number
): Promise<{ tx: string; matchId: string; matchEscrow: string }> {
  const program = getArenaProgram();
  const [arenaConfigPda] = getArenaConfigPda();

  // Fetch config to get current match count (= next matchId)
  const configAcct = await (program.account as any).arenaConfig.fetch(arenaConfigPda);
  const matchId = configAcct.matchCount as BN;
  const [matchEscrowPda] = getMatchEscrowPda(matchId);

  const tx = await program.methods
    .createMatch(arenaType, new BN(wagerAmount))
    .accounts({
      arenaConfig: arenaConfigPda,
      matchEscrow: matchEscrowPda,
      player: playerKeypair.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .signers([playerKeypair])
    .rpc();

  return {
    tx,
    matchId: matchId.toString(),
    matchEscrow: matchEscrowPda.toBase58(),
  };
}

export async function joinMatch(
  playerKeypair: Keypair,
  matchId: number
): Promise<{ tx: string }> {
  const program = getArenaProgram();
  const matchIdBn = new BN(matchId);
  const [matchEscrowPda] = getMatchEscrowPda(matchIdBn);

  const tx = await program.methods
    .joinMatch(matchIdBn)
    .accounts({
      matchEscrow: matchEscrowPda,
      player: playerKeypair.publicKey,
    })
    .signers([playerKeypair])
    .rpc();

  return { tx };
}

export async function fundMatch(
  playerKeypair: Keypair,
  matchId: number
): Promise<{ tx: string }> {
  const program = getArenaProgram();
  const matchIdBn = new BN(matchId);
  const [matchEscrowPda] = getMatchEscrowPda(matchIdBn);

  const tx = await program.methods
    .fundMatch(matchIdBn)
    .accounts({
      matchEscrow: matchEscrowPda,
      player: playerKeypair.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .signers([playerKeypair])
    .rpc();

  return { tx };
}

export async function activateMatch(matchId: number): Promise<{ tx: string }> {
  const program = getArenaProgram();
  const authority = getAuthority();
  const matchIdBn = new BN(matchId);
  const [arenaConfigPda] = getArenaConfigPda();
  const [matchEscrowPda] = getMatchEscrowPda(matchIdBn);

  const tx = await program.methods
    .activateMatch(matchIdBn)
    .accounts({
      arenaConfig: arenaConfigPda,
      matchEscrow: matchEscrowPda,
      oracle: authority.publicKey,
    })
    .signers([authority])
    .rpc();

  return { tx };
}

export async function submitResult(
  matchId: number,
  winner: string,
  combatHash: number[]
): Promise<{ tx: string }> {
  const program = getArenaProgram();
  const authority = getAuthority();
  const matchIdBn = new BN(matchId);
  const [arenaConfigPda] = getArenaConfigPda();
  const [matchEscrowPda] = getMatchEscrowPda(matchIdBn);

  const tx = await program.methods
    .submitResult(matchIdBn, new PublicKey(winner), Buffer.from(combatHash))
    .accounts({
      arenaConfig: arenaConfigPda,
      matchEscrow: matchEscrowPda,
      oracle: authority.publicKey,
    })
    .signers([authority])
    .rpc();

  return { tx };
}

export async function claimWinnings(
  winnerKeypair: Keypair,
  matchId: number
): Promise<{ tx: string }> {
  const program = getArenaProgram();
  const matchIdBn = new BN(matchId);
  const [arenaConfigPda] = getArenaConfigPda();
  const [matchEscrowPda] = getMatchEscrowPda(matchIdBn);
  const treasury = getTreasury();

  const tx = await program.methods
    .claimWinnings(matchIdBn)
    .accounts({
      arenaConfig: arenaConfigPda,
      matchEscrow: matchEscrowPda,
      winner: winnerKeypair.publicKey,
      treasury: treasury,
      systemProgram: SystemProgram.programId,
    })
    .signers([winnerKeypair])
    .rpc();

  return { tx };
}

// ── Authority-operated functions ──

export async function createMatchOperated(
  arenaType: number,
  wagerAmount: number,
  playerA: string
): Promise<{ tx: string; matchId: string; matchEscrow: string }> {
  const program = getArenaProgram();
  const authority = getAuthority();
  const [arenaConfigPda] = getArenaConfigPda();

  const configAcct = await (program.account as any).arenaConfig.fetch(arenaConfigPda);
  const matchId = configAcct.matchCount as BN;
  const [matchEscrowPda] = getMatchEscrowPda(matchId);

  const tx = await program.methods
    .createMatchOperated(arenaType, new BN(wagerAmount), new PublicKey(playerA))
    .accounts({
      arenaConfig: arenaConfigPda,
      matchEscrow: matchEscrowPda,
      authority: authority.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  return {
    tx,
    matchId: matchId.toString(),
    matchEscrow: matchEscrowPda.toBase58(),
  };
}

export async function joinMatchOperated(
  matchId: number,
  playerB: string
): Promise<{ tx: string }> {
  const program = getArenaProgram();
  const authority = getAuthority();
  const matchIdBn = new BN(matchId);
  const [arenaConfigPda] = getArenaConfigPda();
  const [matchEscrowPda] = getMatchEscrowPda(matchIdBn);

  const tx = await program.methods
    .joinMatchOperated(matchIdBn, new PublicKey(playerB))
    .accounts({
      arenaConfig: arenaConfigPda,
      matchEscrow: matchEscrowPda,
      authority: authority.publicKey,
    })
    .signers([authority])
    .rpc();

  return { tx };
}

export async function fundMatchOperated(
  matchId: number
): Promise<{ tx: string }> {
  const program = getArenaProgram();
  const authority = getAuthority();
  const matchIdBn = new BN(matchId);
  const [arenaConfigPda] = getArenaConfigPda();
  const [matchEscrowPda] = getMatchEscrowPda(matchIdBn);

  const tx = await program.methods
    .fundMatchOperated(matchIdBn)
    .accounts({
      arenaConfig: arenaConfigPda,
      matchEscrow: matchEscrowPda,
      authority: authority.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  return { tx };
}

export async function claimWinningsOperated(
  matchId: number
): Promise<{ tx: string }> {
  const program = getArenaProgram();
  const authority = getAuthority();
  const matchIdBn = new BN(matchId);
  const [arenaConfigPda] = getArenaConfigPda();
  const [matchEscrowPda] = getMatchEscrowPda(matchIdBn);
  const treasury = getTreasury();

  const tx = await program.methods
    .claimWinningsOperated(matchIdBn)
    .accounts({
      arenaConfig: arenaConfigPda,
      matchEscrow: matchEscrowPda,
      authority: authority.publicKey,
      treasury: treasury,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  return { tx };
}

export async function cancelMatch(matchId: number, playerA: string, playerB: string): Promise<{ tx: string }> {
  const program = getArenaProgram();
  const authority = getAuthority();
  const matchIdBn = new BN(matchId);
  const [arenaConfigPda] = getArenaConfigPda();
  const [matchEscrowPda] = getMatchEscrowPda(matchIdBn);

  const tx = await program.methods
    .cancelMatch(matchIdBn)
    .accounts({
      arenaConfig: arenaConfigPda,
      matchEscrow: matchEscrowPda,
      authority: authority.publicKey,
      playerA: new PublicKey(playerA),
      playerB: new PublicKey(playerB),
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  return { tx };
}

export async function getMatch(matchId: number): Promise<any> {
  const program = getArenaProgram();
  const matchIdBn = new BN(matchId);
  const [matchEscrowPda] = getMatchEscrowPda(matchIdBn);

  try {
    const escrow = await (program.account as any).matchEscrow.fetch(matchEscrowPda);
    return {
      matchId: (escrow.matchId as BN).toString(),
      arenaType: escrow.arenaType,
      state: escrow.state,
      playerA: (escrow.playerA as PublicKey).toBase58(),
      playerB: (escrow.playerB as PublicKey).toBase58(),
      wagerAmount: (escrow.wagerAmount as BN).toString(),
      createdAt: (escrow.createdAt as BN).toString(),
      settledAt: (escrow.settledAt as BN).toString(),
      winner: (escrow.winner as PublicKey).toBase58(),
      combatHash: Array.from(escrow.combatHash as number[]),
    };
  } catch {
    return null;
  }
}

// ── Composite operations ──

/**
 * Settle a completed match and pay the winner directly.
 * Steps: submit result → claim winnings (to authority) → transfer payout to winner wallet.
 * Returns tx signatures and payout amount.
 */
export async function settleAndPay(
  matchId: number,
  winner: string,
  combatHash: number[]
): Promise<{
  submitTx: string;
  claimTx: string;
  payoutTx: string;
  payoutLamports: string;
  feeLamports: string;
}> {
  const program = getArenaProgram();
  const authority = getAuthority();
  const connection = getConnection();
  const matchIdBn = new BN(matchId);
  const [arenaConfigPda] = getArenaConfigPda();
  const [matchEscrowPda] = getMatchEscrowPda(matchIdBn);
  const treasury = getTreasury();

  // 1. Submit result on-chain
  const submitTx = await program.methods
    .submitResult(matchIdBn, new PublicKey(winner), Buffer.from(combatHash))
    .accounts({
      arenaConfig: arenaConfigPda,
      matchEscrow: matchEscrowPda,
      oracle: authority.publicKey,
    })
    .signers([authority])
    .rpc();

  console.log(`[arena] Result submitted for match ${matchId}: tx=${submitTx}`);

  // 2. Fetch match to calculate payout
  const escrow = await (program.account as any).matchEscrow.fetch(matchEscrowPda);
  const wagerAmount = (escrow.wagerAmount as BN).toNumber();
  const totalPot = wagerAmount * 2;

  // Read fee from config
  const configAcct = await (program.account as any).arenaConfig.fetch(arenaConfigPda);
  const feeBps = configAcct.feeBps as number;
  const fee = Math.floor((totalPot * feeBps) / 10000);
  const payout = totalPot - fee;

  // 3. Claim winnings to authority
  const claimTx = await program.methods
    .claimWinningsOperated(matchIdBn)
    .accounts({
      arenaConfig: arenaConfigPda,
      matchEscrow: matchEscrowPda,
      authority: authority.publicKey,
      treasury: treasury,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  console.log(`[arena] Winnings claimed for match ${matchId}: tx=${claimTx}`);

  // 4. Transfer payout from authority to winner wallet
  const winnerPubkey = new PublicKey(winner);
  const { Transaction } = await import("@solana/web3.js");
  const transferTx = new Transaction().add(
    SystemProgram.transfer({
      fromPubkey: authority.publicKey,
      toPubkey: winnerPubkey,
      lamports: payout,
    })
  );
  transferTx.feePayer = authority.publicKey;
  transferTx.recentBlockhash = (await connection.getLatestBlockhash()).blockhash;
  transferTx.sign(authority);
  const payoutTx = await connection.sendRawTransaction(transferTx.serialize());
  await connection.confirmTransaction(payoutTx);

  console.log(`[arena] Payout ${payout} lamports sent to ${winner}: tx=${payoutTx}`);

  return {
    submitTx,
    claimTx,
    payoutTx,
    payoutLamports: payout.toString(),
    feeLamports: fee.toString(),
  };
}

/**
 * Cancel a match and refund. Used for draws/timeouts.
 */
export async function cancelAndRefund(
  matchId: number,
  playerA: string,
  playerB: string
): Promise<{ tx: string }> {
  return cancelMatch(matchId, playerA, playerB);
}

export async function getArenaConfig(): Promise<any> {
  const program = getArenaProgram();
  const [arenaConfigPda] = getArenaConfigPda();

  try {
    const cfg = await (program.account as any).arenaConfig.fetch(arenaConfigPda);
    return {
      authority: (cfg.authority as PublicKey).toBase58(),
      oracle: (cfg.oracle as PublicKey).toBase58(),
      treasury: (cfg.treasury as PublicKey).toBase58(),
      feeBps: cfg.feeBps,
      minWager: (cfg.minWager as BN).toString(),
      maxWager: (cfg.maxWager as BN).toString(),
      matchCount: (cfg.matchCount as BN).toString(),
      paused: cfg.paused,
    };
  } catch {
    return null;
  }
}

// ── Cancel match (operated) – all SOL goes to authority ──

export async function cancelMatchOperated(
  matchId: number
): Promise<{ tx: string }> {
  const program = getArenaProgram();
  const authority = getAuthority();
  const matchIdBn = new BN(matchId);
  const [arenaConfigPda] = getArenaConfigPda();
  const [matchEscrowPda] = getMatchEscrowPda(matchIdBn);

  const tx = await program.methods
    .cancelMatchOperated(matchIdBn)
    .accounts({
      arenaConfig: arenaConfigPda,
      matchEscrow: matchEscrowPda,
      authority: authority.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  return { tx };
}

// ── Marketplace escrow helpers (authority-operated) ──

function getMarketConfigPda(): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("market_config")],
    config.marketplaceProgramId
  );
}

async function deductEscrowOperated(
  amount: number,
  playerWallet: string
): Promise<string> {
  const mpProgram = getMarketplaceProgram();
  const authority = getAuthority();
  const playerPubkey = new PublicKey(playerWallet);
  const [marketConfigPda] = getMarketConfigPda();
  const [escrowPda] = getPlayerEscrowPda(playerPubkey);

  const tx = await mpProgram.methods
    .deductEscrowOperated(new BN(amount), playerPubkey)
    .accounts({
      marketConfig: marketConfigPda,
      playerEscrow: escrowPda,
      authority: authority.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  return tx;
}

async function creditEscrowOperated(
  amount: number,
  playerWallet: string
): Promise<string> {
  const mpProgram = getMarketplaceProgram();
  const authority = getAuthority();
  const playerPubkey = new PublicKey(playerWallet);
  const [marketConfigPda] = getMarketConfigPda();
  const [escrowPda] = getPlayerEscrowPda(playerPubkey);

  const tx = await mpProgram.methods
    .creditEscrowOperated(new BN(amount), playerPubkey)
    .accounts({
      marketConfig: marketConfigPda,
      playerEscrow: escrowPda,
      authority: authority.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  return tx;
}

// ── Escrow-based composite operations ──

/**
 * Fund a match from both players' escrow balances.
 * Steps:
 * 1. Deduct wager from player A's escrow → authority
 * 2. Deduct wager from player B's escrow → authority (rollback A on failure)
 * 3. Fund the match escrow from authority (rollback both on failure)
 */
export async function fundMatchFromEscrow(
  matchId: number,
  playerAWallet: string,
  playerBWallet: string,
  wagerAmountLamports: number
): Promise<{
  deductATx: string;
  deductBTx: string;
  fundTx: string;
}> {
  // 1. Deduct from player A
  const deductATx = await deductEscrowOperated(wagerAmountLamports, playerAWallet);
  console.log(`[arena] Deducted ${wagerAmountLamports} from player A (${playerAWallet}): tx=${deductATx}`);

  // 2. Deduct from player B
  let deductBTx: string;
  try {
    deductBTx = await deductEscrowOperated(wagerAmountLamports, playerBWallet);
    console.log(`[arena] Deducted ${wagerAmountLamports} from player B (${playerBWallet}): tx=${deductBTx}`);
  } catch (err: any) {
    // Rollback: credit player A back
    console.error(`[arena] Deduct player B failed, rolling back player A: ${err.message}`);
    await creditEscrowOperated(wagerAmountLamports, playerAWallet);
    throw new Error(`Failed to deduct player B escrow: ${err.message}`);
  }

  // 3. Fund the match escrow from authority
  let fundTx: string;
  try {
    const result = await fundMatchOperated(matchId);
    fundTx = result.tx;
    console.log(`[arena] Match ${matchId} funded from escrow: tx=${fundTx}`);
  } catch (err: any) {
    // Rollback: credit both players back
    console.error(`[arena] Fund match failed, rolling back both players: ${err.message}`);
    await creditEscrowOperated(wagerAmountLamports, playerAWallet);
    await creditEscrowOperated(wagerAmountLamports, playerBWallet);
    throw new Error(`Failed to fund match: ${err.message}`);
  }

  return { deductATx, deductBTx, fundTx };
}

/**
 * Settle a completed match and pay the winner into their escrow.
 * Steps:
 * 1. Submit result on-chain (oracle)
 * 2. Claim winnings to authority
 * 3. Credit winner's escrow with payout (total pot minus fee)
 */
export async function settleAndPayToEscrow(
  matchId: number,
  winner: string,
  combatHash: number[]
): Promise<{
  submitTx: string;
  claimTx: string;
  creditTx: string;
  payoutLamports: string;
  feeLamports: string;
}> {
  const program = getArenaProgram();
  const authority = getAuthority();
  const matchIdBn = new BN(matchId);
  const [arenaConfigPda] = getArenaConfigPda();
  const [matchEscrowPda] = getMatchEscrowPda(matchIdBn);
  const treasury = getTreasury();

  // 1. Submit result
  const submitTx = await program.methods
    .submitResult(matchIdBn, new PublicKey(winner), Buffer.from(combatHash))
    .accounts({
      arenaConfig: arenaConfigPda,
      matchEscrow: matchEscrowPda,
      oracle: authority.publicKey,
    })
    .signers([authority])
    .rpc();

  console.log(`[arena] Result submitted for match ${matchId}: tx=${submitTx}`);

  // 2. Fetch match to calculate payout
  const escrow = await (program.account as any).matchEscrow.fetch(matchEscrowPda);
  const wagerAmount = (escrow.wagerAmount as BN).toNumber();
  const totalPot = wagerAmount * 2;

  const configAcct = await (program.account as any).arenaConfig.fetch(arenaConfigPda);
  const feeBps = configAcct.feeBps as number;
  const fee = Math.floor((totalPot * feeBps) / 10000);
  const payout = totalPot - fee;

  // 3. Claim winnings to authority (fee goes to treasury on-chain)
  const claimTx = await program.methods
    .claimWinningsOperated(matchIdBn)
    .accounts({
      arenaConfig: arenaConfigPda,
      matchEscrow: matchEscrowPda,
      authority: authority.publicKey,
      treasury: treasury,
      systemProgram: SystemProgram.programId,
    })
    .signers([authority])
    .rpc();

  console.log(`[arena] Winnings claimed for match ${matchId}: tx=${claimTx}`);

  // 4. Credit winner's escrow with payout (retry up to 3 times)
  let creditTx: string = "";
  for (let attempt = 0; attempt < 3; attempt++) {
    try {
      creditTx = await creditEscrowOperated(payout, winner);
      console.log(`[arena] Credited ${payout} lamports to winner escrow (${winner}): tx=${creditTx}`);
      break;
    } catch (err: any) {
      console.error(`[arena] Credit winner escrow attempt ${attempt + 1} failed: ${err.message}`);
      if (attempt === 2) {
        // Log for manual recovery — authority still holds the payout SOL
        console.error(`[arena] CRITICAL: Failed to credit winner escrow after 3 attempts. Manual recovery needed for match ${matchId}, winner ${winner}, payout ${payout} lamports`);
        throw new Error(`Failed to credit winner escrow: ${err.message}`);
      }
      // Wait before retry
      await new Promise((r) => setTimeout(r, 2000 * (attempt + 1)));
    }
  }

  return {
    submitTx,
    claimTx,
    creditTx,
    payoutLamports: payout.toString(),
    feeLamports: fee.toString(),
  };
}

/**
 * Cancel a match and refund both players' escrow balances.
 * Steps:
 * 1. Cancel match (all SOL goes to authority)
 * 2. Credit player A's escrow
 * 3. Credit player B's escrow
 */
export async function cancelAndRefundToEscrow(
  matchId: number,
  playerAWallet: string,
  playerBWallet: string,
  wagerAmountLamports: number
): Promise<{
  cancelTx: string;
  refundATx: string;
  refundBTx: string;
}> {
  // 1. Cancel match — all SOL goes to authority
  const cancelResult = await cancelMatchOperated(matchId);
  console.log(`[arena] Match ${matchId} cancelled (operated): tx=${cancelResult.tx}`);

  // 2. Credit player A's escrow (retry up to 3 times)
  let refundATx: string = "";
  for (let attempt = 0; attempt < 3; attempt++) {
    try {
      refundATx = await creditEscrowOperated(wagerAmountLamports, playerAWallet);
      console.log(`[arena] Refunded ${wagerAmountLamports} to player A escrow (${playerAWallet}): tx=${refundATx}`);
      break;
    } catch (err: any) {
      console.error(`[arena] Refund player A attempt ${attempt + 1} failed: ${err.message}`);
      if (attempt === 2) {
        console.error(`[arena] CRITICAL: Failed to refund player A escrow. Manual recovery needed.`);
        throw new Error(`Failed to refund player A escrow: ${err.message}`);
      }
      await new Promise((r) => setTimeout(r, 2000 * (attempt + 1)));
    }
  }

  // 3. Credit player B's escrow (retry up to 3 times)
  let refundBTx: string = "";
  for (let attempt = 0; attempt < 3; attempt++) {
    try {
      refundBTx = await creditEscrowOperated(wagerAmountLamports, playerBWallet);
      console.log(`[arena] Refunded ${wagerAmountLamports} to player B escrow (${playerBWallet}): tx=${refundBTx}`);
      break;
    } catch (err: any) {
      console.error(`[arena] Refund player B attempt ${attempt + 1} failed: ${err.message}`);
      if (attempt === 2) {
        console.error(`[arena] CRITICAL: Failed to refund player B escrow. Manual recovery needed.`);
        throw new Error(`Failed to refund player B escrow: ${err.message}`);
      }
      await new Promise((r) => setTimeout(r, 2000 * (attempt + 1)));
    }
  }

  return {
    cancelTx: cancelResult.tx,
    refundATx,
    refundBTx,
  };
}

// Re-export escrow balance query
export { getEscrowBalance } from "./darkAuction";
