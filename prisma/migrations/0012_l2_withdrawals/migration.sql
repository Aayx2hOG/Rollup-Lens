ALTER TABLE rollup_batches
ADD COLUMN IF NOT EXISTS withdrawal_count INTEGER NOT NULL DEFAULT 0
    CONSTRAINT rollup_batches_withdrawal_count_nonnegative CHECK (withdrawal_count >= 0),
ADD COLUMN IF NOT EXISTS withdrawal_root TEXT NOT NULL DEFAULT repeat('0', 64)
    CONSTRAINT rollup_batches_withdrawal_root_hex CHECK (withdrawal_root ~ '^[0-9a-f]{64}$');

CREATE TABLE IF NOT EXISTS withdrawal_pool (
    id BIGSERIAL PRIMARY KEY,
    hash TEXT NOT NULL UNIQUE,
    from_address TEXT NOT NULL,
    l1_recipient TEXT NOT NULL,
    amount BIGINT NOT NULL CHECK (amount > 0),
    nonce BIGINT NOT NULL CHECK (nonce >= 0),
    signature TEXT NOT NULL,
    status TEXT NOT NULL,
    failure_reason TEXT,
    included_block_height BIGINT,
    included_withdrawal_index INTEGER,
    raw_withdrawal JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT withdrawal_pool_inclusion_valid CHECK (
        (status <> 'included' AND included_block_height IS NULL AND included_withdrawal_index IS NULL)
        OR (status = 'included' AND included_block_height IS NOT NULL AND included_withdrawal_index IS NOT NULL AND included_withdrawal_index >= 0)
    )
);

CREATE INDEX IF NOT EXISTS withdrawal_pool_status_id_idx
ON withdrawal_pool(status, id);

CREATE INDEX IF NOT EXISTS withdrawal_pool_from_address_nonce_idx
ON withdrawal_pool(from_address, nonce);

CREATE INDEX IF NOT EXISTS withdrawal_pool_created_at_idx
ON withdrawal_pool(created_at);

CREATE TABLE IF NOT EXISTS withdrawals (
    hash TEXT PRIMARY KEY,
    block_height BIGINT NOT NULL REFERENCES blocks(height) ON DELETE CASCADE ON UPDATE CASCADE,
    withdrawal_index INTEGER NOT NULL CHECK (withdrawal_index >= 0),
    from_address TEXT NOT NULL,
    l1_recipient TEXT NOT NULL,
    amount BIGINT NOT NULL CHECK (amount > 0),
    nonce BIGINT NOT NULL CHECK (nonce >= 0),
    signature TEXT NOT NULL,
    raw_withdrawal JSONB NOT NULL,
    l1_claim_tx_hash TEXT,
    claimed_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS withdrawals_block_height_idx
ON withdrawals(block_height);

CREATE INDEX IF NOT EXISTS withdrawals_from_address_idx
ON withdrawals(from_address);

CREATE INDEX IF NOT EXISTS withdrawals_l1_recipient_idx
ON withdrawals(l1_recipient);

CREATE TABLE IF NOT EXISTS rollup_batch_withdrawals (
    batch_number BIGINT NOT NULL REFERENCES rollup_batches(batch_number) ON DELETE CASCADE ON UPDATE CASCADE,
    withdrawal_hash TEXT NOT NULL REFERENCES withdrawals(hash) ON DELETE CASCADE ON UPDATE CASCADE,
    block_height BIGINT NOT NULL,
    withdrawal_index INTEGER NOT NULL CHECK (withdrawal_index >= 0),
    PRIMARY KEY (batch_number, withdrawal_hash)
);

CREATE INDEX IF NOT EXISTS rollup_batch_withdrawals_withdrawal_hash_idx
ON rollup_batch_withdrawals(withdrawal_hash);

CREATE INDEX IF NOT EXISTS rollup_batch_withdrawals_block_height_idx
ON rollup_batch_withdrawals(block_height);
