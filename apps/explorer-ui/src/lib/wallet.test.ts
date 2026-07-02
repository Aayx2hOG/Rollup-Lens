import { describe, expect, test } from "bun:test";
import { verifyAsync } from "@noble/ed25519";
import {
  addressFromPublicKey,
  createKeypair,
  importKeypair,
  signTransfer,
} from "./wallet";

function fromBase64(value: string) {
  return Uint8Array.from(atob(value), (character) => character.charCodeAt(0));
}

describe("browser rollup wallet", () => {
  test("creates and reimports a CLI-compatible keypair", async () => {
    const keypair = await createKeypair();
    const imported = await importKeypair(JSON.stringify(keypair));

    expect(imported).toEqual(keypair);
    expect(fromBase64(keypair.secret_key_base64)).toHaveLength(32);
    expect(fromBase64(keypair.public_key_base64)).toHaveLength(32);
    expect(fromBase64(addressFromPublicKey(keypair.public_key_base64))).toHaveLength(20);
  });

  test("signs the exact chain-core transfer payload", async () => {
    const keypair = await createKeypair();
    const transfer = await signTransfer(keypair, "receiver", 12, 3);
    const payload = JSON.stringify({
      domain: "zk-rollup-transfer-v1",
      chain_id: "zk-rollup-local-v1",
      from: transfer.from,
      to: transfer.to,
      amount: 12,
      nonce: 3,
      public_key_base64: transfer.public_key_base64,
    });

    expect(
      await verifyAsync(
        fromBase64(transfer.signature_base64),
        new TextEncoder().encode(payload),
        fromBase64(transfer.public_key_base64),
      ),
    ).toBe(true);
  });

  test("rejects a mismatched key file", async () => {
    const first = await createKeypair();
    const second = await createKeypair();

    await expect(
      importKeypair(JSON.stringify({
        secret_key_base64: first.secret_key_base64,
        public_key_base64: second.public_key_base64,
      })),
    ).rejects.toThrow("does not match");
  });
});
