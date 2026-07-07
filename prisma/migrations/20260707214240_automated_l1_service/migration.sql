-- DropIndex
DROP INDEX "rollup_batches_operator_queue_idx";

-- AlterTable
ALTER TABLE "l1_deposits" ALTER COLUMN "observed_at" SET DATA TYPE TIMESTAMP(3),
ALTER COLUMN "updated_at" SET DATA TYPE TIMESTAMP(3);

-- AlterTable
ALTER TABLE "l1_indexer_checkpoints" ALTER COLUMN "updated_at" SET DATA TYPE TIMESTAMP(3);

-- AlterTable
ALTER TABLE "rollup_batches" ALTER COLUMN "operator_proof_started_at" SET DATA TYPE TIMESTAMP(3),
ALTER COLUMN "operator_proof_completed_at" SET DATA TYPE TIMESTAMP(3),
ALTER COLUMN "operator_updated_at" SET DATA TYPE TIMESTAMP(3);

-- RenameIndex
ALTER INDEX "l1_deposits_block_idx" RENAME TO "l1_deposits_chain_id_inbox_address_l1_block_number_l1_log_i_idx";

-- RenameIndex
ALTER INDEX "l1_deposits_recipient_idx" RENAME TO "l1_deposits_recipient_l2_address_deposit_id_idx";

-- RenameIndex
ALTER INDEX "l1_deposits_status_idx" RENAME TO "l1_deposits_status_deposit_id_idx";
