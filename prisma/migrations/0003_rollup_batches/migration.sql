CREATE TABLE IF NOT EXISTS rollup_batches (
    batch_number BIGINT PRIMARY KEY CONSTRAINT rollup_batches_batch_number_nonnegative CHECK (batch_number >= 0),
    start_block_height BIGINT NOT NULL CONSTRAINT rollup_batches_start_height_positive CHECK (start_block_height > 0),
    end_block_height BIGINT NOT NULL CONSTRAINT rollup_batches_end_height_positive CHECK (end_block_height > 0),
    prev_state_root TEXT NOT NULL CONSTRAINT rollup_batches_prev_state_root_hex CHECK (prev_state_root ~ '^[0-9a-f]{64}$'),
    new_state_root TEXT NOT NULL CONSTRAINT rollup_batches_new_state_root_hex CHECK (new_state_root ~ '^[0-9a-f]{64}$'),
    transaction_root TEXT NOT NULL CONSTRAINT rollup_batches_transaction_root_hex CHECK (transaction_root ~ '^[0-9a-f]{64}$'),
    block_hash_root TEXT NOT NULL CONSTRAINT rollup_batches_block_hash_root_hex CHECK (block_hash_root ~ '^[0-9a-f]{64}$'),
    data_hash TEXT NOT NULL UNIQUE CONSTRAINT rollup_batches_data_hash_hex CHECK (data_hash ~ '^[0-9a-f]{64}$'),
    transaction_count INTEGER NOT NULL CONSTRAINT rollup_batches_transaction_count_nonnegative CHECK (transaction_count >= 0),
    status TEXT NOT NULL CONSTRAINT rollup_batches_status_valid CHECK (status IN ('built', 'submitted', 'finalized', 'challenged', 'rejected')),
    raw_commitment JSONB NOT NULL,
    CONSTRAINT rollup_batches_height_range_valid CHECK (end_block_height >= start_block_height)
);

CREATE INDEX IF NOT EXISTS rollup_batches_start_height_idx ON rollup_batches(start_block_height);
CREATE INDEX IF NOT EXISTS rollup_batches_end_height_idx ON rollup_batches(end_block_height);

CREATE TABLE IF NOT EXISTS rollup_batch_transactions (
    batch_number BIGINT NOT NULL REFERENCES rollup_batches(batch_number) ON DELETE CASCADE ON UPDATE CASCADE,
    transaction_hash TEXT NOT NULL REFERENCES transactions(hash) ON DELETE CASCADE ON UPDATE CASCADE,
    block_height BIGINT NOT NULL CONSTRAINT rollup_batch_transactions_block_height_positive CHECK (block_height > 0),
    transaction_index INTEGER NOT NULL CONSTRAINT rollup_batch_transactions_index_nonnegative CHECK (transaction_index >= 0),
    PRIMARY KEY (batch_number, transaction_hash)
);

CREATE INDEX IF NOT EXISTS rollup_batch_transactions_hash_idx ON rollup_batch_transactions(transaction_hash);
CREATE INDEX IF NOT EXISTS rollup_batch_transactions_block_height_idx ON rollup_batch_transactions(block_height);
