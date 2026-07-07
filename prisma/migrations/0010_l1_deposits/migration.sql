CREATE TABLE IF NOT EXISTS l1_deposits (
    id BIGSERIAL PRIMARY KEY,
    chain_id BIGINT NOT NULL CHECK (chain_id > 0),
    inbox_address TEXT NOT NULL CHECK (inbox_address ~ '^0x[0-9a-f]{40}$'),
    deposit_id BIGINT NOT NULL CHECK (deposit_id >= 0),
    l1_tx_hash TEXT NOT NULL CHECK (l1_tx_hash ~ '^0x[0-9a-f]{64}$'),
    l1_block_number BIGINT NOT NULL CHECK (l1_block_number >= 0),
    l1_block_hash TEXT NOT NULL CHECK (l1_block_hash ~ '^0x[0-9a-f]{64}$'),
    l1_log_index INTEGER NOT NULL CHECK (l1_log_index >= 0),
    depositor_l1_address TEXT NOT NULL CHECK (depositor_l1_address ~ '^0x[0-9a-f]{40}$'),
    recipient_l2_address TEXT NOT NULL CHECK (recipient_l2_address ~ '^0x[0-9a-f]{40}$'),
    amount BIGINT NOT NULL CHECK (amount > 0),
    rolling_commitment TEXT NOT NULL CHECK (rolling_commitment ~ '^0x[0-9a-f]{64}$'),
    status TEXT NOT NULL DEFAULT 'observed'
        CHECK (status IN ('observed', 'confirmed', 'executed', 'finalized', 'orphaned')),
    confirmations INTEGER NOT NULL DEFAULT 0 CHECK (confirmations >= 0),
    observed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (chain_id, inbox_address, deposit_id, l1_block_hash, l1_log_index)
);

CREATE UNIQUE INDEX IF NOT EXISTS l1_deposits_canonical_deposit_idx
ON l1_deposits(chain_id, inbox_address, deposit_id)
WHERE status <> 'orphaned';

CREATE INDEX IF NOT EXISTS l1_deposits_recipient_idx
ON l1_deposits(recipient_l2_address, deposit_id);

CREATE INDEX IF NOT EXISTS l1_deposits_status_idx
ON l1_deposits(status, deposit_id);

CREATE INDEX IF NOT EXISTS l1_deposits_block_idx
ON l1_deposits(chain_id, inbox_address, l1_block_number, l1_log_index);
