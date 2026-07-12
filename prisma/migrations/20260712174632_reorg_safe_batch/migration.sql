-- AlterTable
ALTER TABLE "l1_batch_finalization_events" ALTER COLUMN "observed_at" SET DATA TYPE TIMESTAMP(3),
ALTER COLUMN "updated_at" SET DATA TYPE TIMESTAMP(3);

-- RenameIndex
ALTER INDEX "l1_batch_finalization_events_block_idx" RENAME TO "l1_batch_finalization_events_chain_id_contract_address_l1_b_idx";

-- RenameIndex
ALTER INDEX "l1_batch_finalization_events_status_idx" RENAME TO "l1_batch_finalization_events_status_batch_number_idx";
