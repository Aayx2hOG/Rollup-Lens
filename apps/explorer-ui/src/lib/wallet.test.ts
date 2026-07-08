import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { RollupBatchSummary } from "./api";
import {
  connectWallet,
  depositToL2,
  parseProofArtifact,
  signTransfer,
  submitBatchWithProof,
} from "./wallet";

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

  test("sends an L1 deposit transaction to the configured inbox", async () => {
    let params: unknown[] | undefined;
    window.ethereum = {
      request: async (request) => {
        expect(request.method).toBe("eth_sendTransaction");
        params = request.params;
        return `0x${"22".repeat(32)}`;
      },
    };

    const txHash = await depositToL2(
      "0x1111111111111111111111111111111111111111",
      "0x2222222222222222222222222222222222222222",
      "0.25",
      "0x3333333333333333333333333333333333333333",
    );

    expect(txHash).toBe(`0x${"22".repeat(32)}`);
    expect(params).toEqual([{
      from: "0x1111111111111111111111111111111111111111",
      to: "0x3333333333333333333333333333333333333333",
      value: "0x3782dace9d90000",
      data: `0xf340fa010000000000000000000000002222222222222222222222222222222222222222`,
    }]);
  });

  test("submits a validity proof for the selected batch", async () => {
    let params: unknown[] | undefined;
    window.ethereum = {
      request: async (request) => {
        expect(request.method).toBe("eth_sendTransaction");
        params = request.params;
        return `0x${"44".repeat(32)}`;
      },
    };

    const txHash = await submitBatchWithProof(
      "0x1111111111111111111111111111111111111111",
      batchFixture(),
      {
        public_values: publicValuesFixture(),
        proof: "0x1234",
      },
      "0x3333333333333333333333333333333333333333",
    );

    expect(txHash).toBe(`0x${"44".repeat(32)}`);
    expect(params).toEqual([{
      from: "0x1111111111111111111111111111111111111111",
      to: "0x3333333333333333333333333333333333333333",
      data: expect.stringMatching(/^0x59cc2189/),
    }]);
  });

  test("rejects a proof artifact for a different batch", async () => {
    window.ethereum = { request: async () => `0x${"44".repeat(32)}` };
    await expect(
      submitBatchWithProof(
        "0x1111111111111111111111111111111111111111",
        { ...batchFixture(), batch_number: 9 },
        { public_values: publicValuesFixture(), proof: "0x1234" },
        "0x3333333333333333333333333333333333333333",
      ),
    ).rejects.toThrow("selected settlement package");
  });

  test("parses prover artifact JSON", () => {
    expect(parseProofArtifact('{"public_values":"0x1234","proof":"0xabcd"}')).toEqual({
      public_values: "0x1234",
      proof: "0xabcd",
    });
  });
});

function batchFixture(): RollupBatchSummary {
  return {
    batch_number: 7,
    start_block_height: 1,
    end_block_height: 1,
    prev_state_root: `0x${"11".repeat(32)}`,
    new_state_root: `0x${"22".repeat(32)}`,
    transaction_root: `0x${"33".repeat(32)}`,
    block_hash_root: `0x${"44".repeat(32)}`,
    deposit_root: `0x${"55".repeat(32)}`,
    data_hash: `0x${"66".repeat(32)}`,
    transaction_count: 1,
    deposit_count: 1,
    first_deposit_id: 0,
    last_deposit_id: 0,
    status: "built",
    l1_chain_id: null,
    l1_contract_address: null,
    l1_submit_tx_hash: null,
    l1_submitted_at: null,
    l1_submit_block_number: null,
    l1_submit_gas_used: null,
    l1_submit_receipt: null,
    l1_finalize_tx_hash: null,
    l1_finalized_at: null,
    l1_finalize_block_number: null,
    l1_finalize_gas_used: null,
    l1_finalize_receipt: null,
  };
}

function publicValuesFixture() {
  const batch = batchFixture();
  return [
    "0x",
    batch.batch_number.toString(16).padStart(64, "0"),
    batch.prev_state_root.slice(2),
    batch.new_state_root.slice(2),
    batch.transaction_root.slice(2),
    batch.block_hash_root.slice(2),
    batch.data_hash.slice(2),
    batch.deposit_root.slice(2),
    "0".repeat(64),
    "0".repeat(64),
    batch.deposit_count.toString(16).padStart(64, "0"),
    batch.transaction_count.toString(16).padStart(64, "0"),
    `${"0".repeat(24)}${"22".repeat(20)}`,
  ].join("");
}
