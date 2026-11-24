//! Unit helpers for the per-account timestamps shared by the credential stores.
//!
//! Both `cockpit-core` and `cockpit-tools` keep their own copy of
//! `secure_account_storage` (the copies have drifted), so the rule that decides
//! what a stored timestamp means lives here and is `#[path]`-included by both
//! crates. Keeping it in one file is the point: the millisecond/second mix-up
//! that pinned switch times at year 58863 was fixed once already and had to be
//! fixed again in the second copy.

/// Values at or above this bound are legacy millisecond stamps; second-based
/// stamps stay below it until the year 5138. A real timestamp never sits in
/// between.
const MILLIS_TIMESTAMP_FLOOR: i64 = 100_000_000_000;

/// Builds before `3ddcffd6` recorded switch times in milliseconds. Those values
/// stay on disk until the account is rewritten, and both the UI (which
/// multiplies by 1000) and the monotonic write guard compare them numerically,
/// so a raw millisecond stamp renders as year 58863 and outranks every later
/// second-based write. Fold it back to seconds before either happens.
pub(crate) fn epoch_seconds(value: i64) -> i64 {
    if value >= MILLIS_TIMESTAMP_FLOOR {
        value / 1000
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::epoch_seconds;

    #[test]
    fn seconds_pass_through_untouched() {
        assert_eq!(epoch_seconds(1_795_397_283), 1_795_397_283);
        assert_eq!(epoch_seconds(0), 0);
    }

    #[test]
    fn millisecond_stamps_are_folded_to_seconds() {
        assert_eq!(epoch_seconds(1_795_397_283_233), 1_795_397_283);
    }

    #[test]
    fn the_boundary_itself_is_treated_as_milliseconds() {
        assert_eq!(epoch_seconds(100_000_000_000), 100_000_000);
    }
}
