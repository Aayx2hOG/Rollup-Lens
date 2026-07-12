ALTER TABLE rollup_batches
ADD COLUMN IF NOT EXISTS l1_finalize_block_hash TEXT
    CONSTRAINT rollup_batches_l1_finalize_block_hash_hex
    CHECK (l1_finalize_block_hash IS NULL OR l1_finalize_block_hash ~ '^0x[0-9a-f]{64}$'),
ADD COLUMN IF NOT EXISTS l1_finalize_log_index INTEGER
    CONSTRAINT rollup_batches_l1_finalize_log_index_nonnegative
    CHECK (l1_finalize_log_index IS NULL OR l1_finalize_log_index >= 0),
ADD COLUMN IF NOT EXISTS l1_finality_status TEXT
    CONSTRAINT rollup_batches_l1_finality_status_valid
    CHECK (l1_finality_status IS NULL OR l1_finality_status IN ('observed', 'safe', 'finalized', 'orphaned'));

CREATE TABLE IF NOT EXISTS l1_batch_finalization_events (
    id BIGSERIAL PRIMARY KEY,
    chain_id BIGINT NOT NULL CHECK (chain_id > 0),
    contract_address TEXT NOT NULL CHECK (contract_address ~ '^0x[0-9a-f]{40}$'),
    batch_number BIGINT NOT NULL CHECK (batch_number >= 0),
    l1_tx_hash TEXT NOT NULL CHECK (l1_tx_hash ~ '^0x[0-9a-f]{64}$'),
    l1_block_number BIGINT NOT NULL CHECK (l1_block_number >= 0),
    l1_block_hash TEXT NOT NULL CHECK (l1_block_hash ~ '^0x[0-9a-f]{64}$'),
    l1_log_index INTEGER NOT NULL CHECK (l1_log_index >= 0),
    prev_state_root TEXT NOT NULL CHECK (prev_state_root ~ '^0x[0-9a-f]{64}$'),
    new_state_root TEXT NOT NULL CHECK (new_state_root ~ '^0x[0-9a-f]{64}$'),
    transaction_root TEXT NOT NULL CHECK (transaction_root ~ '^0x[0-9a-f]{64}$'),
    block_hash_root TEXT NOT NULL CHECK (block_hash_root ~ '^0x[0-9a-f]{64}$'),
    data_hash TEXT NOT NULL CHECK (data_hash ~ '^0x[0-9a-f]{64}$'),
    transaction_count INTEGER NOT NULL CHECK (transaction_count >= 0),
    batch_commitment TEXT NOT NULL CHECK (batch_commitment ~ '^0x[0-9a-f]{64}$'),
    status TEXT NOT NULL DEFAULT 'observed'
        CHECK (status IN ('observed', 'safe', 'finalized', 'orphaned')),
    l1_gas_used BIGINT CHECK (l1_gas_used IS NULL OR l1_gas_used >= 0),
    l1_receipt JSONB,
    observed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (chain_id, contract_address, batch_number, l1_block_hash, l1_tx_hash, l1_log_index)
);

CREATE UNIQUE INDEX IF NOT EXISTS l1_batch_finalization_events_canonical_idx
ON l1_batch_finalization_events(chain_id, contract_address, batch_number)
WHERE status <> 'orphaned';

CREATE INDEX IF NOT EXISTS l1_batch_finalization_events_block_idx
ON l1_batch_finalization_events(chain_id, contract_address, l1_block_number);

CREATE INDEX IF NOT EXISTS l1_batch_finalization_events_status_idx
ON l1_batch_finalization_events(status, batch_number);
