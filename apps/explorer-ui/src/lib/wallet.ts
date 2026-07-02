import { getPublicKeyAsync, signAsync, utils } from "@noble/ed25519";
import { sha256 } from "@noble/hashes/sha2.js";
import type { Transfer } from "./api";

const TRANSFER_DOMAIN = "zk-rollup-transfer-v1";
const CHAIN_ID = "zk-rollup-local-v1";

export type RollupKeypair = {
  secret_key_base64: string;
  public_key_base64: string;
};

function bytesToBase64(bytes: Uint8Array) {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

function base64ToBytes(value: string) {
  const binary = atob(value);
  return Uint8Array.from(binary, (character) => character.charCodeAt(0));
}

function assertLength(bytes: Uint8Array, expected: number, label: string) {
  if (bytes.length !== expected) {
    throw new Error(`${label} must decode to ${expected} bytes`);
  }
}

export async function createKeypair(): Promise<RollupKeypair> {
  const secret = utils.randomSecretKey();
  const publicKey = await getPublicKeyAsync(secret);
  return {
    secret_key_base64: bytesToBase64(secret),
    public_key_base64: bytesToBase64(publicKey),
  };
}

export async function importKeypair(json: string): Promise<RollupKeypair> {
  const parsed = JSON.parse(json) as Partial<RollupKeypair>;
  if (typeof parsed.secret_key_base64 !== "string") {
    throw new Error("Key file is missing secret_key_base64");
  }
  const secret = base64ToBytes(parsed.secret_key_base64);
  assertLength(secret, 32, "Secret key");
  const publicKey = await getPublicKeyAsync(secret);
  const publicKeyBase64 = bytesToBase64(publicKey);
  if (
    typeof parsed.public_key_base64 === "string" &&
    parsed.public_key_base64 !== publicKeyBase64
  ) {
    throw new Error("Key file public key does not match its secret key");
  }
  return {
    secret_key_base64: parsed.secret_key_base64,
    public_key_base64: publicKeyBase64,
  };
}

export function addressFromPublicKey(publicKeyBase64: string) {
  const publicKey = base64ToBytes(publicKeyBase64);
  assertLength(publicKey, 32, "Public key");
  return bytesToBase64(sha256(publicKey).slice(0, 20));
}

export async function signTransfer(
  keypair: RollupKeypair,
  to: string,
  amount: number,
  nonce: number,
): Promise<Transfer> {
  if (!Number.isSafeInteger(amount) || amount <= 0) {
    throw new Error("Amount must be a positive safe integer");
  }
  if (!Number.isSafeInteger(nonce) || nonce < 0) {
    throw new Error("Nonce must be a non-negative safe integer");
  }

  const secret = base64ToBytes(keypair.secret_key_base64);
  assertLength(secret, 32, "Secret key");
  const from = addressFromPublicKey(keypair.public_key_base64);
  // Property order is part of the Rust serde_json signing payload.
  const payload = JSON.stringify({
    domain: TRANSFER_DOMAIN,
    chain_id: CHAIN_ID,
    from,
    to,
    amount,
    nonce,
    public_key_base64: keypair.public_key_base64,
  });
  const signature = await signAsync(new TextEncoder().encode(payload), secret);

  return {
    from,
    to,
    amount,
    nonce,
    public_key_base64: keypair.public_key_base64,
    signature_base64: bytesToBase64(signature),
  };
}

export function downloadKeypair(keypair: RollupKeypair) {
  const blob = new Blob([`${JSON.stringify(keypair, null, 2)}\n`], {
    type: "application/json",
  });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = `rollup-key-${addressFromPublicKey(keypair.public_key_base64)}.json`;
  anchor.click();
  URL.revokeObjectURL(url);
}
