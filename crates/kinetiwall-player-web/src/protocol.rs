//! Controller ⇄ web player host wire protocol: one JSON object per line over child stdin/stdout.

use kinetiwall_playback::{PlaybackErrorKind, PlaybackSessionId, PlayerReport};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_LINE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum ToHost {
    /// Initialize WebView2 environment and runtime.
    Prepare { session_id: PlaybackSessionId },
    /// Load validated local web wallpaper entrypoint.
    Start {
        session_id: PlaybackSessionId,
        entrypoint_path: String,
    },
    /// Stop playback and navigate away to blank.
    Stop { session_id: PlaybackSessionId },
    /// Bounded audio visualization update frame.
    VisualizerUpdate {
        session_id: PlaybackSessionId,
        frame: kinetiwall_audio::VisualizerFrame,
    },
    /// Teardown WebView2 controller, environment, and exit.
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostOp {
    Prepare,
    Start,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "msg", rename_all = "snake_case")]
pub enum FromHost {
    Ack {
        session_id: PlaybackSessionId,
        op: HostOp,
        ok: bool,
        error: Option<PlaybackErrorKind>,
    },
    Report {
        session_id: PlaybackSessionId,
        report: PlayerReport,
    },
    SecurityNotice {
        session_id: PlaybackSessionId,
        detail: String,
    },
}
