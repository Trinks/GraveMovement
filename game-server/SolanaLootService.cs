using System;
using System.Security.Cryptography;
using System.Text;
using System.Threading.Tasks;
using Core;
using UnityEngine;

namespace Server.Solana
{
    /// <summary>
    /// Server-side service that registers VrfLootProvider delegates,
    /// bridging Core loot generation with the Solana sidecar.
    /// Attach to a persistent GameObject on the dedicated server.
    /// </summary>
    public class SolanaLootService : MonoBehaviour
    {
        private SolanaLootApiClient _lootApi;

        private void Awake()
        {
            SolanaConfig.Initialize();

            if (!SolanaConfig.Enabled)
            {
                Debug.Log("[SolanaLootService] Solana disabled, VRF loot will not be available");
                return;
            }

            _lootApi = new SolanaLootApiClient(SolanaConfig.ApiClient);

            // Register delegates so Core assembly can request VRF
            VrfLootProvider.RequestVrfSeed = HandleRequestVrfSeed;
            VrfLootProvider.PublishReceipt = HandlePublishReceipt;

            Debug.Log("[SolanaLootService] VRF loot provider registered");
        }

        private void OnDestroy()
        {
            VrfLootProvider.RequestVrfSeed = null;
            VrfLootProvider.PublishReceipt = null;
            SolanaConfig.Dispose();
        }

        // =====================================================================
        // VRF Seed Request
        // =====================================================================

        private void HandleRequestVrfSeed(
            int creatureId, string creatureName, int[] eligiblePlayerIds,
            Action<int, byte[]> onSeedReceived, Action<string> onError)
        {
            _ = RequestVrfSeedAsync(creatureId, creatureName, eligiblePlayerIds, onSeedReceived, onError);
        }

        private const int VRF_TIMEOUT_MS = 10000;

        private async System.Threading.Tasks.Task RequestVrfSeedAsync(
            int creatureId, string creatureName, int[] eligiblePlayerIds,
            Action<int, byte[]> onSeedReceived, Action<string> onError)
        {
            try
            {
                var vrfTask = _lootApi.RequestVrfAndWaitAsync(
                    creatureId, creatureName, eligiblePlayerIds);
                var timeoutTask = System.Threading.Tasks.Task.Delay(VRF_TIMEOUT_MS);

                var completed = await System.Threading.Tasks.Task.WhenAny(vrfTask, timeoutTask);

                if (completed == timeoutTask)
                {
                    Debug.LogWarning($"[SolanaLootService] VRF request timed out after {VRF_TIMEOUT_MS}ms for {creatureName}. Falling back to standard RNG.");
                    MainThreadDispatcher.Enqueue(() =>
                        onError?.Invoke("VRF request timed out"));
                    return;
                }

                var result = await vrfTask;

                if (result == null || !result.fulfilled || result.randomness == null)
                {
                    MainThreadDispatcher.Enqueue(() =>
                        onError?.Invoke("VRF request failed or not fulfilled"));
                    return;
                }

                // Convert int[] randomness to byte[]
                byte[] seed = new byte[result.randomness.Length];
                for (int i = 0; i < result.randomness.Length; i++)
                    seed[i] = (byte)result.randomness[i];

                MainThreadDispatcher.Enqueue(() =>
                    onSeedReceived?.Invoke(result.requestId, seed));
            }
            catch (Exception ex)
            {
                Debug.LogError($"[SolanaLootService] VRF seed request failed: {ex.Message}");
                MainThreadDispatcher.Enqueue(() =>
                    onError?.Invoke(ex.Message));
            }
        }

        // =====================================================================
        // Receipt Publishing (fire-and-forget)
        // =====================================================================

        private void HandlePublishReceipt(
            int vrfRequestId, byte[] lootTableHash, byte[] generatedLootHash,
            LootReceiptItem[] items, long coins, int distributionMode,
            LootReceiptAssignment[] assignments, bool isFallbackRng)
        {
            _ = PublishReceiptAsync(vrfRequestId, lootTableHash, generatedLootHash,
                items, coins, distributionMode, assignments, isFallbackRng);
        }

        private async System.Threading.Tasks.Task PublishReceiptAsync(
            int vrfRequestId, byte[] lootTableHash, byte[] generatedLootHash,
            LootReceiptItem[] items, long coins, int distributionMode,
            LootReceiptAssignment[] assignments, bool isFallbackRng)
        {
            try
            {
                var body = new SolanaLootApiClient.PublishReceiptBody
                {
                    requestId = vrfRequestId,
                    lootTableHash = ByteArrayToIntArray(lootTableHash),
                    generatedLootHash = ByteArrayToIntArray(generatedLootHash),
                    items = ConvertItems(items),
                    coins = coins,
                    distributionMode = distributionMode,
                    assignments = ConvertAssignments(assignments),
                    isFallbackRng = isFallbackRng
                };

                var result = await _lootApi.PublishLootReceiptAsync(body);

                if (result != null)
                    Debug.Log($"[SolanaLootService] Loot receipt published: {result.receiptId} tx={result.tx}");
                else
                    Debug.LogWarning("[SolanaLootService] Failed to publish loot receipt");
            }
            catch (Exception ex)
            {
                Debug.LogError($"[SolanaLootService] Publish receipt error: {ex.Message}");
            }
        }

        // =====================================================================
        // Helpers
        // =====================================================================

        public static byte[] ComputeSha256(string input)
        {
            using var sha = SHA256.Create();
            return sha.ComputeHash(Encoding.UTF8.GetBytes(input));
        }

        private static int[] ByteArrayToIntArray(byte[] bytes)
        {
            if (bytes == null) return Array.Empty<int>();
            int[] result = new int[bytes.Length];
            for (int i = 0; i < bytes.Length; i++)
                result[i] = bytes[i];
            return result;
        }

        private static SolanaLootApiClient.ReceiptItem[] ConvertItems(LootReceiptItem[] items)
        {
            if (items == null) return Array.Empty<SolanaLootApiClient.ReceiptItem>();
            var result = new SolanaLootApiClient.ReceiptItem[items.Length];
            for (int i = 0; i < items.Length; i++)
            {
                result[i] = new SolanaLootApiClient.ReceiptItem
                {
                    itemId = items[i].itemId,
                    quantity = items[i].quantity,
                    poolIndex = items[i].poolIndex,
                    rollValue = items[i].rollValue
                };
            }
            return result;
        }

        private static SolanaLootApiClient.ReceiptAssignment[] ConvertAssignments(LootReceiptAssignment[] assignments)
        {
            if (assignments == null) return Array.Empty<SolanaLootApiClient.ReceiptAssignment>();
            var result = new SolanaLootApiClient.ReceiptAssignment[assignments.Length];
            for (int i = 0; i < assignments.Length; i++)
            {
                result[i] = new SolanaLootApiClient.ReceiptAssignment
                {
                    characterId = assignments[i].characterId,
                    itemId = assignments[i].itemId,
                    quantity = assignments[i].quantity,
                    reason = assignments[i].reason
                };
            }
            return result;
        }
    }
}
