//! Controller-side `PlayerAdapter` implementation that spawns and supervises the
//! isolated `kinetiwall-player-web.exe` process over anonymous pipes (stdin/stdout).

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use kinetiwall_catalog::{WallpaperId, WallpaperType};
use kinetiwall_playback::{
    BackendCapabilities, BackendPerformanceMetrics, CancelAck, PlaybackError, PlaybackErrorKind,
    PlaybackSessionId, PlayerAdapter, PlayerAvailability, PlayerCapabilities, PlayerReport,
    PrepareProgress, ProcessResourceSample, RuntimeConfigUpdate, SessionContext, StopAck,
};

use crate::protocol::{FromHost, HostOp, ToHost};

pub struct WebPlayerAdapter {
    binary_path: Option<String>,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    from_host_rx: Option<Receiver<FromHost>>,
    reader_thread: Option<thread::JoinHandle<()>>,
    _current_session: Option<PlaybackSessionId>,
    catalog_lookup: Arc<dyn Fn(WallpaperId) -> Option<String> + Send + Sync>,
    prev_cpu_time: Option<(u64, Instant)>,
}

impl WebPlayerAdapter {
    pub fn new<F>(catalog_lookup: F) -> Self
    where
        F: Fn(WallpaperId) -> Option<String> + Send + Sync + 'static,
    {
        Self {
            binary_path: None,
            child: None,
            stdin: None,
            from_host_rx: None,
            reader_thread: None,
            _current_session: None,
            catalog_lookup: Arc::new(catalog_lookup),
            prev_cpu_time: None,
        }
    }

    pub fn with_binary_path<F>(binary_path: String, catalog_lookup: F) -> Self
    where
        F: Fn(WallpaperId) -> Option<String> + Send + Sync + 'static,
    {
        Self {
            binary_path: Some(binary_path),
            child: None,
            stdin: None,
            from_host_rx: None,
            reader_thread: None,
            _current_session: None,
            catalog_lookup: Arc::new(catalog_lookup),
            prev_cpu_time: None,
        }
    }

    /// Dispatches a bounded audio visualizer update frame to the isolated player process.
    pub fn send_visualizer_frame(
        &mut self,
        session_id: PlaybackSessionId,
        frame: kinetiwall_audio::VisualizerFrame,
    ) -> Result<(), PlaybackError> {
        self.send_cmd(&ToHost::VisualizerUpdate { session_id, frame })
    }

    fn ensure_process_running(&mut self) -> Result<(), PlaybackError> {
        if self.child.is_some() {
            return Ok(());
        }

        let bin = match &self.binary_path {
            Some(p) => p.clone(),
            None => {
                let current_exe = std::env::current_exe().unwrap_or_default();
                let dir = current_exe.parent().unwrap_or(std::path::Path::new("."));
                let cand1 = dir.join("kinetiwall-player-web.exe");
                let cand2 = dir.join("kinetiwall-player-web");
                if cand1.exists() {
                    cand1.to_string_lossy().into_owned()
                } else if cand2.exists() {
                    cand2.to_string_lossy().into_owned()
                } else {
                    "kinetiwall-player-web".into()
                }
            }
        };

        let mut child = Command::new(&bin)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| {
                PlaybackError::new(
                    PlaybackErrorKind::BackendUnavailable,
                    format!("Failed to spawn `{bin}`: {e}"),
                )
            })?;

        let stdin = child.stdin.take().ok_or_else(|| {
            PlaybackError::new(
                PlaybackErrorKind::Internal,
                "Child process stdin unavailable",
            )
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            PlaybackError::new(
                PlaybackErrorKind::Internal,
                "Child process stdout unavailable",
            )
        })?;

        let (tx, rx) = channel::<FromHost>();
        let reader_thread = thread::Builder::new()
            .name("web-player-reader".into())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                let mut line = String::new();
                while let Ok(bytes) = reader.read_line(&mut line) {
                    if bytes == 0 {
                        break;
                    }
                    let trimmed = line.trim();
                    if let Ok(msg) = serde_json::from_str::<FromHost>(trimmed) {
                        if tx.send(msg).is_err() {
                            break;
                        }
                    }
                    line.clear();
                }
            })
            .map_err(|e| {
                PlaybackError::new(
                    PlaybackErrorKind::Internal,
                    format!("Failed to spawn reader thread: {e}"),
                )
            })?;

        self.child = Some(child);
        self.stdin = Some(stdin);
        self.from_host_rx = Some(rx);
        self.reader_thread = Some(reader_thread);
        Ok(())
    }

    fn send_cmd(&mut self, cmd: &ToHost) -> Result<(), PlaybackError> {
        let stdin = self.stdin.as_mut().ok_or_else(|| {
            PlaybackError::new(PlaybackErrorKind::Internal, "Player process not running")
        })?;
        let json = serde_json::to_string(cmd).map_err(|e| {
            PlaybackError::new(PlaybackErrorKind::Internal, format!("JSON error: {e}"))
        })?;
        writeln!(stdin, "{json}").map_err(|e| {
            PlaybackError::new(PlaybackErrorKind::Internal, format!("Write error: {e}"))
        })?;
        stdin.flush().map_err(|e| {
            PlaybackError::new(PlaybackErrorKind::Internal, format!("Flush error: {e}"))
        })?;
        Ok(())
    }

    pub fn poll_events(&mut self) -> Vec<FromHost> {
        let mut events = Vec::new();
        if let Some(rx) = &self.from_host_rx {
            while let Ok(msg) = rx.try_recv() {
                events.push(msg);
            }
        }
        events
    }
}

impl PlayerAdapter for WebPlayerAdapter {
    fn capabilities(&self) -> PlayerCapabilities {
        PlayerCapabilities {
            wallpaper_type: WallpaperType::Web,
            supports_cancellation: true,
            reports_observed_state: true,
        }
    }

    fn availability(&self) -> PlayerAvailability {
        PlayerAvailability::Available
    }

    fn prepare(&mut self, ctx: &SessionContext) -> Result<PrepareProgress, PlaybackError> {
        self.ensure_process_running()?;
        self._current_session = Some(ctx.session_id);
        self.send_cmd(&ToHost::Prepare {
            session_id: ctx.session_id,
        })?;

        if let Some(rx) = &self.from_host_rx {
            let start = std::time::Instant::now();
            while start.elapsed() < Duration::from_millis(500) {
                if let Ok(msg) = rx.recv_timeout(Duration::from_millis(50)) {
                    match msg {
                        FromHost::Ack {
                            session_id,
                            op: HostOp::Prepare,
                            ok,
                            error,
                        } if session_id == ctx.session_id => {
                            if ok {
                                return Ok(PrepareProgress::Ready);
                            } else {
                                return Err(PlaybackError::new(
                                    error.unwrap_or(PlaybackErrorKind::PreparationFailed),
                                    "Host preparation failed",
                                ));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(PrepareProgress::Pending)
    }

    fn start(&mut self, ctx: &SessionContext) -> Result<(), PlaybackError> {
        let entrypoint_path = (self.catalog_lookup)(ctx.wallpaper_id).ok_or_else(|| {
            PlaybackError::new(
                PlaybackErrorKind::AssetUnavailable,
                "Wallpaper entrypoint path not found in catalog",
            )
        })?;

        self.send_cmd(&ToHost::Start {
            session_id: ctx.session_id,
            entrypoint_path,
        })?;

        if let Some(rx) = &self.from_host_rx {
            let start = std::time::Instant::now();
            while start.elapsed() < Duration::from_millis(500) {
                if let Ok(msg) = rx.recv_timeout(Duration::from_millis(50)) {
                    match msg {
                        FromHost::Ack {
                            session_id,
                            op: HostOp::Start,
                            ok,
                            error,
                        } if session_id == ctx.session_id => {
                            if ok {
                                self._current_session = Some(ctx.session_id);
                                return Ok(());
                            } else {
                                return Err(PlaybackError::new(
                                    error.unwrap_or(PlaybackErrorKind::StartupFailed),
                                    "Host start failed",
                                ));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    }

    fn cancel(&mut self, ctx: &SessionContext) -> Result<CancelAck, PlaybackError> {
        let _ = self.send_cmd(&ToHost::Stop {
            session_id: ctx.session_id,
        });
        Ok(CancelAck::Accepted)
    }

    fn stop(&mut self, ctx: &SessionContext) -> Result<StopAck, PlaybackError> {
        let _ = self.send_cmd(&ToHost::Stop {
            session_id: ctx.session_id,
        });
        self._current_session = None;
        Ok(StopAck::Stopped)
    }

    fn shutdown(&mut self) -> Result<(), PlaybackError> {
        let _ = self.send_cmd(&ToHost::Shutdown);
        if let Some(mut child) = self.child.take() {
            let _ = child.wait();
        }
        self.stdin = None;
        self.from_host_rx = None;
        if let Some(handle) = self.reader_thread.take() {
            let _ = handle.join();
        }
        Ok(())
    }

    fn poll_reports(&mut self) -> Vec<(PlaybackSessionId, PlayerReport)> {
        let mut reports = Vec::new();
        if let Some(child) = &mut self.child {
            if let Ok(Some(_status)) = child.try_wait() {
                if let Some(session_id) = self._current_session.take() {
                    reports.push((
                        session_id,
                        PlayerReport::Failed {
                            kind: PlaybackErrorKind::BackendExited,
                        },
                    ));
                }
            }
        }
        for ev in self.poll_events() {
            if let FromHost::Report { session_id, report } = ev {
                reports.push((session_id, report));
            }
        }
        reports
    }

    fn is_alive(&mut self, _ctx: &SessionContext) -> bool {
        if let Some(child) = &mut self.child {
            matches!(child.try_wait(), Ok(None))
        } else {
            false
        }
    }

    fn backend_capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::for_web(true)
    }

    fn apply_runtime_config(
        &mut self,
        _ctx: &SessionContext,
        _update: &RuntimeConfigUpdate,
    ) -> Result<(), PlaybackError> {
        // Web wallpapers do not expose dynamic volume or mute directly
        Ok(())
    }

    fn sample_metrics(&mut self, _ctx: &SessionContext) -> Option<ProcessResourceSample> {
        let pid = self.child.as_ref()?.id();
        crate::sampler::sample_child_process(pid, &mut self.prev_cpu_time)
    }

    fn backend_metrics(&self, _ctx: &SessionContext) -> BackendPerformanceMetrics {
        BackendPerformanceMetrics {
            hwdec_current: None,
            media_load_ms: None,
            web_navigation_ms: None,
            audio_frames_delivered: None,
        }
    }

    fn process_id(&self, _ctx: &SessionContext) -> Option<u32> {
        self.child.as_ref().map(|c| c.id())
    }
}

impl Drop for WebPlayerAdapter {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
