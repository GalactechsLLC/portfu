use crate::server::runtime::ServerRuntime;
use std::time::Duration;
use tokio::sync::oneshot;

#[tokio::test]
async fn graceful_shutdown_cancels_and_drains_tracked_connections() {
    let runtime = ServerRuntime::default();
    let cancellation = runtime.cancellation();
    let (stopped_tx, stopped_rx) = oneshot::channel();
    runtime.spawn_http(async move {
        cancellation.cancelled().await;
        let _ = stopped_tx.send(());
    });

    assert!(runtime.drain(Duration::from_secs(1)).await);
    stopped_rx
        .await
        .expect("tracked task did not observe shutdown");
}

#[tokio::test]
async fn force_shutdown_drops_uncooperative_tasks() {
    let runtime = ServerRuntime::default();
    let (started_tx, started_rx) = oneshot::channel();
    let (dropped_tx, dropped_rx) = oneshot::channel();
    runtime.spawn_background(async move {
        let _drop_signal = DropSignal(Some(dropped_tx));
        let _ = started_tx.send(());
        std::future::pending::<()>().await;
    });
    started_rx.await.expect("tracked task did not start");

    runtime.force_shutdown();

    tokio::time::timeout(Duration::from_secs(1), dropped_rx)
        .await
        .expect("tracked task was not forcefully dropped")
        .expect("drop signal sender disappeared");
}

struct DropSignal(Option<oneshot::Sender<()>>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}
