// This file contains structured timebomb annotations for use in integration tests.
// Dates are chosen to be permanently in the past or far future so tests never
// depend on the current wall-clock date.

fn main() {
    // ── Expired annotations (date in the past) ────────────────────────────────

    // TODO[2020-01-01]: remove legacy authentication module
    let _legacy = true;

    // FIXME[2019-06-15]: upstream bug workaround, revert after upgrade
    let _workaround = 42;

    // HACK[2018-03-10]: temporary patch for prod incident #4471
    let _patch = "hack";

    // TEMP[2020-12-31]: remove after Q4 migration completes
    let _temp = vec![1, 2, 3];

    // REMOVEME[2021-05-01]: feature flag for experiment A/B, experiment ended
    let _flag = false;

    // TODO[2020-01-01][alice]: remove after alice's team finishes refactor
    let _owned = "owned";

    // ── Expiring-soon annotations (within a typical warn window) ──────────────
    // These use dates far enough in the future to be "ok" but tests that need
    // "expiring soon" should inject a `today` close to these dates.

    // TODO[2025-06-10]: rotate API keys before deadline
    let _keys = "rotate";

    // FIXME[2025-06-08]: temporary disable of rate limiting, re-enable after deploy
    let _rate_limit = true;

    // ── Future annotations (far future — always OK) ───────────────────────────

    // TODO[2099-01-01]: revisit this algorithm when hardware improves
    let _algo = 0;

    // FIXME[2099-12-31]: long-term tech debt, tracked in issue #9999
    let _debt = "future";

    // HACK[2088-07-04]: can be removed once the platform team ships new API
    let _platform = ();

    // TODO[2099-01-01][bob]: bob will handle this in the next major version
    let _bobs_work = "pending";

    // ── Non-matching annotations (should be ignored by scanner) ──────────────

    // TODO: this is a plain TODO with no date — must NOT be matched
    // FIXME: another plain one
    // NOTE[2020-01-01]: NOTE is not in the default tag list — must NOT be matched
    // TODO [2020-01-01]: space between tag and bracket — must NOT be matched

    println!("fixture file");
}
