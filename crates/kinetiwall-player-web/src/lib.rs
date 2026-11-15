//! # `kinetiwall-player-web`
//!
//! Isolated interactive web wallpaper player process host (WebView2) and controller-side
//! `PlayerAdapter` for it.

pub mod adapter;
pub mod engine;
pub mod protocol;
pub mod runtime;
pub mod sampler;
pub mod validate;

pub use adapter::WebPlayerAdapter;
pub use engine::{create_web_engine, WebEngine, WebEngineEvent};
pub use protocol::{FromHost, HostOp, ToHost, MAX_LINE_BYTES, PROTOCOL_VERSION};
pub use runtime::{detect_webview2_runtime, WebView2RuntimeInfo};
pub use validate::{is_uri_permitted, validate_web_wallpaper_path, ValidatedWebWallpaper};
