const configuredBase = import.meta.env.VITE_EXPLORER_API_BASE as string | undefined;
const API_BASE = configuredBase?.replace(/\/$/, "") ?? "/api";

export type ChainSummary = {
  height: number | null;
  block_count: number;
  transaction_count: number;
  pending_transaction_count: number;
  account_count: number;
  state_root: string;
  latest_block_hash: string | null;
  genesis_finalized: boolean;
  valid: boolean;
};

export type Health = {
  status: "ok" | "degraded" | string;
  latest_error: ErrorEvent | null;
};

export type BlockSummary = {
  height: number;
  parent_hash: string;
  state_root: string;
  block_hash: string;
  transaction_count: number;
};

export type Transfer = {
  from: string;
  to: string;
  amount: number;
  nonce: number;
  public_key_base64: string;
  signature_base64: string;
};

export type Block = {
  height: number;
  parent_hash: string;
  transactions: Transfer[];
  state_root: string;
  block_hash: string;
  transaction_views: TransactionView[];
};

export type Account = {
  address: string;
  balance: number;
  nonce: number;
};

export type TransactionView = {
  hash: string;
  block_height: number | null;
  transaction_index: number;
  transfer: Transfer;
};

export type ErrorEvent = {
  id: number;
  service: string;
  severity: string;
  message: string;
  context: unknown;
  created_at: string;
  resolved_at: string | null;
};

export type SearchResult =
  | { kind: "Block"; data: Block }
  | { kind: "Account"; data: Account }
  | { kind: "Transaction"; data: TransactionView };

type ApiError = {
  error?: string;
};

async function request<T>(path: string): Promise<T> {
  const response = await fetch(`${API_BASE}${path}`);
  if (!response.ok) {
    let message = `${response.status} ${response.statusText}`;
    try {
      const payload = (await response.json()) as ApiError;
      if (payload.error) message = payload.error;
    } catch {
      // Keep the HTTP status when the body is not JSON.
    }
    throw new Error(message);
  }
  return response.json() as Promise<T>;
}

export const api = {
  health: () => request<Health>("/health"),
  summary: () => request<ChainSummary>("/chain/summary"),
  blocks: () => request<BlockSummary[]>("/blocks"),
  block: (height: number) => request<Block>(`/blocks/${height}`),
  account: (address: string) => request<Account>(`/accounts/${encodeURIComponent(address)}`),
  transaction: (hash: string) => request<TransactionView>(`/transactions/${encodeURIComponent(hash)}`),
  errors: () => request<ErrorEvent[]>("/errors"),
  search: (query: string) => request<SearchResult>(`/search/${encodeURIComponent(query)}`),
};
