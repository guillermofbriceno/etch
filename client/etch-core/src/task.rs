
use tokio::task::JoinHandle;

/// A spawned task that is aborted when this handle is dropped, unlike a bare
/// `JoinHandle`, which detaches.
pub(crate) struct AbortOnDrop(JoinHandle<()>);

impl AbortOnDrop {
    pub fn new(handle: JoinHandle<()>) -> Self {
        Self(handle)
    }

    pub fn abort(&self) {
        self.0.abort();
    }

    /// The caller must have closed whatever feeds the task first; abort on drop is the
    /// backstop after `grace`.
    pub async fn join_within(&mut self, grace: std::time::Duration) {
        if tokio::time::timeout(grace, &mut self.0).await.is_err() {
            log::warn!("A task did not finish within {grace:?} of shutdown; aborting it");
        }
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
    use tokio::time::Instant;

    #[tokio::test]
    async fn dropping_the_handle_aborts_the_task() {
        let handle = tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(300)).await;
        });
        let abort_handle = handle.abort_handle();

        drop(AbortOnDrop::new(handle));
        tokio::task::yield_now().await;

        assert!(abort_handle.is_finished(), "the task should have been aborted");
    }

    #[tokio::test(start_paused = true)]
    async fn a_task_that_finishes_within_the_grace_runs_to_completion() {
        let finished = Arc::new(AtomicBool::new(false));
        let flag = finished.clone();
        let mut task = AbortOnDrop::new(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            flag.store(true, Ordering::SeqCst);
        }));

        task.join_within(Duration::from_secs(5)).await;

        assert!(finished.load(Ordering::SeqCst), "the task should have been allowed to finish");
    }

    #[tokio::test(start_paused = true)]
    async fn waiting_on_a_stuck_task_stops_once_the_grace_elapses() {
        let mut task = AbortOnDrop::new(tokio::spawn(std::future::pending::<()>()));

        let started = Instant::now();
        task.join_within(Duration::from_secs(5)).await;

        assert_eq!(started.elapsed(), Duration::from_secs(5), "shutdown must not wait past the grace");
    }
}
