import type { Transfer } from "./api";

const DOMAIN = { name: "ZK Rollup", version: "1" } as const;
const ROLLUP_ID = "zk-rollup-local-v1";
const DEPOSIT_SELECTOR = "0xf340fa01";
const viteEnv = (import.meta as ImportMeta & { env?: Record<string, string | undefined> }).env;
const TRANSFER_FIELDS = [
  { name: "from", type: "address" },
  { name: "to", type: "address" },
  { name: "amount", type: "uint256" },
  { name: "nonce", type: "uint256" },
  { name: "rollupId", type: "string" },
] as const;

type EthereumProvider = {
  request(args: { method: string; params?: unknown[] }): Promise<unknown>;
};

declare global {
  interface Window {
    ethereum?: EthereumProvider;
  }
}

function provider() {
  if (!window.ethereum) throw new Error("No Ethereum wallet found");
  return window.ethereum;
}

function normalizeAddress(value: unknown) {
  if (typeof value !== "string" || !/^0x[0-9a-fA-F]{40}$/.test(value)) {
    throw new Error("Wallet returned an invalid Ethereum address");
  }
  return value.toLowerCase();
}

export async function connectWallet() {
  const accounts = await provider().request({ method: "eth_requestAccounts" });
  if (!Array.isArray(accounts) || accounts.length === 0) {
    throw new Error("Wallet did not provide an account");
  }
  return normalizeAddress(accounts[0]);
}

export async function signTransfer(
  from: string,
  to: string,
  amount: number,
  nonce: number,
): Promise<Transfer> {
  const normalizedFrom = normalizeAddress(from);
  const normalizedTo = normalizeAddress(to);
  if (!Number.isSafeInteger(amount) || amount <= 0) throw new Error("Amount must be a positive integer");
  if (!Number.isSafeInteger(nonce) || nonce < 0) throw new Error("Nonce must be a non-negative integer");

  const typedData = {
    types: { EIP712Domain: [
      { name: "name", type: "string" },
      { name: "version", type: "string" },
    ], Transfer: TRANSFER_FIELDS },
    primaryType: "Transfer",
    domain: DOMAIN,
    message: {
      from: normalizedFrom,
      to: normalizedTo,
      amount: String(amount),
      nonce: String(nonce),
      rollupId: ROLLUP_ID,
    },
  };
  const signature = await provider().request({
    method: "eth_signTypedData_v4",
    params: [normalizedFrom, JSON.stringify(typedData)],
  });
  if (typeof signature !== "string" || !/^0x[0-9a-fA-F]{130}$/.test(signature)) {
    throw new Error("Wallet returned an invalid Ethereum signature");
  }
  return { from: normalizedFrom, to: normalizedTo, amount, nonce, signature };
}

export const l1DepositInboxAddress = normalizeOptionalAddress(
  viteEnv?.VITE_L1_DEPOSIT_INBOX,
);

export async function depositToL2(
  from: string,
  recipient: string,
  amountEth: string,
  inboxAddress = l1DepositInboxAddress,
) {
  const inbox = normalizeOptionalAddress(inboxAddress);
  if (!inbox) throw new Error("Deposit contract is not configured");
  const normalizedFrom = normalizeAddress(from);
  const normalizedRecipient = normalizeAddress(recipient);
  const value = parseEthAmount(amountEth);
  if (value <= 0n) throw new Error("Amount must be greater than 0 ETH");

  const txHash = await provider().request({
    method: "eth_sendTransaction",
    params: [{
      from: normalizedFrom,
      to: inbox,
      value: `0x${value.toString(16)}`,
      data: encodeDepositCall(normalizedRecipient),
    }],
  });
  if (typeof txHash !== "string" || !/^0x[0-9a-fA-F]{64}$/.test(txHash)) {
    throw new Error("Wallet returned an invalid transaction hash");
  }
  return txHash.toLowerCase();
}

export function hasWalletProvider() {
  return typeof window !== "undefined" && Boolean(window.ethereum);
}

function normalizeOptionalAddress(value: unknown) {
  if (typeof value !== "string" || value.trim() === "") return null;
  return normalizeAddress(value.trim());
}

function encodeDepositCall(recipient: string) {
  return `${DEPOSIT_SELECTOR}${"0".repeat(24)}${recipient.slice(2)}`;
}

function parseEthAmount(value: string) {
  const trimmed = value.trim();
  if (!/^(?:\d+|\d+\.\d+)$/.test(trimmed)) {
    throw new Error("Enter a valid ETH amount");
  }
  const [whole, fraction = ""] = trimmed.split(".");
  if (fraction.length > 18) {
    throw new Error("ETH amount supports up to 18 decimal places");
  }
  return BigInt(whole) * 10n ** 18n + BigInt(fraction.padEnd(18, "0"));
}
