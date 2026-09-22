//! Whether a failed Matrix sync is worth retrying, and for how long.
//!
//! This module is the *policy*. It owns one decision -- given the result of a
//! sync request, should the loop carry on, wait and try again, or stop? -- and
//! it reaches that decision without a `Client`, a socket, or a clock. The
//! driver that actually calls `Client::sync_with_result_callback` lives in
//! `super::sync` and does nothing but act on what it is told here.
//!
//! The split is the point. A retry policy written inline in the closure around
//! a network call cannot be tested without a network, so in practice it is not
//! tested at all: the ceiling, the delay sequence and the classification of
//! errors are exactly the parts that go wrong quietly, and exactly the parts
//! that a closure hides. Everything below is a pure function of the failures
//! it has been shown.

use std::time::Duration;

use matrix_sdk::ruma::api::client::error::ErrorKind;

use crate::models::backoff_secs;

/// Did the server reject the access token we presented, as opposed to failing
/// for a reason a retry could fix?
///
/// These three `errcode`s all say the saved session is dead: the token is not
/// recognised, no token was accepted, or the account is gone. Everything else --
/// network trouble, 5xx, rate limiting -- is transient and must not cost us the
/// cached client.
///
/// Shared by the two places that have to tell those apart: the connect path,
/// which throws the saved session away when it sees one (see
/// `MatrixService::forget_rejected_session`), and the sync loop, which stops
/// on one and retries through everything else.
pub(crate) fn credentials_rejected(kind: Option<&ErrorKind>) -> bool {
    matches!(
        kind,
        Some(ErrorKind::UnknownToken { .. } | ErrorKind::MissingToken | ErrorKind::UserDeactivated),
    )
}

/// What a failed sync says about the session it was made on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncFailure {
    /// The request did not get through, or the server could not answer it:
    /// a dropped connection, a timeout, a 5xx, rate limiting. The session is
    /// intact and the very same request is worth making again.
    Transient,
    /// The server rejected our credentials. No amount of retrying fixes that.
    CredentialsRejected,
}

impl SyncFailure {
    /// Classify an SDK error. Read off the `errcode` the server sent rather
    /// than guessed at from the message: an error with no client-API errcode
    /// at all never got an answer from the server, which is transient by
    /// definition.
    pub(crate) fn of(err: &matrix_sdk::Error) -> Self {
        if credentials_rejected(err.client_api_error_kind()) {
            Self::CredentialsRejected
        } else {
            Self::Transient
        }
    }
}

/// What the driver should do about the sync result it just handed over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncStep {
    /// Sync is working. Carry on, quietly.
    Proceed,
    /// Sync is working again after a run of failures. Carry on, and say so:
    /// the UI was told the connection was degraded and needs telling it is not.
    Recovered,
    /// Consecutive transient failure number `attempt`. Wait `delay`, then sync
    /// again on the same session -- nothing is torn down for this.
    Retry { attempt: u32, delay: Duration },
    /// `failures` consecutive transient failures without a single success in
    /// between. Stop retrying in place and let the cold reconnect path take
    /// over as the backstop.
    GiveUp { failures: u32 },
    /// The session is over. Stop at once; waiting cannot help.
    SessionInvalidated,
}

/// How many consecutive sync failures to ride out, and how long to wait
/// between them.
///
/// One instance belongs to one sync loop, and its whole state is the number of
/// failures seen since the last success.
pub(crate) struct SyncRetryPolicy {
    consecutive_failures: u32,
}

impl SyncRetryPolicy {
    /// Consecutive transient failures the loop retries through before giving
    /// up and falling back to a full reconnect.
    ///
    /// Six, because six is where the shared `backoff_secs` first reaches its
    /// 60-second cap: the loop therefore waits once at every distinct delay
    /// the backoff produces -- 2, 4, 8, 16, 32, 60 -- and gives up on the
    /// failure after that. The ceiling is a property of the backoff rather
    /// than a number picked out of the air, and it comes to roughly two
    /// minutes of retrying (122s of waiting across seven failed requests).
    ///
    /// Two minutes is deliberately generous, because retrying here is nearly
    /// free and the alternative is not. A retry in place keeps every timeline,
    /// subscription and sqlite handle exactly where it is; the cold reconnect
    /// it defers clears every timeline, blanks the UI, and in production logs
    /// has taken between 5 and 264 seconds to come back. Spending two minutes
    /// on a session that is probably still fine is the cheaper bet -- and a
    /// session that is genuinely dead does not wait at all, because a rejected
    /// token is `SessionInvalidated`, not a transient failure.
    pub(crate) const MAX_RETRIES: u32 = 6;

    pub(crate) fn new() -> Self {
        Self { consecutive_failures: 0 }
    }

    /// Fold one sync result into the policy and say what to do about it.
    /// `None` is a successful sync.
    pub(crate) fn observe(&mut self, failure: Option<SyncFailure>) -> SyncStep {
        match failure {
            // A success clears the run. Whether the driver has to announce
            // anything depends on there having been a run to clear.
            None => {
                if std::mem::take(&mut self.consecutive_failures) == 0 {
                    SyncStep::Proceed
                } else {
                    SyncStep::Recovered
                }
            }

            // Not a failure of the request, a failure of the session. The
            // count is irrelevant and so is the backoff.
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

    /// Drive `policy` through `n` transient failures and collect what it said.
    fn transient_run(policy: &mut SyncRetryPolicy, n: u32) -> Vec<SyncStep> {
        (0..n).map(|_| policy.observe(Some(SyncFailure::Transient))).collect()
    }

    /// The bug this whole module exists for: one failed long poll must not end
    /// the sync loop. A single transient failure is a wait, not a stop.
    #[test]
    fn one_transient_failure_is_retried_not_fatal() {
        let mut policy = SyncRetryPolicy::new();
        assert_eq!(
            policy.observe(Some(SyncFailure::Transient)),
            SyncStep::Retry { attempt: 1, delay: secs(2) },
            "a single dropped request must be retried in place",
        );
    }

    /// The delay sequence is the shared `backoff_secs`, one step per
    /// consecutive failure, and it must reach the cap rather than escalate
    /// past it.
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

    /// Bounded: a session that never comes back has to fall through to the
    /// cold reconnect rather than retry for ever.
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

    /// And it stays given up: nothing resets the count except a success, so a
    /// driver that ignored the verdict cannot be talked back into retrying.
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

    /// Credentials being rejected is not a network problem, so it costs no
    /// waiting at all -- not even the first two seconds.
    #[test]
    fn rejected_credentials_stop_immediately() {
        let mut policy = SyncRetryPolicy::new();
        assert_eq!(
            policy.observe(Some(SyncFailure::CredentialsRejected)),
            SyncStep::SessionInvalidated,
        );

        // Also from partway through a run of transient failures: a token
        // revoked while the network was flaky must not be waited out.
        let mut policy = SyncRetryPolicy::new();
        transient_run(&mut policy, 3);
        assert_eq!(
            policy.observe(Some(SyncFailure::CredentialsRejected)),
            SyncStep::SessionInvalidated,
        );
    }

    /// A success in the middle of a run clears it, so an intermittent
    /// connection is retried indefinitely rather than accumulating towards the
    /// ceiling over hours of uptime.
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

    /// Recovery is announced once, not on every sync that follows it. The
    /// driver turns `Recovered` into an event, and a healthy client syncs
    /// every 30 seconds for as long as the app is open.
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

    /// Only an errcode that means "this session is dead" may be treated as
    /// fatal. Classing a timeout or a 5xx as a credential failure would send
    /// every hiccup down the teardown path this module exists to avoid.
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
