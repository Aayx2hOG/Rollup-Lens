-- DropIndex
DROP INDEX "rollup_batches_l1_finalize_tx_hash_idx";

-- DropIndex
DROP INDEX "rollup_batches_l1_submit_tx_hash_idx";

-- AlterTable
ALTER TABLE "rollup_batches" ALTER COLUMN "l1_submitted_at" SET DATA TYPE TIMESTAMP(3),
ALTER COLUMN "l1_finalized_at" SET DATA TYPE TIMESTAMP(3);
