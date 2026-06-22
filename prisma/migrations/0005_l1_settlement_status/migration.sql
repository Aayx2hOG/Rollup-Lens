ALTER TABLE rollup_batches
ADD COLUMN IF NOT EXISTS l1_chain_id BIGINT CONSTRAINT rollup_batches_l1_chain_id_positive CHECK (l1_chain_id IS NULL OR l1_chain_id > 0),
ADD COLUMN IF NOT EXISTS l1_contract_address TEXT CONSTRAINT rollup_batches_l1_contract_address_hex CHECK (l1_contract_address IS NULL OR l1_contract_address ~ '^0x[0-9a-fA-F]{40}$'),
ADD COLUMN IF NOT EXISTS l1_submit_tx_hash TEXT CONSTRAINT rollup_batches_l1_submit_tx_hash_hex CHECK (l1_submit_tx_hash IS NULL OR l1_submit_tx_hash ~ '^0x[0-9a-fA-F]{64}$'),
ADD COLUMN IF NOT EXISTS l1_submitted_at TIMESTAMPTZ,
ADD COLUMN IF NOT EXISTS l1_finalize_tx_hash TEXT CONSTRAINT rollup_batches_l1_finalize_tx_hash_hex CHECK (l1_finalize_tx_hash IS NULL OR l1_finalize_tx_hash ~ '^0x[0-9a-fA-F]{64}$'),
ADD COLUMN IF NOT EXISTS l1_finalized_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS rollup_batches_l1_submit_tx_hash_idx ON rollup_batches(l1_submit_tx_hash);
CREATE INDEX IF NOT EXISTS rollup_batches_l1_finalize_tx_hash_idx ON rollup_batches(l1_finalize_tx_hash);
