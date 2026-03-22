-- This file contains structured timebomb annotations for use in integration tests.
-- Dates are chosen to be permanently in the past or far future so tests never
-- depend on the current wall-clock date.

-- ── Expired annotations (date in the past) ────────────────────────────────

-- TODO[2020-01-01]: drop temp_users table after migration completes
CREATE TABLE temp_users (
    id SERIAL PRIMARY KEY,
    username VARCHAR(255) NOT NULL,
    legacy_field TEXT
);

-- FIXME[2019-08-15]: remove this view once reporting pipeline is updated
CREATE VIEW legacy_report_view AS
SELECT id, username FROM temp_users;

-- HACK[2018-06-01]: denormalized column added for perf, remove after index added
ALTER TABLE temp_users ADD COLUMN cache_value TEXT;

-- TEMP[2020-03-31]: temporary index for slow query workaround, drop after upgrade
CREATE INDEX idx_temp_cache ON temp_users(cache_value);

-- REMOVEME[2021-01-15]: old audit log table, superseded by audit_v2
CREATE TABLE audit_log_old (
    id SERIAL PRIMARY KEY,
    action TEXT,
    created_at TIMESTAMP DEFAULT NOW()
);

-- TODO[2020-01-01][eve]: eve to drop after confirming backfill job succeeded
ALTER TABLE temp_users ADD COLUMN backfill_done BOOLEAN DEFAULT FALSE;

-- ── Expiring-soon annotations (dates used by tests injecting a close `today`) ──
-- Tests that need "expiring soon" status should inject today = 2025-06-01 or similar.

-- TODO[2025-06-10]: remove this column after data migration window closes
ALTER TABLE temp_users ADD COLUMN migration_flag INTEGER DEFAULT 0;

-- FIXME[2025-06-08]: revert this constraint relaxation after hotfix is verified
ALTER TABLE temp_users ALTER COLUMN legacy_field DROP NOT NULL;

-- ── Future annotations (far future — always OK) ───────────────────────────

-- TODO[2099-01-01]: revisit sharding strategy when user count exceeds 1B
CREATE INDEX idx_users_future ON temp_users(id);

-- FIXME[2099-12-31]: long-term schema tech debt, tracked in issue #8888
COMMENT ON TABLE temp_users IS 'Legacy table, see issue #8888';

-- HACK[2088-09-20]: workaround for ORM limitation, remove when ORM is replaced
CREATE OR REPLACE VIEW compat_users_view AS
SELECT id, username, NULL AS deprecated_col FROM temp_users;

-- TODO[2099-01-01][frank]: frank owns the schema cleanup for the next major version
ALTER TABLE temp_users ADD COLUMN future_field TEXT;

-- ── Non-matching annotations (must be ignored by scanner) ────────────────

-- TODO: plain todo with no date bracket — must NOT be matched
-- FIXME: another undecorated one — must NOT be matched
-- NOTE[2020-01-01]: NOTE is not in the default tag list — must NOT be matched
-- TODO [2020-01-01]: space between tag and bracket — must NOT be matched
