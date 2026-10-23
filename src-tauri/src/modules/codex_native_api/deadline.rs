//! A missing total deadline really disables the timer, without overflowing an Instant.
use std::{cmp::Ordering, future::Future, time::Duration};
use tokio::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Deadline(Option<Instant>);

impl Deadline {
    pub(super) const NONE: Self = Self(None);

    pub(super) fn after(timeout: Duration) -> Self {
        if timeout.is_zero() {
            Self::NONE
        } else {
            Self(Some(Instant::now() + timeout))
        }
    }

    pub(super) fn min(self, other: impl Into<Self>) -> Self {
        let other = other.into();
        match (self.0, other.0) {
            (Some(a), Some(b)) => Self(Some(a.min(b))),
            (Some(_), None) => self,
            (None, _) => other,
        }
    }

    /// Apply an always-finite phase limit even when the total is unlimited.
    pub(super) fn cap(self, phase: Instant) -> Instant {
        self.0.map_or(phase, |total| total.min(phase))
    }

    pub(super) fn instant(self) -> Option<Instant> {
        self.0
    }
}

impl From<Instant> for Deadline {
    fn from(value: Instant) -> Self {
        Self(Some(value))
    }
}

impl From<Option<Instant>> for Deadline {
    fn from(value: Option<Instant>) -> Self {
        Self(value)
    }
}

impl PartialOrd<Deadline> for Instant {
    fn partial_cmp(&self, other: &Deadline) -> Option<Ordering> {
        Some(
            other
                .0
                .map_or(Ordering::Less, |deadline| self.cmp(&deadline)),
        )
    }
}

impl PartialEq<Deadline> for Instant {
    fn eq(&self, other: &Deadline) -> bool {
        other.0 == Some(*self)
    }
}

pub(super) async fn timeout_at<F: Future>(
    deadline: impl Into<Deadline>,
    future: F,
) -> Result<F::Output, tokio::time::error::Elapsed> {
    match deadline.into().0 {
        Some(deadline) => tokio::time::timeout_at(deadline, future).await,
        None => Ok(future.await),
    }
}

pub(super) async fn sleep_until(deadline: impl Into<Deadline>) {
    match deadline.into().0 {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn go_limits_disabled_total_has_no_timer_but_finite_phases_still_expire() {
        let unlimited = Deadline::after(Duration::ZERO);
        assert_eq!(unlimited.instant(), None);
        assert!(Instant::now() + Duration::from_secs(3600) < unlimited);
        let phase = Instant::now() + Duration::from_millis(20);
        assert_eq!(unlimited.cap(phase), phase);
        assert_eq!(unlimited.min(phase).instant(), Some(phase));
        assert_eq!(Deadline::from(phase).min(unlimited).instant(), Some(phase));
        assert_eq!(timeout_at(unlimited, async { 42 }).await.unwrap(), 42);
        assert!(timeout_at(phase, std::future::pending::<()>())
            .await
            .is_err());
        assert!(
            tokio::time::timeout(Duration::from_millis(20), sleep_until(unlimited))
                .await
                .is_err()
        );
    }
}
