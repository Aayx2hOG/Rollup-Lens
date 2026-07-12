ALTER TABLE jobs
ALTER COLUMN locked_at SET DATA TYPE TIMESTAMP(3),
ALTER COLUMN run_after SET DATA TYPE TIMESTAMP(3),
ALTER COLUMN created_at SET DATA TYPE TIMESTAMP(3),
ALTER COLUMN updated_at SET DATA TYPE TIMESTAMP(3);

CREATE INDEX IF NOT EXISTS jobs_status_run_after_priority_id_idx
ON jobs(status, run_after, priority, id);

ALTER TABLE rollup_batches
ALTER COLUMN withdrawal_root SET DEFAULT '0000000000000000000000000000000000000000000000000000000000000000';

ALTER TABLE system_alerts
ALTER COLUMN resolved_at SET DATA TYPE TIMESTAMP(3),
ALTER COLUMN created_at SET DATA TYPE TIMESTAMP(3);

DROP INDEX IF EXISTS system_alerts_created_at_idx;

CREATE INDEX IF NOT EXISTS system_alerts_created_at_idx
ON system_alerts(created_at);

CREATE INDEX IF NOT EXISTS system_alerts_severity_resolved_at_idx
ON system_alerts(severity, resolved_at);

ALTER INDEX IF EXISTS system_alerts_type_subject_idx
RENAME TO system_alerts_alert_type_subject_id_idx;

ALTER TABLE withdrawal_pool
ALTER COLUMN created_at SET DATA TYPE TIMESTAMP(3),
ALTER COLUMN updated_at SET DATA TYPE TIMESTAMP(3);

ALTER TABLE withdrawals
ALTER COLUMN claimed_at SET DATA TYPE TIMESTAMP(3);
