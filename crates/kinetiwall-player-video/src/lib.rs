//! # `kinetiwall-player-video`
//!
//! Isolated video player process host (runtime-loaded libmpv) and the controller-side
//! `PlayerAdapter` for it.

pub mod adapter;
pub mod engine;
pub mod ffi;
pub mod protocol;
pub mod sampler;
pub mod validate;

pub use adapter::VideoPlayerAdapter;
pub use engine::{create_video_engine, EngineEvent, VideoEngine};
pub use protocol::{FromHost, HostOp, ToHost, MAX_LINE_BYTES, PROTOCOL_VERSION};
pub use validate::validate_media_path;
