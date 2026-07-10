CREATE TABLE IF NOT EXISTS jobs (
    id BIGSERIAL PRIMARY KEY,
    job_type TEXT NOT NULL CONSTRAINT jobs_job_type_present CHECK (length(job_type) > 0),
    subject_id TEXT NOT NULL CONSTRAINT jobs_subject_id_present CHECK (length(subject_id) > 0),
    status TEXT NOT NULL DEFAULT 'queued'
        CONSTRAINT jobs_status_valid CHECK (status IN ('queued', 'running', 'succeeded', 'retry', 'failed', 'cancelled')),
    priority INTEGER NOT NULL DEFAULT 0,
    attempts INTEGER NOT NULL DEFAULT 0 CONSTRAINT jobs_attempts_nonnegative CHECK (attempts >= 0),
    max_attempts INTEGER NOT NULL DEFAULT 3 CONSTRAINT jobs_max_attempts_positive CHECK (max_attempts > 0),
    locked_by TEXT,
    locked_at TIMESTAMPTZ,
    run_after TIMESTAMPTZ NOT NULL DEFAULT now(),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (job_type, subject_id),
    CONSTRAINT jobs_lock_state_valid CHECK (
        (status = 'running' AND locked_by IS NOT NULL AND locked_at IS NOT NULL)
        OR (status <> 'running')
    )
);

CREATE INDEX IF NOT EXISTS jobs_ready_idx
ON jobs(status, run_after, priority DESC, id)
WHERE status IN ('queued', 'retry');

CREATE INDEX IF NOT EXISTS jobs_locked_at_idx
ON jobs(locked_at)
WHERE locked_at IS NOT NULL;

CREATE TABLE IF NOT EXISTS system_alerts (
    id BIGSERIAL PRIMARY KEY,
    service TEXT NOT NULL CONSTRAINT system_alerts_service_present CHECK (length(service) > 0),
    severity TEXT NOT NULL CONSTRAINT system_alerts_severity_valid CHECK (severity IN ('info', 'warning', 'error', 'critical')),
    alert_type TEXT NOT NULL CONSTRAINT system_alerts_alert_type_present CHECK (length(alert_type) > 0),
    subject_id TEXT,
    message TEXT NOT NULL CONSTRAINT system_alerts_message_present CHECK (length(message) > 0),
    context JSONB NOT NULL DEFAULT '{}'::jsonb,
    resolved_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS system_alerts_created_at_idx
ON system_alerts(created_at DESC);

CREATE INDEX IF NOT EXISTS system_alerts_open_severity_idx
ON system_alerts(severity, created_at DESC)
WHERE resolved_at IS NULL;

CREATE INDEX IF NOT EXISTS system_alerts_type_subject_idx
ON system_alerts(alert_type, subject_id);
