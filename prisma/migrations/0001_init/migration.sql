CREATE TABLE IF NOT EXISTS blocks (
    height BIGINT PRIMARY KEY CONSTRAINT blocks_height_nonnegative CHECK (height >= 0),
    parent_hash TEXT NOT NULL CONSTRAINT blocks_parent_hash_hex CHECK (parent_hash ~ '^[0-9a-f]{64}$'),
    state_root TEXT NOT NULL CONSTRAINT blocks_state_root_hex CHECK (state_root ~ '^[0-9a-f]{64}$'),
    block_hash TEXT NOT NULL UNIQUE CONSTRAINT blocks_block_hash_hex CHECK (block_hash ~ '^[0-9a-f]{64}$'),
    transaction_count INTEGER NOT NULL CONSTRAINT blocks_transaction_count_nonnegative CHECK (transaction_count >= 0),
    raw_block JSONB NOT NULL
);

CREATE TABLE IF NOT EXISTS transactions (
    hash TEXT PRIMARY KEY CONSTRAINT transactions_hash_hex CHECK (hash ~ '^[0-9a-f]{64}$'),
    block_height BIGINT CONSTRAINT transactions_block_height_nonnegative CHECK (block_height >= 0) REFERENCES blocks(height) ON DELETE CASCADE ON UPDATE CASCADE,
    transaction_index INTEGER NOT NULL CONSTRAINT transactions_index_nonnegative CHECK (transaction_index >= 0),
    from_address TEXT NOT NULL CONSTRAINT transactions_from_address_present CHECK (length(from_address) > 0),
    to_address TEXT NOT NULL CONSTRAINT transactions_to_address_present CHECK (length(to_address) > 0),
    amount BIGINT NOT NULL CONSTRAINT transactions_amount_positive CHECK (amount > 0),
    nonce BIGINT NOT NULL CONSTRAINT transactions_nonce_nonnegative CHECK (nonce >= 0),
    public_key_base64 TEXT NOT NULL CONSTRAINT transactions_public_key_present CHECK (length(public_key_base64) > 0),
    signature_base64 TEXT NOT NULL CONSTRAINT transactions_signature_present CHECK (length(signature_base64) > 0),
    raw_transfer JSONB NOT NULL
);

CREATE INDEX IF NOT EXISTS transactions_block_height_idx ON transactions(block_height);
CREATE INDEX IF NOT EXISTS transactions_from_address_idx ON transactions(from_address);
CREATE INDEX IF NOT EXISTS transactions_to_address_idx ON transactions(to_address);

CREATE TABLE IF NOT EXISTS accounts (
    address TEXT PRIMARY KEY CONSTRAINT accounts_address_present CHECK (length(address) > 0),
    balance BIGINT NOT NULL CONSTRAINT accounts_balance_nonnegative CHECK (balance >= 0),
    nonce BIGINT NOT NULL CONSTRAINT accounts_nonce_nonnegative CHECK (nonce >= 0)
);

CREATE TABLE IF NOT EXISTS indexer_checkpoints (
    id TEXT PRIMARY KEY,
    last_indexed_height BIGINT CONSTRAINT indexer_checkpoints_height_nonnegative CHECK (last_indexed_height >= 0)
);
