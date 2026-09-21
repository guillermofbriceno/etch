//! Tying a spawned task's lifetime to the thing that owns it.

use tokio::task::JoinHandle;

/// A spawned task that is aborted when this handle is dropped.
///
/// A bare `JoinHandle` detaches on drop rather than aborting, so a task whose
/// handle goes out of scope keeps running with everything it captured. That
/// makes it easy to leave work behind on an early return, or to replace a
/// subscription and end up with both the old and new one live. Owning the task
/// through this instead makes every drop a teardown.
pub(crate) struct AbortOnDrop(JoinHandle<()>);

impl AbortOnDrop {
    pub fn new(handle: JoinHandle<()>) -> Self {
        Self(handle)
    }

    /// Stop the task without dropping the handle. Only needed where the owner
    /// outlives the task, such as a session that is torn down but reused.
    pub fn abort(&self) {
        self.0.abort();
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
}
