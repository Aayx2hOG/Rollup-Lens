ALTER TABLE rollup_batches
ADD COLUMN IF NOT EXISTS operator_status TEXT NOT NULL DEFAULT 'queued'
    CONSTRAINT rollup_batches_operator_status_valid
    CHECK (operator_status IN ('queued', 'proving', 'proved', 'broadcast', 'finalized', 'retry')),
ADD COLUMN IF NOT EXISTS operator_attempts INTEGER NOT NULL DEFAULT 0
    CONSTRAINT rollup_batches_operator_attempts_positive CHECK (operator_attempts >= 0),
ADD COLUMN IF NOT EXISTS operator_last_error TEXT,
ADD COLUMN IF NOT EXISTS operator_proof_started_at TIMESTAMPTZ,
ADD COLUMN IF NOT EXISTS operator_proof_completed_at TIMESTAMPTZ,
ADD COLUMN IF NOT EXISTS operator_broadcast_tx_hash TEXT
    CONSTRAINT rollup_batches_operator_broadcast_tx_hash_hex
    CHECK (operator_broadcast_tx_hash IS NULL OR operator_broadcast_tx_hash ~ '^0x[0-9a-fA-F]{64}$'),
ADD COLUMN IF NOT EXISTS operator_updated_at TIMESTAMPTZ NOT NULL DEFAULT now();

CREATE INDEX IF NOT EXISTS rollup_batches_operator_queue_idx
ON rollup_batches(operator_status, batch_number);

CREATE TABLE IF NOT EXISTS l1_indexer_checkpoints (
    chain_id BIGINT NOT NULL CHECK (chain_id > 0),
    contract_address TEXT NOT NULL
        CHECK (contract_address ~ '^0x[0-9a-fA-F]{40}$'),
    next_block BIGINT NOT NULL CHECK (next_block >= 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (chain_id, contract_address)
);
