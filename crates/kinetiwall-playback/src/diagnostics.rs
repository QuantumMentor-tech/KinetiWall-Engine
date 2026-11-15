//! Runtime diagnostics, performance profiling, lifecycle timing analysis,
//! and bounded metric aggregation for the playback subsystem.

use std::collections::VecDeque;
use std::fmt;
use std::time::{Duration, Instant};

use kinetiwall_catalog::{WallpaperId, WallpaperType};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::health::{PlayerHealthStatus, PlayerInstanceId};
use crate::types::{DisplayTargetId, PlaybackSessionId};

/// Maximum number of diagnostic events retained per player target before FIFO eviction.
pub const MAX_RETAINED_EVENTS: usize = 100;

/// Maximum number of resource usage samples retained per player target before FIFO eviction.
pub const MAX_RETAINED_SAMPLES: usize = 60;

/// Category of runtime diagnostic event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticCategory {
    StartupRequested,
    BackendInitStarted,
    BackendReady,
    PlaybackStarted,
    PlaybackSuspended,
    PlaybackResumed,
    PlaybackStopped,
    InitTimedOut,
    BackendReportedError,
    IpcConnectionLost,
    ProcessExitedUnexpectedly,
    RecoveryAttemptStarted,
    RecoveryCompleted,
    RecoveryExhausted,
    ShutdownStarted,
    ShutdownCompleted,
    ResourceCleanupFailed,
    MeasurementUnavailable,
}

impl fmt::Display for DiagnosticCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// Structured, privacy-safe diagnostic event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticEvent {
    pub event_id: Uuid,
    pub instance_id: PlayerInstanceId,
    pub session_id: PlaybackSessionId,
    pub target_id: DisplayTargetId,
    pub wallpaper_id: WallpaperId,
    pub wallpaper_type: WallpaperType,
    pub category: DiagnosticCategory,
    pub timestamp_millis: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

impl DiagnosticEvent {
    #[must_use]
    pub fn new(
        instance_id: PlayerInstanceId,
        session_id: PlaybackSessionId,
        target_id: DisplayTargetId,
        wallpaper_id: WallpaperId,
        wallpaper_type: WallpaperType,
        category: DiagnosticCategory,
        timestamp_millis: u64,
    ) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            instance_id,
            session_id,
            target_id,
            wallpaper_id,
            wallpaper_type,
            category,
            timestamp_millis,
            detail: None,
            duration_ms: None,
        }
    }

    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    #[must_use]
    pub fn with_duration_ms(mut self, duration_ms: u64) -> Self {
        self.duration_ms = Some(duration_ms);
        self
    }
}

/// Monotonic timestamp recorder for key playback lifecycle milestones.
#[derive(Debug, Clone)]
pub struct LifecycleTimings {
    pub startup_requested_at: Instant,
    pub init_started_at: Option<Instant>,
    pub ready_at: Option<Instant>,
    pub playing_at: Option<Instant>,
    pub stop_requested_at: Option<Instant>,
    pub stopped_at: Option<Instant>,
    pub failed_at: Option<Instant>,
}

impl LifecycleTimings {
    #[must_use]
    pub fn new(now: Instant) -> Self {
        Self {
            startup_requested_at: now,
            init_started_at: None,
            ready_at: None,
            playing_at: None,
            stop_requested_at: None,
            stopped_at: None,
            failed_at: None,
        }
    }

    pub fn record_init_started(&mut self, now: Instant) {
        if self.init_started_at.is_none() {
            self.init_started_at = Some(now);
        }
    }

    pub fn record_ready(&mut self, now: Instant) {
        if self.ready_at.is_none() {
            self.ready_at = Some(now);
        }
    }

    pub fn record_playing(&mut self, now: Instant) {
        if self.playing_at.is_none() {
            self.playing_at = Some(now);
        }
    }

    pub fn record_stop_requested(&mut self, now: Instant) {
        if self.stop_requested_at.is_none() {
            self.stop_requested_at = Some(now);
        }
    }

    pub fn record_stopped(&mut self, now: Instant) {
        if self.stopped_at.is_none() {
            self.stopped_at = Some(now);
        }
    }

    pub fn record_failed(&mut self, now: Instant) {
        if self.failed_at.is_none() {
            self.failed_at = Some(now);
        }
    }

    /// Duration from initial start request to confirmed backend readiness.
    #[must_use]
    pub fn time_to_ready(&self) -> Option<Duration> {
        self.ready_at
            .map(|ready| ready.saturating_duration_since(self.startup_requested_at))
    }

    /// Duration from initial start request to confirmed playing state.
    #[must_use]
    pub fn time_to_playing(&self) -> Option<Duration> {
        self.playing_at
            .map(|play| play.saturating_duration_since(self.startup_requested_at))
    }

    /// Active duration while playing (until stop, fail, or current instant if still playing).
    #[must_use]
    pub fn active_duration(&self, now: Instant) -> Option<Duration> {
        self.playing_at.map(|play| {
            if let Some(stopped) = self.stopped_at {
                stopped.saturating_duration_since(play)
            } else if let Some(failed) = self.failed_at {
                failed.saturating_duration_since(play)
            } else {
                now.saturating_duration_since(play)
            }
        })
    }

    /// Duration required to execute a graceful stop.
    #[must_use]
    pub fn stop_duration(&self) -> Option<Duration> {
        match (self.stop_requested_at, self.stopped_at) {
            (Some(req), Some(stopped)) => Some(stopped.saturating_duration_since(req)),
            _ => None,
        }
    }

    /// Generates a serializable summary of all recorded lifecycle durations.
    #[must_use]
    pub fn summary(&self, now: Instant) -> LifecycleTimingSummary {
        LifecycleTimingSummary {
            time_to_ready_ms: self.time_to_ready().map(|d| d.as_millis() as u64),
            time_to_playing_ms: self.time_to_playing().map(|d| d.as_millis() as u64),
            active_duration_ms: self.active_duration(now).map(|d| d.as_millis() as u64),
            stop_duration_ms: self.stop_duration().map(|d| d.as_millis() as u64),
            has_failed: self.failed_at.is_some(),
        }
    }
}

/// Serializable representation of lifecycle durations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LifecycleTimingSummary {
    pub time_to_ready_ms: Option<u64>,
    pub time_to_playing_ms: Option<u64>,
    pub active_duration_ms: Option<u64>,
    pub stop_duration_ms: Option<u64>,
    pub has_failed: bool,
}

/// Point-in-time sample of process resource consumption.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessResourceSample {
    pub timestamp_millis: u64,
    pub pid: u32,
    /// CPU utilization percentage across all cores (0.0 to 100.0). None if first sample or unavailable.
    pub cpu_percent: Option<f32>,
    /// Working set memory in bytes (physical memory assigned to process).
    pub working_set_bytes: Option<u64>,
    /// Private commit memory in bytes.
    pub private_memory_bytes: Option<u64>,
    /// Number of active threads in the process.
    pub thread_count: Option<u32>,
}

impl ProcessResourceSample {
    #[must_use]
    pub fn new(timestamp_millis: u64, pid: u32) -> Self {
        Self {
            timestamp_millis,
            pid,
            cpu_percent: None,
            working_set_bytes: None,
            private_memory_bytes: None,
            thread_count: None,
        }
    }
}

/// Backend-specific performance and diagnostic telemetry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BackendPerformanceMetrics {
    /// Active hardware decoding mode (e.g. "d3d11va", "no") for video backends.
    pub hwdec_current: Option<String>,
    /// Duration of media loading in milliseconds.
    pub media_load_ms: Option<u64>,
    /// Web navigation latency in milliseconds.
    pub web_navigation_ms: Option<u64>,
    /// Count of audio-reactive frames delivered to renderer.
    pub audio_frames_delivered: Option<u64>,
}

/// Aggregated statistical summary of resource usage over recent sample history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ResourceSummary {
    pub sample_count: usize,
    pub average_cpu_percent: Option<f32>,
    pub peak_working_set_bytes: Option<u64>,
    pub peak_private_memory_bytes: Option<u64>,
    pub latest_working_set_bytes: Option<u64>,
}

/// Authoritative point-in-time diagnostic report for a supervised target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticsSnapshot {
    pub instance_id: PlayerInstanceId,
    pub session_id: PlaybackSessionId,
    pub target_id: DisplayTargetId,
    pub wallpaper_id: WallpaperId,
    pub wallpaper_type: WallpaperType,
    pub health_status: PlayerHealthStatus,
    pub timings: LifecycleTimingSummary,
    pub latest_sample: Option<ProcessResourceSample>,
    pub resource_summary: ResourceSummary,
    pub backend_metrics: BackendPerformanceMetrics,
    pub recovery_attempts: u32,
    pub recent_events: Vec<DiagnosticEvent>,
}

/// In-memory bounded aggregator retaining recent diagnostic events and metrics.
#[derive(Debug, Clone)]
pub struct DiagnosticsAggregator {
    events: VecDeque<DiagnosticEvent>,
    samples: VecDeque<ProcessResourceSample>,
    backend_metrics: BackendPerformanceMetrics,
    total_failures: u32,
    total_recoveries: u32,
}

impl Default for DiagnosticsAggregator {
    fn default() -> Self {
        Self::new()
    }
}

impl DiagnosticsAggregator {
    #[must_use]
    pub fn new() -> Self {
        Self {
            events: VecDeque::with_capacity(MAX_RETAINED_EVENTS),
            samples: VecDeque::with_capacity(MAX_RETAINED_SAMPLES),
            backend_metrics: BackendPerformanceMetrics::default(),
            total_failures: 0,
            total_recoveries: 0,
        }
    }

    /// Records a diagnostic event, evicting the oldest event if capacity is reached.
    pub fn record_event(&mut self, event: DiagnosticEvent) {
        match event.category {
            DiagnosticCategory::BackendReportedError
            | DiagnosticCategory::ProcessExitedUnexpectedly
            | DiagnosticCategory::InitTimedOut => {
                self.total_failures = self.total_failures.saturating_add(1);
            }
            DiagnosticCategory::RecoveryCompleted => {
                self.total_recoveries = self.total_recoveries.saturating_add(1);
            }
            _ => {}
        }

        if self.events.len() >= MAX_RETAINED_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    /// Records a resource usage sample, evicting the oldest sample if capacity is reached.
    pub fn record_sample(&mut self, sample: ProcessResourceSample) {
        if self.samples.len() >= MAX_RETAINED_SAMPLES {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    /// Updates backend-specific performance metrics.
    pub fn update_backend_metrics(&mut self, metrics: BackendPerformanceMetrics) {
        self.backend_metrics = metrics;
    }

    #[must_use]
    pub fn events(&self) -> Vec<DiagnosticEvent> {
        self.events.iter().cloned().collect()
    }

    #[must_use]
    pub fn samples(&self) -> Vec<ProcessResourceSample> {
        self.samples.iter().cloned().collect()
    }

    #[must_use]
    pub fn latest_sample(&self) -> Option<ProcessResourceSample> {
        self.samples.back().cloned()
    }

    #[must_use]
    pub fn backend_metrics(&self) -> &BackendPerformanceMetrics {
        &self.backend_metrics
    }

    #[must_use]
    pub fn total_failures(&self) -> u32 {
        self.total_failures
    }

    #[must_use]
    pub fn total_recoveries(&self) -> u32 {
        self.total_recoveries
    }

    /// Computes statistical resource summary over retained samples.
    #[must_use]
    pub fn summarize_resources(&self) -> ResourceSummary {
        let sample_count = self.samples.len();
        if sample_count == 0 {
            return ResourceSummary::default();
        }

        let mut cpu_sum = 0.0f32;
        let mut cpu_count = 0usize;
        let mut peak_ws: Option<u64> = None;
        let mut peak_priv: Option<u64> = None;
        let latest_ws = self.samples.back().and_then(|s| s.working_set_bytes);

        for s in &self.samples {
            if let Some(cpu) = s.cpu_percent {
                if cpu.is_finite() && cpu >= 0.0 {
                    cpu_sum += cpu;
                    cpu_count += 1;
                }
            }
            if let Some(ws) = s.working_set_bytes {
                peak_ws = Some(peak_ws.map_or(ws, |p| p.max(ws)));
            }
            if let Some(priv_mem) = s.private_memory_bytes {
                peak_priv = Some(peak_priv.map_or(priv_mem, |p| p.max(priv_mem)));
            }
        }

        let average_cpu_percent = if cpu_count > 0 {
            Some(cpu_sum / cpu_count as f32)
        } else {
            None
        };

        ResourceSummary {
            sample_count,
            average_cpu_percent,
            peak_working_set_bytes: peak_ws,
            peak_private_memory_bytes: peak_priv,
            latest_working_set_bytes: latest_ws,
        }
    }

    /// Resets all retained samples and events on target shutdown or replacement.
    pub fn clear(&mut self) {
        self.events.clear();
        self.samples.clear();
        self.backend_metrics = BackendPerformanceMetrics::default();
    }
}
