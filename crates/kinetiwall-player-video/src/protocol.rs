//! Controller ⇄ player-host wire protocol: one JSON object per line over the child's stdin/stdout
//! (an anonymous pipe owned by the spawning adapter; no server, port, or named pipe is created).
//!
//! Hard limits: a line is at most [`MAX_LINE_BYTES`]. The host never receives free-form command
//! strings: only the typed messages below, and the media path is the only caller-controlled string.

use kinetiwall_playback::{PlaybackErrorKind, PlaybackSessionId, PlayerReport};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_LINE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum ToHost {
    /// Create and initialize the native player. Ack `ok` guarantees an initialized context.
    Prepare { session_id: PlaybackSessionId },
    /// Request loading of a local media file. Ack `ok` means *load requested/accepted only*.
    Start {
        session_id: PlaybackSessionId,
        media_path: String,
    },
    /// Stop playback of the session. Idempotent.
    Stop { session_id: PlaybackSessionId },
    /// Dynamically adjust volume.
    SetVolume {
        session_id: PlaybackSessionId,
        volume: f32,
    },
    /// Dynamically adjust mute state.
    SetMute {
        session_id: PlaybackSessionId,
        muted: bool,
    },
    /// Dynamically adjust pause state.
    SetPause {
        session_id: PlaybackSessionId,
        paused: bool,
    },
    /// Release everything and exit.
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
    /// Authoritative backend-observed state (never emitted for a mere request).
    Report {
        session_id: PlaybackSessionId,
        report: PlayerReport,
    },
    /// Evidence of the decoding mode actually reported by libmpv (`hwdec-current`).
    HwDec {
        session_id: PlaybackSessionId,
        mode: String,
    },
}
