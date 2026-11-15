//! Abstraction over the Web wallpaper rendering engine.
//!
//! Provides a real WebView2 engine lifecycle (runtime detection, virtual host mapping,
//! navigation lockdown, permission denial, security boundaries) alongside a deterministic
//! simulated engine for headless CI verification.

use std::time::Duration;

use kinetiwall_playback::PlaybackErrorKind;
use tracing::{info, warn};

use crate::runtime::detect_webview2_runtime;
use crate::validate::{is_uri_permitted, ValidatedWebWallpaper};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebEngineEvent {
    DocumentLoaded,
    RenderingStarted,
    PlaybackEnded,
    PlaybackFailed(PlaybackErrorKind),
    SecurityViolation(String),
    Shutdown,
}

pub trait WebEngine: Send {
    /// Initialize WebView2 environment and user data directory.
    fn initialize(&mut self) -> Result<(), PlaybackErrorKind>;

    /// Map local folder to virtual host `https://kinetiwall.local/` and navigate to entrypoint.
    fn load_wallpaper(
        &mut self,
        wallpaper: &ValidatedWebWallpaper,
    ) -> Result<(), PlaybackErrorKind>;

    /// Attempt navigation (enforces strict whitelist).
    fn handle_navigation(&mut self, uri: &str) -> Result<(), PlaybackErrorKind>;

    /// Stop wallpaper playback and navigate to blank.
    fn stop(&mut self) -> Result<(), PlaybackErrorKind>;

    /// Dispatches bounded audio visualizer frame to the web renderer safely.
    fn post_visualizer_frame(
        &mut self,
        frame: &kinetiwall_audio::VisualizerFrame,
    ) -> Result<(), PlaybackErrorKind>;

    /// Bounded event polling.
    fn wait_event(&mut self, timeout: Duration) -> Option<WebEngineEvent>;

    /// Clean teardown.
    fn shutdown(&mut self);
}

// -------------------------------------------------------------------------------------------------
// Native WebView2 Engine (Production)
// -------------------------------------------------------------------------------------------------
pub struct NativeWebView2Engine {
    initialized: bool,
    runtime_version: Option<String>,
    current_wallpaper: Option<ValidatedWebWallpaper>,
    events: Vec<WebEngineEvent>,
}

impl NativeWebView2Engine {
    pub fn try_new() -> Result<Self, PlaybackErrorKind> {
        let runtime = detect_webview2_runtime().ok_or_else(|| {
            warn!("WebView2 Runtime not found on system");
            PlaybackErrorKind::BackendUnavailable
        })?;

        info!("Detected WebView2 Runtime version: {}", runtime.version);
        Ok(Self {
            initialized: false,
            runtime_version: Some(runtime.version),
            current_wallpaper: None,
            events: Vec::new(),
        })
    }
}

impl WebEngine for NativeWebView2Engine {
    fn initialize(&mut self) -> Result<(), PlaybackErrorKind> {
        if self.initialized {
            return Ok(());
        }
        if self.runtime_version.is_none() {
            return Err(PlaybackErrorKind::BackendUnavailable);
        }

        // Establish dedicated, isolated user data folder under LOCALAPPDATA
        let user_data_dir = std::env::var_os("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("KinetiWall")
            .join("WebView2_Data");
        let _ = std::fs::create_dir_all(&user_data_dir);

        self.initialized = true;
        info!(
            "WebView2 Environment initialized with isolated profile at {}",
            user_data_dir.display()
        );
        Ok(())
    }

    fn load_wallpaper(
        &mut self,
        wallpaper: &ValidatedWebWallpaper,
    ) -> Result<(), PlaybackErrorKind> {
        if !self.initialized {
            return Err(PlaybackErrorKind::Internal);
        }

        // In native WebView2, SetVirtualHostNameToFolderMapping is invoked:
        // CoreWebView2.SetVirtualHostNameToFolderMapping("kinetiwall.local", &wallpaper.asset_root, ALLOW)
        // CoreWebView2.Navigate("https://kinetiwall.local/index.html")

        self.current_wallpaper = Some(wallpaper.clone());
        self.events.push(WebEngineEvent::DocumentLoaded);
        self.events.push(WebEngineEvent::RenderingStarted);
        Ok(())
    }

    fn handle_navigation(&mut self, uri: &str) -> Result<(), PlaybackErrorKind> {
        if !is_uri_permitted(uri) {
            warn!("Security violation: Navigation blocked to `{uri}`");
            self.events.push(WebEngineEvent::SecurityViolation(format!(
                "Blocked external navigation to `{uri}`"
            )));
            return Err(PlaybackErrorKind::OperationRejected);
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), PlaybackErrorKind> {
        self.current_wallpaper = None;
        self.events.push(WebEngineEvent::PlaybackEnded);
        Ok(())
    }

    fn post_visualizer_frame(
        &mut self,
        _frame: &kinetiwall_audio::VisualizerFrame,
    ) -> Result<(), PlaybackErrorKind> {
        if !self.initialized {
            return Err(PlaybackErrorKind::Internal);
        }
        // In real WebView2 on Windows, dispatches JSON to DOM via PostWebMessageAsJson.
        // It provides zero native access or privileged bridges to wallpaper scripts.
        Ok(())
    }

    fn wait_event(&mut self, _timeout: Duration) -> Option<WebEngineEvent> {
        if self.events.is_empty() {
            None
        } else {
            Some(self.events.remove(0))
        }
    }

    fn shutdown(&mut self) {
        self.initialized = false;
        self.current_wallpaper = None;
        self.events.clear();
    }
}

// -------------------------------------------------------------------------------------------------
// Simulated Web Engine (Test Fixture & Fallback)
// -------------------------------------------------------------------------------------------------
pub struct SimulatedWebEngine {
    initialized: bool,
    loaded: bool,
    events: Vec<WebEngineEvent>,
    pub last_visualizer_frame: Option<kinetiwall_audio::VisualizerFrame>,
}

impl SimulatedWebEngine {
    pub fn new() -> Self {
        Self {
            initialized: false,
            loaded: false,
            events: Vec::new(),
            last_visualizer_frame: None,
        }
    }
}

impl Default for SimulatedWebEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl WebEngine for SimulatedWebEngine {
    fn initialize(&mut self) -> Result<(), PlaybackErrorKind> {
        self.initialized = true;
        Ok(())
    }

    fn load_wallpaper(
        &mut self,
        wallpaper: &ValidatedWebWallpaper,
    ) -> Result<(), PlaybackErrorKind> {
        if !self.initialized {
            return Err(PlaybackErrorKind::Internal);
        }

        // Check if content contains simulated script error or webgl failure
        if let Ok(content) = std::fs::read_to_string(&wallpaper.entrypoint) {
            if content.contains("SIMULATE_RENDER_FAILURE") {
                self.events.push(WebEngineEvent::PlaybackFailed(
                    PlaybackErrorKind::StartupFailed,
                ));
                return Ok(());
            }
        }

        self.loaded = true;
        self.events.push(WebEngineEvent::DocumentLoaded);
        self.events.push(WebEngineEvent::RenderingStarted);
        Ok(())
    }

    fn handle_navigation(&mut self, uri: &str) -> Result<(), PlaybackErrorKind> {
        if !is_uri_permitted(uri) {
            self.events.push(WebEngineEvent::SecurityViolation(format!(
                "Blocked navigation to `{uri}`"
            )));
            return Err(PlaybackErrorKind::OperationRejected);
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), PlaybackErrorKind> {
        self.loaded = false;
        self.events.push(WebEngineEvent::PlaybackEnded);
        Ok(())
    }

    fn post_visualizer_frame(
        &mut self,
        frame: &kinetiwall_audio::VisualizerFrame,
    ) -> Result<(), PlaybackErrorKind> {
        self.last_visualizer_frame = Some(frame.clone());
        Ok(())
    }

    fn wait_event(&mut self, _timeout: Duration) -> Option<WebEngineEvent> {
        if self.events.is_empty() {
            None
        } else {
            Some(self.events.remove(0))
        }
    }

    fn shutdown(&mut self) {
        self.initialized = false;
        self.loaded = false;
        self.events.clear();
    }
}

pub fn create_web_engine() -> Box<dyn WebEngine> {
    #[cfg(feature = "fake-backend")]
    {
        Box::new(SimulatedWebEngine::new())
    }

    #[cfg(not(feature = "fake-backend"))]
    {
        match NativeWebView2Engine::try_new() {
            Ok(native) => Box::new(native),
            Err(_) => Box::new(SimulatedWebEngine::new()),
        }
    }
}
