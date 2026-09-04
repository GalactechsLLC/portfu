use std::future::Future;
use std::time::Duration;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

pub(crate) struct ServerRuntime {
    graceful: CancellationToken,
    background_shutdown: CancellationToken,
    force: CancellationToken,
    http_connections: TaskTracker,
    #[cfg(feature = "websocket")]
    websocket_connections: TaskTracker,
    background_tasks: TaskTracker,
}

impl Default for ServerRuntime {
    fn default() -> Self {
        Self {
            graceful: CancellationToken::new(),
            background_shutdown: CancellationToken::new(),
            force: CancellationToken::new(),
            http_connections: TaskTracker::new(),
            #[cfg(feature = "websocket")]
            websocket_connections: TaskTracker::new(),
            background_tasks: TaskTracker::new(),
        }
    }
}

impl ServerRuntime {
    pub fn cancellation(&self) -> CancellationToken {
        self.graceful.clone()
    }

    #[cfg(feature = "websocket")]
    pub fn is_shutting_down(&self) -> bool {
        self.graceful.is_cancelled()
    }

    pub fn spawn_http<F>(&self, task: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        Self::spawn_tracked(&self.http_connections, self.force.clone(), task);
    }

    #[cfg(feature = "websocket")]
    pub fn spawn_websocket<F>(&self, task: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        Self::spawn_tracked(&self.websocket_connections, self.force.clone(), task);
    }

    pub fn spawn_background<F>(&self, task: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let shutdown = self.background_shutdown.clone();
        let force = self.force.clone();
        self.background_tasks.spawn(async move {
            tokio::select! {
                _ = shutdown.cancelled() => {}
                _ = force.cancelled() => {}
                _ = task => {}
            }
        });
    }

    pub fn begin_shutdown(&self) {
        self.graceful.cancel();
    }

    pub fn force_shutdown(&self) {
        self.begin_shutdown();
        self.close_trackers();
        self.force.cancel();
    }

    pub async fn drain(&self, grace_period: Duration) -> bool {
        self.begin_shutdown();
        self.http_connections.close();
        self.background_tasks.close();
        let drained = timeout(grace_period, async {
            // WebSockets can be created by an HTTP upgrade until the HTTP tracker is empty.
            self.http_connections.wait().await;
            #[cfg(feature = "websocket")]
            {
                self.websocket_connections.close();
                self.websocket_connections.wait().await;
            }
            self.background_shutdown.cancel();
            self.background_tasks.wait().await;
        })
        .await
        .is_ok();
        if !drained {
            self.force_shutdown();
        }
        drained
    }

    fn close_trackers(&self) {
        self.http_connections.close();
        #[cfg(feature = "websocket")]
        self.websocket_connections.close();
        self.background_tasks.close();
    }

    fn spawn_tracked<F>(tracker: &TaskTracker, force: CancellationToken, task: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        tracker.spawn(async move {
            tokio::select! {
                _ = force.cancelled() => {}
                _ = task => {}
            }
        });
    }
}

#[cfg(test)]
#[path = "../../tests/unit/server_runtime.rs"]
mod tests;
