//! Whether a failed Matrix sync is worth retrying, and for how long.
//!
//! A pure policy with no client or clock; the driver that acts on it is `super::sync`.

use std::time::Duration;

use matrix_sdk::ruma::api::client::error::ErrorKind;

use crate::models::backoff_secs;

/// Whether the server rejected the access token, as opposed to a failure a retry could fix.
pub(crate) fn credentials_rejected(kind: Option<&ErrorKind>) -> bool {
    matches!(
        kind,
        Some(ErrorKind::UnknownToken { .. } | ErrorKind::MissingToken | ErrorKind::UserDeactivated),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncFailure {
    Transient,
    CredentialsRejected,
}

impl SyncFailure {
    /// An error with no client-API errcode never got an answer from the server, so it is transient.
    pub(crate) fn of(err: &matrix_sdk::Error) -> Self {
        if credentials_rejected(err.client_api_error_kind()) {
            Self::CredentialsRejected
        } else {
            Self::Transient
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncStep {
    Proceed,
    Recovered,
    Retry { attempt: u32, delay: Duration },
    GiveUp { failures: u32 },
    SessionInvalidated,
}

pub(crate) struct SyncRetryPolicy {
    consecutive_failures: u32,
}

impl SyncRetryPolicy {
    /// Six is where `backoff_secs` first reaches its 60s cap; about two minutes of
    /// retrying is cheaper than the cold reconnect it defers.
    pub(crate) const MAX_RETRIES: u32 = 6;

    pub(crate) fn new() -> Self {
        Self { consecutive_failures: 0 }
    }

    pub(crate) fn observe(&mut self, failure: Option<SyncFailure>) -> SyncStep {
        match failure {
            None => {
                if std::mem::take(&mut self.consecutive_failures) == 0 {
                    SyncStep::Proceed
                } else {
                    SyncStep::Recovered
                }
            }

            Some(SyncFailure::CredentialsRejected) => SyncStep::SessionInvalidated,

            Some(SyncFailure::Transient) => {
                self.consecutive_failures = self.consecutive_failures.saturating_add(1);
                let failures = self.consecutive_failures;
                if failures > Self::MAX_RETRIES {
                    SyncStep::GiveUp { failures }
                } else {
                    SyncStep::Retry {
                        attempt: failures,
                        delay: Duration::from_secs(backoff_secs(failures)),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn transient_run(policy: &mut SyncRetryPolicy, n: u32) -> Vec<SyncStep> {
        (0..n).map(|_| policy.observe(Some(SyncFailure::Transient))).collect()
    }

    #[test]
    fn one_transient_failure_is_retried_not_fatal() {
        let mut policy = SyncRetryPolicy::new();
        assert_eq!(
            policy.observe(Some(SyncFailure::Transient)),
            SyncStep::Retry { attempt: 1, delay: secs(2) },
            "a single dropped request must be retried in place",
        );
    }

    #[test]
    fn transient_failures_back_off_along_the_shared_curve() {
        let mut policy = SyncRetryPolicy::new();
        let steps = transient_run(&mut policy, SyncRetryPolicy::MAX_RETRIES);

        assert_eq!(
            steps,
            vec![
                SyncStep::Retry { attempt: 1, delay: secs(2) },
                SyncStep::Retry { attempt: 2, delay: secs(4) },
                SyncStep::Retry { attempt: 3, delay: secs(8) },
                SyncStep::Retry { attempt: 4, delay: secs(16) },
                SyncStep::Retry { attempt: 5, delay: secs(32) },
                SyncStep::Retry { attempt: 6, delay: secs(60) },
            ],
            "the retry delays must follow backoff_secs up to its 60s cap",
        );

        let total: Duration = steps.iter().map(|s| match s {
            SyncStep::Retry { delay, .. } => *delay,
            other => panic!("expected a retry, got {other:?}"),
        }).sum();
        assert_eq!(total, secs(122), "the retry window should come to about two minutes");
    }

    #[test]
    fn retrying_gives_up_at_the_ceiling() {
        let mut policy = SyncRetryPolicy::new();
        transient_run(&mut policy, SyncRetryPolicy::MAX_RETRIES);

        assert_eq!(
            policy.observe(Some(SyncFailure::Transient)),
            SyncStep::GiveUp { failures: SyncRetryPolicy::MAX_RETRIES + 1 },
            "the failure after the last retry must end the loop",
        );
    }

    #[test]
    fn giving_up_is_not_reconsidered_on_the_next_failure() {
        let mut policy = SyncRetryPolicy::new();
        transient_run(&mut policy, SyncRetryPolicy::MAX_RETRIES + 1);

        for _ in 0..3 {
            assert!(
                matches!(policy.observe(Some(SyncFailure::Transient)), SyncStep::GiveUp { .. }),
                "once past the ceiling every further failure is still a give-up",
            );
        }
    }

    #[test]
    fn rejected_credentials_stop_immediately() {
        let mut policy = SyncRetryPolicy::new();
        assert_eq!(
            policy.observe(Some(SyncFailure::CredentialsRejected)),
            SyncStep::SessionInvalidated,
        );

        let mut policy = SyncRetryPolicy::new();
        transient_run(&mut policy, 3);
        assert_eq!(
            policy.observe(Some(SyncFailure::CredentialsRejected)),
            SyncStep::SessionInvalidated,
        );
    }

    #[test]
    fn a_success_clears_the_run_and_announces_recovery() {
        let mut policy = SyncRetryPolicy::new();
        transient_run(&mut policy, SyncRetryPolicy::MAX_RETRIES);

        assert_eq!(policy.observe(None), SyncStep::Recovered, "the UI has to be told it is back");
        assert_eq!(
            policy.observe(Some(SyncFailure::Transient)),
            SyncStep::Retry { attempt: 1, delay: secs(2) },
            "the backoff must start over after a success",
        );
    }

    #[test]
    fn an_uninterrupted_run_of_successes_is_silent() {
        let mut policy = SyncRetryPolicy::new();
        assert_eq!(policy.observe(None), SyncStep::Proceed, "a first sync announces nothing");

        policy.observe(Some(SyncFailure::Transient));
        assert_eq!(policy.observe(None), SyncStep::Recovered);
        for _ in 0..5 {
            assert_eq!(policy.observe(None), SyncStep::Proceed, "recovery is announced once");
        }
    }

    #[test]
    fn only_credential_errcodes_are_fatal() {
        assert!(credentials_rejected(Some(&ErrorKind::UnknownToken { soft_logout: false })));
        assert!(credentials_rejected(Some(&ErrorKind::UnknownToken { soft_logout: true })));
        assert!(credentials_rejected(Some(&ErrorKind::MissingToken)));
        assert!(credentials_rejected(Some(&ErrorKind::UserDeactivated)));

        assert!(!credentials_rejected(None), "a transport error is not a rejection");
        assert!(!credentials_rejected(Some(&ErrorKind::NotFound)));
        assert!(!credentials_rejected(Some(&ErrorKind::Unrecognized)));
        assert!(!credentials_rejected(Some(&ErrorKind::forbidden())));
        assert!(
            !credentials_rejected(Some(&ErrorKind::LimitExceeded { retry_after: None })),
            "rate limiting is the server asking us to wait, not to go away",
        );
    }
}
