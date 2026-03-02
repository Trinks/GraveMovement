import { PublicKey } from "@solana/web3.js";
import { createUmi } from "@metaplex-foundation/umi-bundle-defaults";
import {
  keypairIdentity,
  publicKey,
  generateSigner,
} from "@metaplex-foundation/umi";
import {
  createCollection,
  create,
  fetchCollection,
  fetchAssetsByCollection,
  fetchAssetsByOwner,
  burn,
  fetchAsset,
} from "@metaplex-foundation/mpl-core";
import { config, loadKeypairFromFile } from "../config";
import { getAuthority } from "./solana";

// ── Umi instance (lazy-initialized) ──

let _umi: ReturnType<typeof createUmi> | null = null;

function getUmi() {
  if (_umi) return _umi;

  const authority = getAuthority();
  const umi = createUmi(config.solanaRpcUrl);

  // Convert web3.js Keypair to Umi keypair identity
  const umiKeypair = umi.eddsa.createKeypairFromSecretKey(authority.secretKey);
  umi.use(keypairIdentity(umiKeypair));

  _umi = umi;
  return _umi;
}

// ── Collection address ──

let _collectionAddress: string = config.gameNftCollection;

function getCollectionAddress(): string {
  if (!_collectionAddress) {
    throw new Error(
      "Game NFT collection not initialized. Call /initialize-collection first or set GAME_NFT_COLLECTION env var."
    );
  }
  return _collectionAddress;
}

// ── Service functions ──

export async function initializeCollection(
  name: string,
  uri: string
): Promise<{ collectionAddress: string; tx: string }> {
  const umi = getUmi();
  const collectionSigner = generateSigner(umi);

  const tx = await createCollection(umi, {
    collection: collectionSigner,
    name,
    uri,
  }).sendAndConfirm(umi);

  const collectionAddress = collectionSigner.publicKey.toString();
  _collectionAddress = collectionAddress;

  console.log(`[vault] Collection created: ${collectionAddress}`);

  return {
    collectionAddress,
    tx: Buffer.from(tx.signature).toString("base64"),
  };
}

export interface MintNftMetadata {
  rarity?: string;
  category?: string;
  dropTimestamp?: number;
  description?: string;
  uri?: string;
  isEquipment?: boolean;
  equipmentSlot?: string;
  requiredLevel?: number;
  isBound?: boolean;
}

export async function mintNft(
  playerWallet: string,
  itemId: number,
  itemName: string,
  vrfRequestId: number,
  metadata: MintNftMetadata
): Promise<{ mintAddress: string; tx: string }> {
  const umi = getUmi();
  const collectionAddress = getCollectionAddress();
  const assetSigner = generateSigner(umi);

  const collection = await fetchCollection(
    umi,
    publicKey(collectionAddress)
  );

  // Build the on-chain attribute list
  const attributeList = [
    { key: "itemId", value: String(itemId) },
    { key: "itemName", value: itemName },
    { key: "vrfRequestId", value: String(vrfRequestId) },
    { key: "rarity", value: metadata.rarity || "Common" },
    { key: "category", value: metadata.category || "Misc" },
    {
      key: "dropTimestamp",
      value: String(metadata.dropTimestamp || Math.floor(Date.now() / 1000)),
    },
  ];

  // Add optional rich attributes from ItemInfo when available
  if (metadata.description) {
    attributeList.push({ key: "description", value: metadata.description });
  }
  if (metadata.isEquipment) {
    attributeList.push({ key: "isEquipment", value: "true" });
    if (metadata.equipmentSlot) {
      attributeList.push({ key: "equipmentSlot", value: metadata.equipmentSlot });
    }
  }
  if (metadata.requiredLevel && metadata.requiredLevel > 0) {
    attributeList.push({ key: "requiredLevel", value: String(metadata.requiredLevel) });
  }
  if (metadata.isBound) {
    attributeList.push({ key: "isBound", value: "true" });
  }

  const tx = await create(umi, {
    asset: assetSigner,
    name: itemName,
    uri: metadata.uri || "",
    collection: collection,
    owner: publicKey(playerWallet),
    plugins: [
      {
        type: "Attributes",
        attributeList,
      },
    ],
  }).sendAndConfirm(umi);

  const mintAddress = assetSigner.publicKey.toString();
  console.log(`[vault] NFT minted: ${mintAddress} for wallet ${playerWallet} (item: ${itemName})`);

  return {
    mintAddress,
    tx: Buffer.from(tx.signature).toString("base64"),
  };
}

export async function burnNft(
  mintAddress: string
): Promise<{ itemId: number; itemName: string; tx: string }> {
  const umi = getUmi();
  const collectionAddress = getCollectionAddress();

  // Fetch the asset to read its attributes before burning
  const asset = await fetchAsset(umi, publicKey(mintAddress));

  // Extract itemId and itemName from the Attributes plugin
  let itemIdStr = "unknown";
  let itemName = "unknown";

  if (asset.attributes) {
    for (const attr of asset.attributes.attributeList) {
      if (attr.key === "itemId") itemIdStr = attr.value;
      if (attr.key === "itemName") itemName = attr.value;
    }
  }

  // Parse itemId string to number (stored as string in Metaplex Attributes)
  const itemId = parseInt(itemIdStr, 10);
  if (isNaN(itemId)) {
    throw new Error(
      `NFT ${mintAddress} has invalid itemId attribute: "${itemIdStr}"`
    );
  }

  const collection = await fetchCollection(
    umi,
    publicKey(collectionAddress)
  );

  const tx = await burn(umi, {
    asset: asset,
    collection: collection,
  }).sendAndConfirm(umi);

  console.log(`[vault] NFT burned: ${mintAddress} (item: ${itemName})`);

  return {
    itemId,
    itemName,
    tx: Buffer.from(tx.signature).toString("base64"),
  };
}

export interface WalletNftEntry {
  mintAddress: string;
  itemId: number;
  itemName: string;
  imageUri: string;
}

export async function queryWalletNfts(
  walletAddress: string
): Promise<WalletNftEntry[]> {
  const umi = getUmi();
  const collectionAddress = getCollectionAddress();

  // Fetch assets owned by the wallet (much smaller result set than scanning entire collection).
  // Then filter client-side to only include assets from our game collection.
  console.log(`[vault] Querying assets for wallet ${walletAddress}, collection ${collectionAddress}`);

  let assets;
  try {
    assets = await fetchAssetsByOwner(umi, publicKey(walletAddress));
  } catch (err: any) {
    console.error(`[vault] fetchAssetsByOwner failed: ${err.message}`);
    // Fallback: try fetching by collection and filter by owner
    try {
      console.log(`[vault] Fallback: fetching by collection`);
      const allAssets = await fetchAssetsByCollection(umi, publicKey(collectionAddress));
      assets = allAssets.filter(a => a.owner.toString() === walletAddress);
    } catch (err2: any) {
      console.error(`[vault] fetchAssetsByCollection fallback also failed: ${err2.message}`);
      return [];
    }
  }

  const results: WalletNftEntry[] = [];

  for (const asset of assets) {
    // Filter to assets belonging to our game collection
    const ua = asset.updateAuthority;
    if (
      ua.type !== "Collection" ||
      !ua.address ||
      ua.address.toString() !== collectionAddress
    ) {
      continue;
    }

    let itemIdStr = "";
    let itemName = "";
    let imageUri = asset.uri || "";

    if (asset.attributes) {
      for (const attr of asset.attributes.attributeList) {
        if (attr.key === "itemId") itemIdStr = attr.value;
        if (attr.key === "itemName") itemName = attr.value;
      }
    }

    // Parse itemId string to number (stored as string in Metaplex Attributes)
    const itemId = parseInt(itemIdStr, 10);
    if (isNaN(itemId)) {
      console.warn(`[vault] Skipping NFT ${asset.publicKey.toString()} with invalid itemId: "${itemIdStr}"`);
      continue;
    }

    results.push({
      mintAddress: asset.publicKey.toString(),
      itemId,
      itemName: itemName || asset.name,
      imageUri,
    });
  }

  console.log(`[vault] Queried wallet ${walletAddress}: ${results.length} NFTs from collection`);
  return results;
}
