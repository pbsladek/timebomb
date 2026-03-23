# This file contains structured timebomb annotations for use in integration tests.
# Dates are chosen to be permanently in the past or far future so tests never
# depend on the current wall-clock date.

# FIXME[2019-03-15]: monkey-patch for upstream library bug, remove after upgrade
import os

# ── Expired annotations (date in the past) ────────────────────────────────

# TODO[2020-01-01]: remove compatibility shim for Python 2
legacy_compat = True

_env = os.environ.get("LEGACY", "")


# HACK[2018-11-20]: work around broken CSV parser, replace with stdlib
def parse_csv_hacky(line):
    return line.split(",")


# TEMP[2020-06-30]: temporary feature flag for rollout
ENABLE_NEW_DASHBOARD = False


# REMOVEME[2021-02-28]: dead code left over from old billing system
def old_billing_logic():
    pass


# TODO[2020-01-01][carol]: carol to remove after data migration completes
MIGRATION_DONE = False

# ── Expiring-soon annotations (dates used by tests injecting a close `today`) ──
# Tests that need "expiring soon" status should inject today = 2025-06-01 or similar.

# TODO[2025-06-10]: rotate service account credentials before expiry
SERVICE_ACCOUNT_KEY = "placeholder"

# FIXME[2025-06-08]: disable debug logging before next release
DEBUG_LOGGING = True

# ── Future annotations (far future — always OK) ───────────────────────────


# TODO[2099-01-01]: migrate to async IO once team is trained
def sync_fetch(url):
    return url


# FIXME[2099-12-31]: long-term tech debt, tracked in issue #1234
TECH_DEBT = "acknowledged"


# HACK[2088-04-01]: workaround for platform limitation, revisit in next decade
def platform_workaround():
    return 42


# TODO[2099-01-01][dave]: dave's team owns this cleanup task
OWNED_CLEANUP = None

# ── Non-matching annotations (must be ignored by scanner) ────────────────

# TODO: plain todo with no date bracket — must NOT be matched
# FIXME: another undecorated one — must NOT be matched
# NOTE[2020-01-01]: NOTE is not in the default tag list — must NOT be matched
# TODO [2020-01-01]: space between tag and bracket — must NOT be matched
