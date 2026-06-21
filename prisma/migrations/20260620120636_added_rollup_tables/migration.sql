-- DropIndex
DROP INDEX "error_events_created_at_idx";

-- AlterTable
ALTER TABLE "error_events" ALTER COLUMN "resolved_at" SET DATA TYPE TIMESTAMP(3),
ALTER COLUMN "created_at" SET DATA TYPE TIMESTAMP(3);

-- CreateIndex
CREATE INDEX "error_events_created_at_idx" ON "error_events"("created_at");

-- RenameIndex
ALTER INDEX "rollup_batch_transactions_hash_idx" RENAME TO "rollup_batch_transactions_transaction_hash_idx";

-- RenameIndex
ALTER INDEX "rollup_batches_end_height_idx" RENAME TO "rollup_batches_end_block_height_idx";

-- RenameIndex
ALTER INDEX "rollup_batches_start_height_idx" RENAME TO "rollup_batches_start_block_height_idx";
