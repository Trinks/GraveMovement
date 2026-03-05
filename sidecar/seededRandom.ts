/**
 * Deterministic PRNG using xoshiro256** algorithm (Blackman & Vigna, 2019).
 * TypeScript port of the C# SeededRandom class for provably fair verification.
 *
 * Uses BigInt for 64-bit unsigned arithmetic (JavaScript has no native uint64).
 * Must produce IDENTICAL output to the C# version for the same seed.
 */

const MASK_64 = 0xFFFFFFFFFFFFFFFFn; // 2^64 - 1
const MASK_32 = 0xFFFFFFFFn;         // 2^32 - 1

function rotateLeft(value: bigint, count: number): bigint {
  return ((value << BigInt(count)) | (value >> BigInt(64 - count))) & MASK_64;
}

function readUInt64LE(buf: Buffer, offset: number): bigint {
  let result = 0n;
  for (let i = 0; i < 8; i++) {
    result |= BigInt(buf[offset + i]) << BigInt(i * 8);
  }
  return result;
}

function hexToBytes(hex: string): Buffer {
  if (hex.length % 2 !== 0) {
    throw new Error("Hex string must have even length.");
  }
  return Buffer.from(hex, "hex");
}

export class SeededRandom {
  // 256-bit internal state: four 64-bit words
  private _s0: bigint;
  private _s1: bigint;
  private _s2: bigint;
  private _s3: bigint;
  private _callCount: number = 0;

  /**
   * Construct from a 32-byte VRF seed (Buffer) or a 64-character hex string.
   */
  constructor(seed: Buffer | string) {
    const buf: Buffer = typeof seed === "string" ? hexToBytes(seed) : seed;

    if (buf.length !== 32) {
      throw new Error("VRF seed must be exactly 32 bytes.");
    }

    // Read four 64-bit little-endian words from the seed bytes.
    this._s0 = readUInt64LE(buf, 0);
    this._s1 = readUInt64LE(buf, 8);
    this._s2 = readUInt64LE(buf, 16);
    this._s3 = readUInt64LE(buf, 24);

    // xoshiro256** must not have all-zero state.
    // A valid VRF output is never all-zeros, but guard defensively.
    if (this._s0 === 0n && this._s1 === 0n && this._s2 === 0n && this._s3 === 0n) {
      this._s0 = 0x9E3779B97F4A7C15n; // golden ratio constant
    }
  }

  get callCount(): number {
    return this._callCount;
  }

  /**
   * Returns a uniformly distributed float in [0, 1).
   * Equivalent to C# SeededRandom.Value().
   *
   * Uses top 53 bits of a 64-bit raw output for double precision,
   * then the double is narrowed to float precision to match C#'s (float)d cast.
   */
  value(): number {
    const raw = this.next();
    // >> 11 gives 53 significant bits; multiply by 2^-53 to get [0,1)
    const d = Number(raw >> 11n) / Number(1n << 53n);
    // C# casts double to float — Math.fround replicates single-precision rounding
    return Math.fround(d);
  }

  /**
   * Returns a uniformly distributed int in [minInclusive, maxInclusive].
   * Note: inclusive upper bound (unlike many Random.Range implementations).
   *
   * Uses Lemire's nearly divisionless unbiased method.
   */
  range(minInclusive: number, maxInclusive: number): number {
    if (minInclusive === maxInclusive) return minInclusive;
    if (minInclusive > maxInclusive) {
      // Swap silently (mirrors Unity/C# behavior)
      [minInclusive, maxInclusive] = [maxInclusive, minInclusive];
    }

    const rangeSize = (maxInclusive - minInclusive + 1) >>> 0; // uint32
    return minInclusive + this.unbiasedUInt32(rangeSize);
  }

  // ── Core xoshiro256** generator ──

  /**
   * Core xoshiro256** step. Returns the next raw 64-bit output as BigInt.
   */
  private next(): bigint {
    this._callCount++;

    // xoshiro256** output function: s1 * 5, rotl 7, * 9
    const result = (rotateLeft((this._s1 * 5n) & MASK_64, 7) * 9n) & MASK_64;

    // xoshiro256** state update
    const t = (this._s1 << 17n) & MASK_64;
    this._s2 = (this._s2 ^ this._s0) & MASK_64;
    this._s3 = (this._s3 ^ this._s1) & MASK_64;
    this._s1 = (this._s1 ^ this._s2) & MASK_64;
    this._s0 = (this._s0 ^ this._s3) & MASK_64;
    this._s2 = (this._s2 ^ t) & MASK_64;
    this._s3 = rotateLeft(this._s3, 45);

    return result;
  }

  /**
   * Lemire's nearly divisionless unbiased integer range [0, bound).
   * Avoids the modulo bias of the naive (raw % bound) approach.
   *
   * Matches the C# implementation exactly:
   *   x = (uint)Next() * (ulong)bound
   *   leftover = (uint)x
   *   threshold = (uint)(-(int)bound) % bound  =>  (2^32 - bound) % bound
   */
  private unbiasedUInt32(bound: number): number {
    if (bound <= 1) return 0;

    const boundBig = BigInt(bound >>> 0);

    // Cast raw 64-bit output to uint32 (lower 32 bits), then multiply by bound
    let x = (this.next() & MASK_32) * boundBig;
    let leftover = x & MASK_32;

    if (leftover < boundBig) {
      // Rejection threshold: (2^32 - bound) % bound
      const threshold = ((0x100000000n - boundBig) % boundBig);
      while (leftover < threshold) {
        x = (this.next() & MASK_32) * boundBig;
        leftover = x & MASK_32;
      }
    }

    return Number(x >> 32n);
  }
}
