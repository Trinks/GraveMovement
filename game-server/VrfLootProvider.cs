using System;

namespace Core
{
    /// <summary>
    /// Static delegate bridge for VRF loot operations.
    /// Lives in Core assembly so Creature.GenerateLoot() can call it.
    /// Server assembly registers implementations at startup.
    /// </summary>
    public static class VrfLootProvider
    {
        /// <summary>
        /// Request a VRF seed from the Solana sidecar.
        /// Parameters: creatureId, creatureName, eligiblePlayerIds, onSeedReceived(requestId, seed), onError
        /// </summary>
        public delegate void RequestVrfSeedDelegate(
            int creatureId, string creatureName, int[] eligiblePlayerIds,
            Action<int, byte[]> onSeedReceived, Action<string> onError);

        /// <summary>
        /// Publish a loot receipt on-chain after loot generation.
        /// Fire-and-forget; errors are logged but don't affect gameplay.
        /// </summary>
        public delegate void PublishReceiptDelegate(
            int vrfRequestId, byte[] lootTableHash, byte[] generatedLootHash,
            LootReceiptItem[] items, long coins, int distributionMode,
            LootReceiptAssignment[] assignments, bool isFallbackRng);

        /// <summary>
        /// Set by Server assembly's SolanaLootService on startup.
        /// Null when Solana integration is disabled or not running.
        /// </summary>
        public static RequestVrfSeedDelegate RequestVrfSeed;

        /// <summary>
        /// Set by Server assembly's SolanaLootService on startup.
        /// </summary>
        public static PublishReceiptDelegate PublishReceipt;

        /// <summary>Whether VRF loot is available (delegate registered).</summary>
        public static bool IsAvailable => RequestVrfSeed != null;

        // =====================================================================
        // Dark Bank / NFT Vault Delegates
        // Set by Server assembly's SolanaVaultService on startup.
        // =====================================================================

        /// <summary>
        /// Mint a Metaplex Core NFT for a withdrawn game item.
        /// Parameters: playerWallet, itemId, itemName, vrfRequestId, metadata,
        ///             onSuccess(mintAddress, txSignature), onError(message)
        /// </summary>
        public delegate void MintNftDelegate(
            string playerWallet, int itemId, string itemName, int vrfRequestId,
            NftItemMetadata metadata,
            Action<string, string> onSuccess, Action<string> onError);

        /// <summary>
        /// Burn a Metaplex Core NFT when depositing back into the game.
        /// Parameters: mintAddress, onSuccess(itemId, itemName), onError(message)
        /// </summary>
        public delegate void BurnNftDelegate(
            string mintAddress,
            Action<int, string> onSuccess, Action<string> onError);

        /// <summary>
        /// Query a wallet for game-collection NFTs.
        /// Parameters: walletAddress, onSuccess(nfts), onError(message)
        /// </summary>
        public delegate void QueryWalletNftsDelegate(
            string walletAddress,
            Action<WalletNftInfo[]> onSuccess, Action<string> onError);

        /// <summary>
        /// Verify a wallet address against the backend's hashed wallet_mappings.
        /// Parameters: accountId, walletAddress, onSuccess(valid), onError(message)
        /// </summary>
        public delegate void VerifyWalletDelegate(
            long accountId, string walletAddress,
            Action<bool> onResult, Action<string> onError);

        public static MintNftDelegate MintNft;
        public static BurnNftDelegate BurnNft;
        public static QueryWalletNftsDelegate QueryWalletNfts;
        public static VerifyWalletDelegate VerifyWallet;

        /// <summary>Whether the NFT vault is available (delegate registered).</summary>
        public static bool IsVaultAvailable => MintNft != null;
    }

    // =====================================================================
    // VRF Loot Structs
    // =====================================================================

    [Serializable]
    public struct LootReceiptItem
    {
        public int itemId;
        public int quantity;
        public int poolIndex;
        public int rollValue;
    }

    [Serializable]
    public struct LootReceiptAssignment
    {
        public int characterId;
        public int itemId;
        public int quantity;
        public int reason; // 0=FFA, 1=Personal, 2=RoundRobin, 3=NeedRoll, 4=MasterLoot
    }

    // =====================================================================
    // NFT Vault Structs
    // =====================================================================

    /// <summary>
    /// Metadata sent to the sidecar when minting a game item as an NFT.
    /// </summary>
    [Serializable]
    public struct NftItemMetadata
    {
        public int itemId;
        public string itemName;
        public int vrfRequestId;
        public string rarity;
        public string category;
        public long dropTimestamp;

        /// <summary>Item description from ItemInfo, used for NFT description on-chain.</summary>
        public string description;

        /// <summary>
        /// Off-chain metadata URI (e.g. hosted JSON conforming to Metaplex standard).
        /// If empty, the sidecar will leave the URI blank on the asset.
        /// </summary>
        public string uri;

        /// <summary>Whether the item is a piece of equipment (weapon/armor/accessory).</summary>
        public bool isEquipment;

        /// <summary>Equipment slot (e.g. "Weapon_MainHand", "Chest"). Empty if not equipment.</summary>
        public string equipmentSlot;

        /// <summary>Minimum level required to use/equip the item. 0 if none.</summary>
        public int requiredLevel;

        /// <summary>Whether the item is bound and non-tradeable in-game.</summary>
        public bool isBound;
    }

    /// <summary>
    /// Represents a game-collection NFT found in a player's wallet.
    /// </summary>
    [Serializable]
    public struct WalletNftInfo
    {
        public string mintAddress;
        public int itemId;
        public string itemName;
        public string imageUri;
    }
}
