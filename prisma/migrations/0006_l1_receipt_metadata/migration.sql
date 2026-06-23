ALTER TABLE rollup_batches
ADD COLUMN IF NOT EXISTS l1_submit_block_number BIGINT CONSTRAINT rollup_batches_l1_submit_block_number_positive CHECK (l1_submit_block_number IS NULL OR l1_submit_block_number >= 0),
ADD COLUMN IF NOT EXISTS l1_submit_gas_used BIGINT CONSTRAINT rollup_batches_l1_submit_gas_used_positive CHECK (l1_submit_gas_used IS NULL OR l1_submit_gas_used >= 0),
ADD COLUMN IF NOT EXISTS l1_submit_receipt JSONB,
ADD COLUMN IF NOT EXISTS l1_finalize_block_number BIGINT CONSTRAINT rollup_batches_l1_finalize_block_number_positive CHECK (l1_finalize_block_number IS NULL OR l1_finalize_block_number >= 0),
ADD COLUMN IF NOT EXISTS l1_finalize_gas_used BIGINT CONSTRAINT rollup_batches_l1_finalize_gas_used_positive CHECK (l1_finalize_gas_used IS NULL OR l1_finalize_gas_used >= 0),
ADD COLUMN IF NOT EXISTS l1_finalize_receipt JSONB;
