ALTER TABLE rollup_batches
ADD COLUMN IF NOT EXISTS deposit_count INTEGER NOT NULL DEFAULT 0
    CONSTRAINT rollup_batches_deposit_count_nonnegative CHECK (deposit_count >= 0),
ADD COLUMN IF NOT EXISTS deposit_root TEXT NOT NULL DEFAULT repeat('0', 64)
    CONSTRAINT rollup_batches_deposit_root_hex CHECK (deposit_root ~ '^[0-9a-f]{64}$'),
ADD COLUMN IF NOT EXISTS first_deposit_id BIGINT
    CONSTRAINT rollup_batches_first_deposit_id_nonnegative CHECK (first_deposit_id IS NULL OR first_deposit_id >= 0),
ADD COLUMN IF NOT EXISTS last_deposit_id BIGINT
    CONSTRAINT rollup_batches_last_deposit_id_nonnegative CHECK (last_deposit_id IS NULL OR last_deposit_id >= 0),
ADD CONSTRAINT rollup_batches_deposit_range_valid CHECK (
    (deposit_count = 0 AND first_deposit_id IS NULL AND last_deposit_id IS NULL)
    OR (deposit_count > 0 AND first_deposit_id IS NOT NULL AND last_deposit_id IS NOT NULL AND last_deposit_id >= first_deposit_id)
);

CREATE TABLE IF NOT EXISTS rollup_batch_deposits (
    batch_number BIGINT NOT NULL REFERENCES rollup_batches(batch_number) ON DELETE CASCADE ON UPDATE CASCADE,
    deposit_row_id BIGINT NOT NULL REFERENCES l1_deposits(id) ON DELETE RESTRICT ON UPDATE CASCADE,
    deposit_id BIGINT NOT NULL CHECK (deposit_id >= 0),
    deposit_index INTEGER NOT NULL CHECK (deposit_index >= 0),
    PRIMARY KEY (batch_number, deposit_row_id),
    UNIQUE (deposit_row_id),
    UNIQUE (batch_number, deposit_index)
);

CREATE INDEX IF NOT EXISTS rollup_batch_deposits_deposit_id_idx
ON rollup_batch_deposits(deposit_id);
