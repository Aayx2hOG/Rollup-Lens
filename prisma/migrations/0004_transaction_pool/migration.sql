CREATE TABLE IF NOT EXISTS transaction_pool (
    id BIGSERIAL PRIMARY KEY,
    hash TEXT NOT NULL UNIQUE CONSTRAINT transaction_pool_hash_hex CHECK (hash ~ '^[0-9a-f]{64}$'),
    from_address TEXT NOT NULL CONSTRAINT transaction_pool_from_address_present CHECK (length(from_address) > 0),
    to_address TEXT NOT NULL CONSTRAINT transaction_pool_to_address_present CHECK (length(to_address) > 0),
    amount BIGINT NOT NULL CONSTRAINT transaction_pool_amount_positive CHECK (amount > 0),
    nonce BIGINT NOT NULL CONSTRAINT transaction_pool_nonce_nonnegative CHECK (nonce >= 0),
    public_key_base64 TEXT NOT NULL CONSTRAINT transaction_pool_public_key_present CHECK (length(public_key_base64) > 0),
    signature_base64 TEXT NOT NULL CONSTRAINT transaction_pool_signature_present CHECK (length(signature_base64) > 0),
    status TEXT NOT NULL CONSTRAINT transaction_pool_status_valid CHECK (status IN ('pending', 'included', 'failed', 'expired')),
    failure_reason TEXT,
    included_block_height BIGINT CONSTRAINT transaction_pool_included_block_height_positive CHECK (included_block_height > 0),
    included_transaction_index INTEGER CONSTRAINT transaction_pool_included_transaction_index_nonnegative CHECK (included_transaction_index >= 0),
    raw_transfer JSONB NOT NULL,
    created_at TIMESTAMP(3) NOT NULL DEFAULT now(),
    updated_at TIMESTAMP(3) NOT NULL DEFAULT now(),
    CONSTRAINT transaction_pool_included_status_consistent CHECK (
        (status = 'included' AND included_block_height IS NOT NULL AND included_transaction_index IS NOT NULL)
        OR (status <> 'included' AND included_block_height IS NULL AND included_transaction_index IS NULL)
    )
);

CREATE INDEX IF NOT EXISTS transaction_pool_status_id_idx ON transaction_pool(status, id);
CREATE INDEX IF NOT EXISTS transaction_pool_from_address_nonce_idx ON transaction_pool(from_address, nonce);
CREATE INDEX IF NOT EXISTS transaction_pool_created_at_idx ON transaction_pool(created_at);
