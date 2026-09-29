
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
