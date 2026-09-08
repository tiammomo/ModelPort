use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use tokio::{
    sync::{Notify, Semaphore},
    task::JoinHandle,
};
use tracing::{info, warn};

use crate::{
    AppError, config::RuntimeAdapterConfig, enterprise_ledger::EnterpriseLedger, metrics::Metrics,
    runtime_adapter::RuntimeAdapterClient,
};

// One fleet-wide cap keeps the maximum 64 configured adapters from exhausting the process.
const MAX_CONCURRENT_COLLECTIONS: usize = 4;
const MAX_BACKOFF: Duration = Duration::from_secs(30);

pub(crate) struct RuntimeAdapterCollector {
    draining: Arc<AtomicBool>,
    wake: Arc<Notify>,
    tasks: Vec<JoinHandle<()>>,
}

impl RuntimeAdapterCollector {
    pub(crate) fn start(
        adapters: BTreeMap<String, RuntimeAdapterConfig>,
        ledger: Arc<EnterpriseLedger>,
        metrics: Arc<Metrics>,
        draining: Arc<AtomicBool>,
    ) -> Result<Self, AppError> {
        let clients = adapters
            .into_iter()
            .map(|(adapter_id, config)| {
                RuntimeAdapterClient::new(config.client_config)
                    .map(|client| (adapter_id, client, config.poll_interval))
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        let permits = Arc::new(Semaphore::new(MAX_CONCURRENT_COLLECTIONS));
        let wake = Arc::new(Notify::new());
        let mut tasks = Vec::with_capacity(clients.len());

        for (adapter_id, client, poll_interval) in clients {
            tasks.push(tokio::spawn(collection_loop(
                adapter_id,
                client,
                poll_interval,
                Arc::clone(&ledger),
                Arc::clone(&metrics),
                Arc::clone(&permits),
                Arc::clone(&draining),
                Arc::clone(&wake),
            )));
        }

        Ok(Self {
            draining,
            wake,
            tasks,
        })
    }

    pub(crate) async fn shutdown(mut self, timeout: Duration) -> bool {
        self.draining.store(true, Ordering::Release);
        self.wake.notify_waiters();
        let deadline = tokio::time::Instant::now() + timeout;
        while let Some(mut task) = self.tasks.pop() {
            match tokio::time::timeout_at(deadline, &mut task).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    warn!(error = %error, "Runtime Adapter collector task failed to join");
                }
                Err(_) => {
                    task.abort();
                    for pending in &self.tasks {
                        pending.abort();
                    }
                    return false;
                }
            }
        }
        true
    }
}

#[allow(clippy::too_many_arguments)]
async fn collection_loop(
    adapter_id: String,
    client: RuntimeAdapterClient,
    poll_interval: Duration,
    ledger: Arc<EnterpriseLedger>,
    metrics: Arc<Metrics>,
    permits: Arc<Semaphore>,
    draining: Arc<AtomicBool>,
    wake: Arc<Notify>,
) {
    let mut consecutive_failures = 0_u32;
    loop {
        if draining.load(Ordering::Acquire) {
            break;
        }

        let permit = tokio::select! {
            permit = Arc::clone(&permits).acquire_owned() => match permit {
                Ok(permit) => permit,
                Err(_) => break,
            },
            _ = wake.notified() => continue,
        };
        if draining.load(Ordering::Acquire) {
            drop(permit);
            break;
        }

        let result = collect_once(&client, &ledger).await;
        drop(permit);
        let delay = match result {
            Ok(()) => {
                consecutive_failures = 0;
                metrics.record_runtime_adapter_collection(&adapter_id, Ok(()));
                poll_interval
            }
            Err(error) => {
                consecutive_failures = consecutive_failures.saturating_add(1);
                let error_class = log_collection_failure(&adapter_id, &error);
                metrics.record_runtime_adapter_collection(&adapter_id, Err(error_class));
                retry_delay(&adapter_id, consecutive_failures, poll_interval)
            }
        };

        // A notification can arrive while collection is in flight, so re-check before sleeping.
        if draining.load(Ordering::Acquire) {
            break;
        }

        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = wake.notified() => {}
        }
    }
    info!(adapter_id, "Runtime Adapter Compute collection stopped");
}

async fn collect_once(
    client: &RuntimeAdapterClient,
    ledger: &EnterpriseLedger,
) -> Result<(), AppError> {
    let observation = client.collect_compute_inventory().await?;
    ledger
        .persist_runtime_compute_inventory(&observation.inventory)
        .await?;
    Ok(())
}

fn log_collection_failure(adapter_id: &str, error: &AppError) -> &'static str {
    let error_class = error.telemetry_code();
    warn!(
        adapter_id,
        error_class, "Runtime Adapter Compute collection failed"
    );
    error_class
}

fn retry_delay(adapter_id: &str, consecutive_failures: u32, poll_interval: Duration) -> Duration {
    let exponent = consecutive_failures.saturating_sub(1).min(5);
    let base = Duration::from_secs(1_u64 << exponent);
    let cap = poll_interval.min(MAX_BACKOFF);
    let base = base.min(cap);
    let jitter_window_ms = (base.as_millis() / 4).max(1);
    // Stable per-adapter jitter keeps tests deterministic while de-synchronizing the fleet.
    let hash = adapter_id.bytes().fold(0_u64, |value, byte| {
        value.wrapping_mul(31).wrapping_add(u64::from(byte))
    });
    let jitter_ms = u64::try_from(u128::from(hash) % jitter_window_ms).unwrap_or(0);
    base.saturating_add(Duration::from_millis(jitter_ms))
        .min(cap)
}

#[cfg(test)]
mod tests {
    use std::{
        io::{self, Write},
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering as AtomicOrdering},
        },
    };

    use axum::{
        Router,
        extract::State,
        http::{StatusCode, header::CONTENT_TYPE},
        response::{IntoResponse, Response},
        routing::get,
    };
    use tracing_subscriber::fmt::MakeWriter;

    use super::*;
    use crate::{config::RuntimeAdapterConfig, runtime_adapter::RuntimeAdapterClientConfig};

    const CAPABILITIES: &str = include_str!(
        "../../tests/fixtures/runtime-adapters/qwen-llama-cpp-capabilities-v1alpha1.json"
    );
    const INVENTORY: &str = include_str!(
        "../../tests/fixtures/runtime-adapters/qwen-llama-cpp-compute-inventory-v1alpha1.json"
    );

    #[derive(Clone)]
    struct FakeAdapter {
        calls: Arc<Mutex<Vec<&'static str>>>,
        capabilities: String,
        inventory: String,
        fail: bool,
        delay: Duration,
        concurrency: Arc<ConcurrencyProbe>,
    }

    #[derive(Default)]
    struct ConcurrencyProbe {
        active: AtomicUsize,
        maximum: AtomicUsize,
    }

    #[derive(Clone)]
    struct CapturedWriter(Arc<Mutex<Vec<u8>>>);

    struct CapturedGuard(Arc<Mutex<Vec<u8>>>);

    impl Write for CapturedGuard {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for CapturedWriter {
        type Writer = CapturedGuard;

        fn make_writer(&'a self) -> Self::Writer {
            CapturedGuard(Arc::clone(&self.0))
        }
    }

    #[tokio::test]
    async fn starts_immediately_persists_and_stops() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let base_url = spawn_adapter(
            "qwen-llama-cpp-reference",
            Arc::clone(&calls),
            Arc::default(),
            Duration::ZERO,
            false,
        )
        .await;
        let config = adapter_config(
            "qwen-llama-cpp-reference",
            base_url,
            Duration::from_secs(60),
        );
        let ledger = Arc::new(EnterpriseLedger::memory());
        let metrics = Arc::new(Metrics::new());
        let draining = Arc::new(AtomicBool::new(false));
        let collector = RuntimeAdapterCollector::start(
            BTreeMap::from([("qwen-llama-cpp-reference".to_owned(), config)]),
            Arc::clone(&ledger),
            Arc::clone(&metrics),
            draining,
        )
        .unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if ledger
                    .latest_runtime_compute_inventory(
                        "qwen-llama-cpp-reference",
                        Duration::from_secs(90),
                    )
                    .await
                    .unwrap()
                    .inventory
                    .is_some()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        assert_eq!(*calls.lock().unwrap(), vec!["capabilities", "inventory"]);
        assert!(collector.shutdown(Duration::from_secs(1)).await);
        assert!(metrics.render_prometheus().contains(
            r#"modelport_runtime_adapter_collection_successes_total{adapter_id="qwen-llama-cpp-reference"} 1"#
        ));
    }

    #[test]
    fn retry_backoff_is_bounded_and_deterministic() {
        let first = retry_delay("edge-1", 1, Duration::from_secs(10));
        assert_eq!(first, retry_delay("edge-1", 1, Duration::from_secs(10)));
        assert!(first >= Duration::from_secs(1));
        assert!(retry_delay("edge-1", 20, Duration::from_secs(10)) <= Duration::from_secs(10));
    }

    #[tokio::test]
    async fn shutdown_wakes_long_poll_intervals_without_waiting_for_the_timer() {
        let collector = RuntimeAdapterCollector {
            draining: Arc::new(AtomicBool::new(false)),
            wake: Arc::new(Notify::new()),
            tasks: Vec::new(),
        };
        let started = std::time::Instant::now();
        assert!(collector.shutdown(Duration::from_millis(50)).await);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn shutdown_waits_for_an_in_flight_collection_and_starts_no_new_attempt() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let base_url = spawn_adapter(
            "drain-adapter",
            Arc::clone(&calls),
            Arc::default(),
            Duration::from_millis(100),
            false,
        )
        .await;
        let ledger = Arc::new(EnterpriseLedger::memory());
        let collector = RuntimeAdapterCollector::start(
            BTreeMap::from([(
                "drain-adapter".to_owned(),
                adapter_config("drain-adapter", base_url, Duration::from_millis(1)),
            )]),
            Arc::clone(&ledger),
            Arc::new(Metrics::new()),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();

        wait_until(Duration::from_secs(1), || !calls.lock().unwrap().is_empty()).await;
        let started = std::time::Instant::now();
        assert!(collector.shutdown(Duration::from_secs(1)).await);
        assert!(started.elapsed() >= Duration::from_millis(50));
        assert_eq!(*calls.lock().unwrap(), vec!["capabilities", "inventory"]);
        assert!(
            ledger
                .latest_runtime_compute_inventory("drain-adapter", Duration::from_secs(90))
                .await
                .unwrap()
                .inventory
                .is_some()
        );
    }

    #[tokio::test]
    async fn shutdown_aborts_an_in_flight_collection_after_the_deadline() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let base_url = spawn_adapter(
            "timeout-adapter",
            Arc::clone(&calls),
            Arc::default(),
            Duration::from_secs(1),
            false,
        )
        .await;
        let collector = RuntimeAdapterCollector::start(
            BTreeMap::from([(
                "timeout-adapter".to_owned(),
                adapter_config("timeout-adapter", base_url, Duration::from_secs(60)),
            )]),
            Arc::new(EnterpriseLedger::memory()),
            Arc::new(Metrics::new()),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();

        wait_until(Duration::from_secs(1), || !calls.lock().unwrap().is_empty()).await;
        assert!(!collector.shutdown(Duration::from_millis(20)).await);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(*calls.lock().unwrap(), vec!["capabilities"]);
    }

    #[test]
    fn failure_logging_exposes_only_adapter_id_and_bounded_error_class() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(CapturedWriter(Arc::clone(&captured)))
            .finish();
        let error = AppError::Upstream {
            status: 502,
            body: "secret upstream body Bearer test-token https://runtime.invalid".to_owned(),
            retry_after_secs: None,
        };

        tracing::subscriber::with_default(subscriber, || {
            assert_eq!(
                log_collection_failure("safe-adapter", &error),
                "upstream_http"
            );
        });
        let output = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
        assert!(output.contains("safe-adapter"));
        assert!(output.contains("upstream_http"));
        for sensitive in [
            "secret upstream body",
            "Bearer",
            "test-token",
            "runtime.invalid",
        ] {
            assert!(!output.contains(sensitive));
        }
    }

    #[tokio::test]
    async fn repeated_collection_is_idempotent_and_never_overlaps_one_adapter() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let concurrency = Arc::new(ConcurrencyProbe::default());
        let base_url = spawn_adapter(
            "repeat-adapter",
            Arc::clone(&calls),
            Arc::clone(&concurrency),
            Duration::from_millis(10),
            false,
        )
        .await;
        let ledger = Arc::new(EnterpriseLedger::memory());
        let metrics = Arc::new(Metrics::new());
        let collector = RuntimeAdapterCollector::start(
            BTreeMap::from([(
                "repeat-adapter".to_owned(),
                adapter_config("repeat-adapter", base_url, Duration::from_millis(1)),
            )]),
            Arc::clone(&ledger),
            Arc::clone(&metrics),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();

        wait_until(Duration::from_secs(2), || calls.lock().unwrap().len() >= 4).await;
        assert_eq!(concurrency.maximum.load(AtomicOrdering::Acquire), 1);
        assert!(collector.shutdown(Duration::from_secs(1)).await);
        assert!(metrics.render_prometheus().contains(
            r#"modelport_runtime_adapter_collection_successes_total{adapter_id="repeat-adapter"} 2"#
        ));
    }

    #[tokio::test]
    async fn collection_is_globally_bounded_to_four_adapters() {
        let concurrency = Arc::new(ConcurrencyProbe::default());
        let mut adapters = BTreeMap::new();
        for index in 0..5 {
            let adapter_id = format!("edge-{index}");
            let base_url = spawn_adapter(
                &adapter_id,
                Arc::default(),
                Arc::clone(&concurrency),
                Duration::from_millis(50),
                false,
            )
            .await;
            adapters.insert(
                adapter_id.clone(),
                adapter_config(&adapter_id, base_url, Duration::from_secs(60)),
            );
        }
        let collector = RuntimeAdapterCollector::start(
            adapters,
            Arc::new(EnterpriseLedger::memory()),
            Arc::new(Metrics::new()),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();

        wait_until(Duration::from_secs(2), || {
            concurrency.maximum.load(AtomicOrdering::Acquire) >= MAX_CONCURRENT_COLLECTIONS
        })
        .await;
        assert_eq!(
            concurrency.maximum.load(AtomicOrdering::Acquire),
            MAX_CONCURRENT_COLLECTIONS
        );
        assert!(collector.shutdown(Duration::from_secs(3)).await);
    }

    #[tokio::test]
    async fn a_failing_adapter_does_not_block_a_healthy_adapter() {
        let healthy_url = spawn_adapter(
            "healthy-adapter",
            Arc::default(),
            Arc::default(),
            Duration::ZERO,
            false,
        )
        .await;
        let failing_url = spawn_adapter(
            "failing-adapter",
            Arc::default(),
            Arc::default(),
            Duration::ZERO,
            true,
        )
        .await;
        let ledger = Arc::new(EnterpriseLedger::memory());
        let metrics = Arc::new(Metrics::new());
        let collector = RuntimeAdapterCollector::start(
            BTreeMap::from([
                (
                    "failing-adapter".to_owned(),
                    adapter_config("failing-adapter", failing_url, Duration::from_secs(60)),
                ),
                (
                    "healthy-adapter".to_owned(),
                    adapter_config("healthy-adapter", healthy_url, Duration::from_secs(60)),
                ),
            ]),
            Arc::clone(&ledger),
            Arc::clone(&metrics),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let healthy = ledger
                    .latest_runtime_compute_inventory(
                        "healthy-adapter",
                        Duration::from_secs(90),
                    )
                    .await
                    .unwrap()
                    .inventory
                    .is_some();
                let failure = metrics.render_prometheus().contains(
                    r#"modelport_runtime_adapter_collection_failures_total{adapter_id="failing-adapter",error="upstream_http"} 1"#,
                );
                if healthy && failure {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(collector.shutdown(Duration::from_secs(1)).await);
    }

    fn adapter_config(
        adapter_id: &str,
        base_url: String,
        poll_interval: Duration,
    ) -> RuntimeAdapterConfig {
        RuntimeAdapterConfig {
            client_config: RuntimeAdapterClientConfig::new(adapter_id, base_url, "test-token")
                .unwrap(),
            credential_env: "TEST_TOKEN".to_owned(),
            poll_interval,
            stale_after: Duration::from_secs(90),
        }
    }

    async fn spawn_adapter(
        adapter_id: &str,
        calls: Arc<Mutex<Vec<&'static str>>>,
        concurrency: Arc<ConcurrencyProbe>,
        delay: Duration,
        fail: bool,
    ) -> String {
        let capabilities = CAPABILITIES.replace("qwen-llama-cpp-reference", adapter_id);
        let inventory = INVENTORY.replace("qwen-llama-cpp-reference", adapter_id);
        let app = Router::new()
            .route(
                "/runtime-adapter/v1alpha1/capabilities",
                get(|State(state): State<FakeAdapter>| async move {
                    state.calls.lock().unwrap().push("capabilities");
                    adapter_response(&state, state.capabilities.clone()).await
                }),
            )
            .route(
                "/runtime-adapter/v1alpha1/inventory/compute",
                get(|State(state): State<FakeAdapter>| async move {
                    state.calls.lock().unwrap().push("inventory");
                    adapter_response(&state, state.inventory.clone()).await
                }),
            )
            .with_state(FakeAdapter {
                calls,
                capabilities,
                inventory,
                fail,
                delay,
                concurrency,
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{address}")
    }

    async fn adapter_response(state: &FakeAdapter, body: String) -> Response {
        let active = state
            .concurrency
            .active
            .fetch_add(1, AtomicOrdering::AcqRel)
            .saturating_add(1);
        state
            .concurrency
            .maximum
            .fetch_max(active, AtomicOrdering::AcqRel);
        tokio::time::sleep(state.delay).await;
        state
            .concurrency
            .active
            .fetch_sub(1, AtomicOrdering::AcqRel);
        if state.fail {
            return (StatusCode::BAD_GATEWAY, "secret upstream body").into_response();
        }
        ([(CONTENT_TYPE, "application/json")], body).into_response()
    }

    async fn wait_until(timeout: Duration, predicate: impl Fn() -> bool) {
        tokio::time::timeout(timeout, async {
            while !predicate() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}
