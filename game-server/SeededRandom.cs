using System;
using System.Runtime.CompilerServices;

namespace Core
{
    /// <summary>
    /// Deterministic PRNG using xoshiro256** algorithm (Blackman & Vigna, 2019).
    /// Seeded from a 32-byte VRF output for provably fair loot generation.
    ///
    /// This is a drop-in replacement for UnityEngine.Random in the loot pipeline.
    /// It is intentionally free of any Unity dependencies so client-side verification
    /// tools (console apps, web) can run identical verification without Unity.
    ///
    /// Thread safety: NOT thread-safe. Each call site should use its own instance.
    /// </summary>
    public sealed class SeededRandom
    {
        // 256-bit internal state: four 64-bit words
        private ulong _s0;
        private ulong _s1;
        private ulong _s2;
        private ulong _s3;

        // Tracks how many raw Next() calls have been consumed.
        // Published in LootReceipt for exhaustive audit.
        private int _callCount;

        public int CallCount => _callCount;

        /// <summary>
        /// Construct from a 32-byte VRF seed.
        /// The seed bytes are interpreted as four little-endian uint64 state words.
        /// </summary>
        /// <param name="vrfSeed">Exactly 32 bytes from VRF output.</param>
        /// <exception cref="ArgumentException">If seed is not 32 bytes.</exception>
        public SeededRandom(byte[] vrfSeed)
        {
            if (vrfSeed == null || vrfSeed.Length != 32)
                throw new ArgumentException("VRF seed must be exactly 32 bytes.", nameof(vrfSeed));

            // Read four 64-bit little-endian words from the seed bytes.
            _s0 = ReadUInt64LE(vrfSeed, 0);
            _s1 = ReadUInt64LE(vrfSeed, 8);
            _s2 = ReadUInt64LE(vrfSeed, 16);
            _s3 = ReadUInt64LE(vrfSeed, 24);

            // xoshiro256** must not have all-zero state.
            // A valid VRF output is never all-zeros, but guard defensively.
            if (_s0 == 0 && _s1 == 0 && _s2 == 0 && _s3 == 0)
            {
                _s0 = 0x9E3779B97F4A7C15UL; // golden ratio constant
            }
        }

        /// <summary>
        /// Construct from a hex-encoded 64-character VRF seed string (for deserializing receipts).
        /// </summary>
        public SeededRandom(string vrfSeedHex)
            : this(HexToBytes(vrfSeedHex)) { }

        /// <summary>
        /// Returns a uniformly distributed float in [0, 1).
        /// Equivalent to UnityEngine.Random.value.
        /// </summary>
        [MethodImpl(MethodImplOptions.AggressiveInlining)]
        public float Value()
        {
            // Use top 53 bits of a 64-bit raw output for double precision,
            // then cast to float. This avoids float rounding bias.
            ulong raw = Next();
            // >> 11 gives 53 significant bits; multiply by 2^-53 to get [0,1)
            double d = (raw >> 11) * (1.0 / (1UL << 53));
            return (float)d;
        }

        /// <summary>
        /// Returns a uniformly distributed int in [minInclusive, maxInclusive].
        /// Note: inclusive upper bound (unlike Unity's Random.Range(int,int) which is exclusive).
        ///
        /// Uses unbiased rejection sampling (Lemire's nearly divisionless method)
        /// to avoid modulo bias on non-power-of-two ranges.
        /// </summary>
        public int Range(int minInclusive, int maxInclusive)
        {
            if (minInclusive == maxInclusive) return minInclusive;
            if (minInclusive > maxInclusive)
            {
                // Mirror Unity's behavior: swap silently
                (minInclusive, maxInclusive) = (maxInclusive, minInclusive);
            }

            uint range = (uint)(maxInclusive - minInclusive) + 1u;
            return minInclusive + (int)UnbiasedUInt32(range);
        }

        // -----------------------------------------------------------------------
        // Core xoshiro256** generator
        // -----------------------------------------------------------------------

        /// <summary>
        /// Core xoshiro256** step. Returns the next raw 64-bit output.
        /// </summary>
        [MethodImpl(MethodImplOptions.AggressiveInlining)]
        private ulong Next()
        {
            _callCount++;

            // xoshiro256** output function: s1 * 5, rotl 7, * 9
            ulong result = RotateLeft(_s1 * 5, 7) * 9;

            // xoshiro256** state update
            ulong t = _s1 << 17;
            _s2 ^= _s0;
            _s3 ^= _s1;
            _s1 ^= _s2;
            _s0 ^= _s3;
            _s2 ^= t;
            _s3 = RotateLeft(_s3, 45);

            return result;
        }

        /// <summary>
        /// Lemire's nearly divisionless unbiased integer range [0, bound).
        /// Avoids the modulo bias of the naive (raw % bound) approach.
        /// Reference: https://lemire.me/blog/2019/06/06/nearly-divisionless-random-integer-generation-on-various-systems/
        /// </summary>
        private uint UnbiasedUInt32(uint bound)
        {
            if (bound <= 1) return 0;

            ulong x = (uint)Next() * (ulong)bound;
            uint leftover = (uint)x;

            if (leftover < bound)
            {
                // Rejection threshold: (2^32 - bound) % bound = (-bound % bound)
                uint threshold = (uint)(-(int)bound) % bound;
                while (leftover < threshold)
                {
                    x = (uint)Next() * (ulong)bound;
                    leftover = (uint)x;
                }
            }

            return (uint)(x >> 32);
        }

        // -----------------------------------------------------------------------
        // Helpers
        // -----------------------------------------------------------------------

        [MethodImpl(MethodImplOptions.AggressiveInlining)]
        private static ulong RotateLeft(ulong value, int count)
            => (value << count) | (value >> (64 - count));

        private static ulong ReadUInt64LE(byte[] buf, int offset)
        {
            return (ulong)buf[offset]
                | ((ulong)buf[offset + 1] << 8)
                | ((ulong)buf[offset + 2] << 16)
                | ((ulong)buf[offset + 3] << 24)
                | ((ulong)buf[offset + 4] << 32)
                | ((ulong)buf[offset + 5] << 40)
                | ((ulong)buf[offset + 6] << 48)
                | ((ulong)buf[offset + 7] << 56);
        }

        public static byte[] HexToBytes(string hex)
        {
            if (hex.Length % 2 != 0)
                throw new ArgumentException("Hex string must have even length.");
            byte[] bytes = new byte[hex.Length / 2];
            for (int i = 0; i < bytes.Length; i++)
                bytes[i] = Convert.ToByte(hex.Substring(i * 2, 2), 16);
            return bytes;
        }

        public string SeedHex { get; private set; } // set after construction via factory

        // Factory method that also records the hex seed for logging
        public static SeededRandom FromVrfOutput(byte[] vrfOutput)
        {
            var rng = new SeededRandom(vrfOutput);
            rng.SeedHex = BitConverter.ToString(vrfOutput).Replace("-", "").ToLower();
            return rng;
        }
    }
}
