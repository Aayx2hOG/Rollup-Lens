const configuredBase = import.meta.env.VITE_EXPLORER_API_BASE as string | undefined;
export const API_BASE = configuredBase?.replace(/\/$/, "") ?? "/api";

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

export type Page<T> = {
  items: T[];
  limit: number;
  offset: number;
  total: number;
  next_offset: number | null;
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

export type RollupBatchSummary = {
  batch_number: number;
  start_block_height: number;
  end_block_height: number;
  prev_state_root: string;
  new_state_root: string;
  transaction_root: string;
  block_hash_root: string;
  data_hash: string;
  transaction_count: number;
  status: string;
  l1_chain_id: number | null;
  l1_contract_address: string | null;
  l1_submit_tx_hash: string | null;
  l1_submitted_at: string | null;
  l1_submit_block_number: number | null;
  l1_submit_gas_used: number | null;
  l1_submit_receipt: unknown | null;
  l1_finalize_tx_hash: string | null;
  l1_finalized_at: string | null;
  l1_finalize_block_number: number | null;
  l1_finalize_gas_used: number | null;
  l1_finalize_receipt: unknown | null;
};

export type RollupBatch = RollupBatchSummary & {
  transactions: TransactionView[];
};

export type TransactionPoolEntry = {
  id: number;
  hash: string;
  status: "pending" | "included" | "failed" | "expired" | string;
  failure_reason: string | null;
  included_block_height: number | null;
  included_transaction_index: number | null;
  created_at: string;
  updated_at: string;
  transfer: Transfer;
};

export type SubmittedTransaction = {
  hash: string;
  status: string;
  transfer: Transfer;
};

export type SequencedBatch = {
  block: Block;
  rollup_batch: RollupBatchSummary | null;
  included_transactions: TransactionPoolEntry[];
  failed_transactions: TransactionPoolEntry[];
};

export type SettlementStatusUpdate = {
  status: "submitted" | "finalized";
  tx_hash: string;
  contract_address: string;
  chain_id: number;
  block_number: number;
  gas_used: number;
  receipt: unknown;
};

export type SearchResult =
  | { kind: "Block"; data: Block }
  | { kind: "Account"; data: Account }
  | { kind: "Transaction"; data: TransactionView };

type ApiError = {
  error?: string;
};

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  let response: Response;
  try {
    response = await fetch(`${API_BASE}${path}`, init);
  } catch (error) {
    throw new Error(apiConnectionMessage(error));
  }
  if (!response.ok) {
    let message = `${response.status} ${response.statusText}`;
    let parsed = false;
    try {
      const payload = (await response.json()) as ApiError;
      parsed = true;
      if (payload.error) message = payload.error;
    } catch {
      // Keep the HTTP status when the body is not JSON.
    }
    if (!parsed && API_BASE === "/api" && response.status >= 500) {
      message = "Explorer API is not reachable. Check the terminal running the UI for the backend error.";
    }
    throw new Error(message);
  }
  return response.json() as Promise<T>;
}

function apiConnectionMessage(error: unknown) {
  const detail = error instanceof Error ? error.message : "request failed";
  return `Explorer API is not reachable through ${API_BASE}. Check the UI development terminal, then refresh. ${detail}`;
}

export const api = {
  health: () => request<Health>("/health"),
  summary: () => request<ChainSummary>("/chain/summary"),
  blocks: (limit = 50, offset = 0) => request<Page<BlockSummary>>(`/blocks?limit=${limit}&offset=${offset}`),
  block: (height: number) => request<Block>(`/blocks/${height}`),
  account: (address: string) => request<Account>(`/accounts/${encodeURIComponent(address)}`),
  transaction: (hash: string) => request<TransactionView>(`/transactions/${encodeURIComponent(hash)}`),
  transactionPool: (limit = 50, offset = 0) => request<Page<TransactionPoolEntry>>(`/rollup/transactions?limit=${limit}&offset=${offset}`),
  submitTransaction: (transfer: Transfer) => request<SubmittedTransaction>("/rollup/transactions", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(transfer),
  }),
  sequenceBatch: (limit = 100) => request<SequencedBatch>("/rollup/batches", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ limit }),
  }),
  errors: () => request<ErrorEvent[]>("/errors"),
  rollupBatches: (limit = 50, offset = 0) => request<Page<RollupBatchSummary>>(`/rollup/batches?limit=${limit}&offset=${offset}`),
  rollupBatch: (batchNumber: number) => request<RollupBatch>(`/rollup/batches/${batchNumber}`),
  updateSettlement: (batchNumber: number, update: SettlementStatusUpdate) => request<RollupBatchSummary>(`/rollup/batches/${batchNumber}/settlement`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(update),
  }),
  search: (query: string) => request<SearchResult>(`/search/${encodeURIComponent(query)}`),
};
