use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::mpsc;


pub const MAX_ROLLING_WINDOW: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPath {
    FastPath,
    CloudFast,
    CloudHeavy,
    LocalFallback,
}

impl ExecutionPath {
    pub fn is_fast_path(&self) -> bool {
        matches!(self, ExecutionPath::FastPath)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ExecutionPath::FastPath => "fast_path",
            ExecutionPath::CloudFast => "cloud_fast",
            ExecutionPath::CloudHeavy => "cloud_heavy",
            ExecutionPath::LocalFallback => "local_fallback",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnTelemetryRecord {
    pub turn_id: u32,
    pub path: ExecutionPath,
    pub routing_reason: String,
    pub ipc_rx_to_router_ms: f64,
    pub router_ms: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_recall_ms: Option<f64>,
    pub llm_ttft_ms: Option<f64>,
    pub first_tts_chunk_dispatch_ms: Option<f64>,
    pub llm_total_ms: Option<f64>,
    pub total_daemon_turn_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagePercentiles {
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub sample_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetrySummary {
    pub fast_path_summary: StagePercentiles,
    pub cloud_path_summary: StagePercentiles,
    pub total_turns_in_window: usize,
}

pub fn calculate_percentiles(mut values: Vec<f64>) -> StagePercentiles {
    let sample_count = values.len();
    if values.is_empty() {
        return StagePercentiles {
            p50_ms: 0.0,
            p95_ms: 0.0,
            sample_count: 0,
        };
    }

    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let idx50 = ((sample_count as f64) * 0.50).floor() as usize;
    let idx50 = idx50.min(sample_count - 1);

    let idx95 = ((sample_count as f64) * 0.95).floor() as usize;
    let idx95 = idx95.min(sample_count - 1);

    StagePercentiles {
        p50_ms: values[idx50],
        p95_ms: values[idx95],
        sample_count,
    }
}

pub struct ActiveTurnTiming {
    pub turn_id: u32,
    pub t_ipc_rx: Instant,
    pub t_router_start: Option<Instant>,
    pub t_router_done: Option<Instant>,
    pub memory_recall_ms: Option<f64>,
    pub t_llm_start: Option<Instant>,
    pub t_llm_first_token: Option<Instant>,
    pub t_first_tts_dispatch: Option<Instant>,
    pub t_llm_done: Option<Instant>,
    pub path: ExecutionPath,
    pub routing_reason: String,
}

impl ActiveTurnTiming {
    pub fn new(turn_id: u32) -> Self {
        Self {
            turn_id,
            t_ipc_rx: Instant::now(),
            t_router_start: None,
            t_router_done: None,
            memory_recall_ms: None,
            t_llm_start: None,
            t_llm_first_token: None,
            t_first_tts_dispatch: None,
            t_llm_done: None,
            path: ExecutionPath::FastPath,
            routing_reason: "default".to_string(),
        }
    }
}

#[derive(Clone)]
pub struct DaemonTelemetry {
    inner: Arc<Mutex<TelemetryInner>>,
    log_tx: mpsc::UnboundedSender<TurnTelemetryRecord>,
}

struct TelemetryInner {
    fast_path_turns: VecDeque<TurnTelemetryRecord>,
    cloud_path_turns: VecDeque<TurnTelemetryRecord>,
}

impl DaemonTelemetry {
    pub fn new() -> Self {
        Self::with_log_path(None)
    }

    pub fn with_log_path(custom_path: Option<PathBuf>) -> Self {
        let (log_tx, mut log_rx) = mpsc::unbounded_channel::<TurnTelemetryRecord>();
        let target_path = custom_path.unwrap_or_else(Self::resolve_log_path);

        // Spawn dedicated disk persistence worker off Tokio workers
        std::thread::spawn(move || {
            if let Some(parent) = target_path.parent() {
                let _ = create_dir_all(parent);
            }

            while let Some(record) = log_rx.blocking_recv() {
                if let Ok(line) = serde_json::to_string(&record) {
                    if let Ok(mut file) = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&target_path)
                    {
                        let _ = writeln!(file, "{}", line);
                    }
                }
            }
        });

        Self {
            inner: Arc::new(Mutex::new(TelemetryInner {
                fast_path_turns: VecDeque::with_capacity(MAX_ROLLING_WINDOW),
                cloud_path_turns: VecDeque::with_capacity(MAX_ROLLING_WINDOW),
            })),
            log_tx,
        }
    }

    fn resolve_log_path() -> PathBuf {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            PathBuf::from(local_app_data)
                .join("Better_Jarvis")
                .join("logs")
                .join("latency_metrics.jsonl")
        } else {
            PathBuf::from("logs").join("latency_metrics.jsonl")
        }
    }

    pub fn record_completed_turn(&self, timing: ActiveTurnTiming) {
        let now = Instant::now();
        let total_daemon_turn_ms = (now - timing.t_ipc_rx).as_secs_f64() * 1000.0;

        let ipc_rx_to_router_ms = timing
            .t_router_start
            .map(|t| (t - timing.t_ipc_rx).as_secs_f64() * 1000.0)
            .unwrap_or(0.0);

        let router_ms = match (timing.t_router_start, timing.t_router_done) {
            (Some(s), Some(d)) => (d - s).as_secs_f64() * 1000.0,
            _ => 0.0,
        };

        let llm_ttft_ms = match (timing.t_llm_start, timing.t_llm_first_token) {
            (Some(s), Some(ft)) => Some((ft - s).as_secs_f64() * 1000.0),
            _ => None,
        };

        let first_tts_chunk_dispatch_ms = timing
            .t_first_tts_dispatch
            .map(|t| (t - timing.t_ipc_rx).as_secs_f64() * 1000.0);

        let llm_total_ms = match (timing.t_llm_start, timing.t_llm_done) {
            (Some(s), Some(d)) => Some((d - s).as_secs_f64() * 1000.0),
            _ => None,
        };

        let record = TurnTelemetryRecord {
            turn_id: timing.turn_id,
            path: timing.path,
            routing_reason: timing.routing_reason,
            ipc_rx_to_router_ms,
            router_ms,
            memory_recall_ms: timing.memory_recall_ms,
            llm_ttft_ms,
            first_tts_chunk_dispatch_ms,
            llm_total_ms,
            total_daemon_turn_ms,
        };

        self.insert_record(record.clone());
        let _ = self.log_tx.send(record);
    }

    pub fn insert_record(&self, record: TurnTelemetryRecord) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let target_deque = if record.path.is_fast_path() {
            &mut inner.fast_path_turns
        } else {
            &mut inner.cloud_path_turns
        };

        if target_deque.len() >= MAX_ROLLING_WINDOW {
            target_deque.pop_front();
        }
        target_deque.push_back(record);
    }

    pub fn get_summary(&self) -> TelemetrySummary {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());

        let fast_durations: Vec<f64> = inner
            .fast_path_turns
            .iter()
            .map(|r| r.total_daemon_turn_ms)
            .collect();

        let cloud_durations: Vec<f64> = inner
            .cloud_path_turns
            .iter()
            .map(|r| r.total_daemon_turn_ms)
            .collect();

        let total_turns_in_window = fast_durations.len() + cloud_durations.len();

        TelemetrySummary {
            fast_path_summary: calculate_percentiles(fast_durations),
            cloud_path_summary: calculate_percentiles(cloud_durations),
            total_turns_in_window,
        }
    }
}

impl Default for DaemonTelemetry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percentile_calculation_exact() {
        let mut vals = Vec::new();
        for i in 1..=100 {
            vals.push(i as f64);
        }

        let p = calculate_percentiles(vals);
        assert_eq!(p.sample_count, 100);
        // Sorted 1..=100: idx 50 is 51.0, idx 95 is 96.0
        assert!((p.p50_ms - 51.0).abs() < 1e-4);
        assert!((p.p95_ms - 96.0).abs() < 1e-4);
    }

    #[test]
    fn test_rolling_window_capacity_cap() {
        let telemetry = DaemonTelemetry::new();

        // Push 250 records into fast_path
        for i in 1..=250 {
            telemetry.insert_record(TurnTelemetryRecord {
                turn_id: i,
                path: ExecutionPath::FastPath,
                routing_reason: "fast_intent".to_string(),
                ipc_rx_to_router_ms: 1.0,
                router_ms: 2.0,
                memory_recall_ms: None,
                llm_ttft_ms: None,
                first_tts_chunk_dispatch_ms: Some(5.0),
                llm_total_ms: None,
                total_daemon_turn_ms: i as f64,
            });
        }

        let summary = telemetry.get_summary();
        assert_eq!(summary.fast_path_summary.sample_count, 200);
        assert_eq!(summary.cloud_path_summary.sample_count, 0);

        // Window contains items 51..=250.
        // Item at idx 50: 51 + 100 = 151.0
        // Item at idx 95: 51 + 190 = 241.0
        assert!((summary.fast_path_summary.p50_ms - 151.0).abs() < 1e-4);
        assert!((summary.fast_path_summary.p95_ms - 241.0).abs() < 1e-4);
    }

    #[test]
    fn test_fast_and_cloud_path_separation() {
        let telemetry = DaemonTelemetry::new();

        telemetry.insert_record(TurnTelemetryRecord {
            turn_id: 1,
            path: ExecutionPath::FastPath,
            routing_reason: "volume".to_string(),
            ipc_rx_to_router_ms: 0.5,
            router_ms: 1.0,
            memory_recall_ms: None,
            llm_ttft_ms: None,
            first_tts_chunk_dispatch_ms: Some(2.0),
            llm_total_ms: None,
            total_daemon_turn_ms: 2.5,
        });

        telemetry.insert_record(TurnTelemetryRecord {
            turn_id: 2,
            path: ExecutionPath::CloudFast,
            routing_reason: "cloud_qa".to_string(),
            ipc_rx_to_router_ms: 0.5,
            router_ms: 1.0,
            memory_recall_ms: None,
            llm_ttft_ms: Some(180.0),
            first_tts_chunk_dispatch_ms: Some(220.0),
            llm_total_ms: Some(450.0),
            total_daemon_turn_ms: 455.0,
        });

        let summary = telemetry.get_summary();
        assert_eq!(summary.fast_path_summary.sample_count, 1);
        assert_eq!(summary.cloud_path_summary.sample_count, 1);
        assert_eq!(summary.total_turns_in_window, 2);

        assert!((summary.fast_path_summary.p50_ms - 2.5).abs() < 1e-4);
        assert!((summary.cloud_path_summary.p50_ms - 455.0).abs() < 1e-4);
    }

    #[test]
    fn test_telemetry_serializes_memory_recall_ms() {
        let record = TurnTelemetryRecord {
            turn_id: 42,
            path: ExecutionPath::CloudFast,
            routing_reason: "general_query".into(),
            ipc_rx_to_router_ms: 0.12,
            router_ms: 0.45,
            memory_recall_ms: Some(8.75),
            llm_ttft_ms: Some(210.5),
            first_tts_chunk_dispatch_ms: Some(250.0),
            llm_total_ms: Some(400.0),
            total_daemon_turn_ms: 410.0,
        };

        let json_str = serde_json::to_string(&record).unwrap();
        assert!(json_str.contains("\"memory_recall_ms\":8.75"));

        let deserialized: TurnTelemetryRecord = serde_json::from_str(&json_str).unwrap();
        assert_eq!(deserialized.memory_recall_ms, Some(8.75));
        assert_eq!(deserialized.turn_id, 42);
    }

    #[test]
    fn test_telemetry_recovers_from_poisoned_mutex() {
        let telemetry = std::sync::Arc::new(DaemonTelemetry::new());
        let telemetry_clone = telemetry.clone();
        let _ = std::thread::spawn(move || {
            let _guard = telemetry_clone.inner.lock().unwrap();
            panic!("simulated panic while holding telemetry lock");
        })
        .join();

        let summary = telemetry.get_summary();
        assert_eq!(summary.total_turns_in_window, 0);
    }
}
