DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM accounts LIMIT 1)
        OR EXISTS (SELECT 1 FROM transactions LIMIT 1)
        OR EXISTS (SELECT 1 FROM transaction_pool LIMIT 1)
    THEN
        RAISE EXCEPTION
            'EIP-712 account migration requires a fresh database because Ed25519 addresses cannot be converted to Ethereum addresses';
    END IF;
END;
$$;

ALTER TABLE transactions
    RENAME COLUMN signature_base64 TO signature;

ALTER TABLE transactions
    DROP COLUMN public_key_base64;

ALTER TABLE transaction_pool
    RENAME COLUMN signature_base64 TO signature;

ALTER TABLE transaction_pool
    DROP COLUMN public_key_base64;

ALTER TABLE accounts
    ADD CONSTRAINT accounts_ethereum_address
    CHECK (address ~ '^0x[0-9a-f]{40}$');

ALTER TABLE transactions
    ADD CONSTRAINT transactions_from_ethereum_address
        CHECK (from_address ~ '^0x[0-9a-f]{40}$'),
    ADD CONSTRAINT transactions_to_ethereum_address
        CHECK (to_address ~ '^0x[0-9a-f]{40}$'),
    ADD CONSTRAINT transactions_ethereum_signature
        CHECK (signature ~ '^0x[0-9a-f]{130}$');

ALTER TABLE transaction_pool
    ADD CONSTRAINT transaction_pool_from_ethereum_address
        CHECK (from_address ~ '^0x[0-9a-f]{40}$'),
    ADD CONSTRAINT transaction_pool_to_ethereum_address
        CHECK (to_address ~ '^0x[0-9a-f]{40}$'),
    ADD CONSTRAINT transaction_pool_ethereum_signature
        CHECK (signature ~ '^0x[0-9a-f]{130}$');
