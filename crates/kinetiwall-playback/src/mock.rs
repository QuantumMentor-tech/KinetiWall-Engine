//! Deterministic fake adapter for contract tests. Simulates scripted outcomes only; it is NOT
//! evidence that any real playback works.

use std::sync::{Arc, Mutex};

use crate::adapter::{CancelAck, PlayerAdapter, PrepareProgress, SessionContext, StopAck};
use crate::types::{PlaybackError, PlaybackErrorKind, PlayerAvailability, PlayerCapabilities};
use kinetiwall_catalog::WallpaperType;

#[derive(Debug, Clone)]
pub struct MockScript {
    pub capabilities: PlayerCapabilities,
    pub availability: PlayerAvailability,
    pub prepare: Result<PrepareProgress, PlaybackErrorKind>,
    pub start: Result<(), PlaybackErrorKind>,
    pub cancel: Result<CancelAck, PlaybackErrorKind>,
    pub stop: Result<StopAck, PlaybackErrorKind>,
    pub shutdown: Result<(), PlaybackErrorKind>,
}

impl MockScript {
    #[must_use]
    pub fn happy(wallpaper_type: WallpaperType) -> Self {
        Self {
            capabilities: PlayerCapabilities {
                wallpaper_type,
                supports_cancellation: true,
                reports_observed_state: true,
            },
            availability: PlayerAvailability::Available,
            prepare: Ok(PrepareProgress::Ready),
            start: Ok(()),
            cancel: Ok(CancelAck::Accepted),
            stop: Ok(StopAck::Stopped),
            shutdown: Ok(()),
        }
    }
}

/// Shared handle so tests can alter the script and inspect calls after the adapter is boxed.
#[derive(Debug, Clone)]
pub struct MockHandle(Arc<Mutex<(MockScript, Vec<String>)>>);

impl MockHandle {
    pub fn script(&self, f: impl FnOnce(&mut MockScript)) {
        f(&mut self.0.lock().unwrap().0);
    }
    #[must_use]
    pub fn calls(&self) -> Vec<String> {
        self.0.lock().unwrap().1.clone()
    }
    #[must_use]
    pub fn count(&self, call: &str) -> usize {
        self.calls().iter().filter(|c| c.as_str() == call).count()
    }
}

pub struct MockPlayerAdapter(MockHandle);

impl MockPlayerAdapter {
    #[must_use]
    pub fn new(script: MockScript) -> (Self, MockHandle) {
        let handle = MockHandle(Arc::new(Mutex::new((script, Vec::new()))));
        (Self(handle.clone()), handle)
    }

    fn rec<T>(
        &self,
        name: &str,
        pick: impl FnOnce(&MockScript) -> Result<T, PlaybackErrorKind>,
    ) -> Result<T, PlaybackError> {
        let mut g = (self.0).0.lock().unwrap();
        g.1.push(name.to_string());
        pick(&g.0).map_err(|k| PlaybackError::new(k, format!("mock {name} scripted failure")))
    }
}

impl PlayerAdapter for MockPlayerAdapter {
    fn capabilities(&self) -> PlayerCapabilities {
        (self.0).0.lock().unwrap().0.capabilities
    }
    fn availability(&self) -> PlayerAvailability {
        (self.0).0.lock().unwrap().0.availability.clone()
    }
    fn prepare(&mut self, _ctx: &SessionContext) -> Result<PrepareProgress, PlaybackError> {
        self.rec("prepare", |s| s.prepare)
    }
    fn start(&mut self, _ctx: &SessionContext) -> Result<(), PlaybackError> {
        self.rec("start", |s| s.start)
    }
    fn cancel(&mut self, _ctx: &SessionContext) -> Result<CancelAck, PlaybackError> {
        self.rec("cancel", |s| s.cancel)
    }
    fn stop(&mut self, _ctx: &SessionContext) -> Result<StopAck, PlaybackError> {
        self.rec("stop", |s| s.stop)
    }
    fn shutdown(&mut self) -> Result<(), PlaybackError> {
        self.rec("shutdown", |s| s.shutdown)
    }
}
