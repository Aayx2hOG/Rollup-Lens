CREATE TABLE IF NOT EXISTS error_events (
    id BIGSERIAL PRIMARY KEY,
    service TEXT NOT NULL CONSTRAINT error_events_service_present CHECK (length(service) > 0),
    severity TEXT NOT NULL CONSTRAINT error_events_severity_valid CHECK (severity IN ('warning', 'error', 'critical')),
    message TEXT NOT NULL CONSTRAINT error_events_message_present CHECK (length(message) > 0),
    context JSONB NOT NULL DEFAULT '{}'::jsonb,
    resolved_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS error_events_created_at_idx ON error_events(created_at DESC);
CREATE INDEX IF NOT EXISTS error_events_unresolved_idx ON error_events(created_at DESC) WHERE resolved_at IS NULL;
