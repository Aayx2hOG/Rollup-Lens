import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { connectWallet, signTransfer } from "./wallet";

beforeEach(() => {
  Object.assign(globalThis, { window: {} });
});

afterEach(() => {
  delete (globalThis as { window?: unknown }).window;
});

describe("Ethereum rollup wallet", () => {
  test("connects to an injected EVM wallet", async () => {
    window.ethereum = {
      request: async ({ method }) => {
        expect(method).toBe("eth_requestAccounts");
        return ["0x1111111111111111111111111111111111111111"];
      },
    };
    expect(await connectWallet()).toBe("0x1111111111111111111111111111111111111111");
  });

  test("requests the canonical EIP-712 transfer", async () => {
    let params: unknown[] | undefined;
    window.ethereum = {
      request: async (request) => {
        params = request.params;
        return `0x${"11".repeat(65)}`;
      },
    };
    const transfer = await signTransfer(
      "0x1111111111111111111111111111111111111111",
      "0x2222222222222222222222222222222222222222",
      12,
      3,
    );
    const typedData = JSON.parse(String(params?.[1]));
    expect(typedData.domain).toEqual({ name: "ZK Rollup", version: "1" });
    expect(typedData.message).toMatchObject({
      amount: "12",
      nonce: "3",
      rollupId: "zk-rollup-local-v1",
    });
    expect(transfer.signature).toHaveLength(132);
  });

  test("rejects malformed recipient addresses before signing", async () => {
    window.ethereum = { request: async () => `0x${"11".repeat(65)}` };
    await expect(
      signTransfer("0x1111111111111111111111111111111111111111", "receiver", 1, 0),
    ).rejects.toThrow("invalid Ethereum address");
  });
});
