# KinetiWall Video Player & libmpv Integration Architecture (`Step 32`)

Crate: [`crates/kinetiwall-player-video`](../../crates/kinetiwall-player-video)

## 1. Process Architecture & Boundaries
In alignment with KinetiWall's process isolation architecture (`ADR-0006`, `CTR-02`, `CTR-05`, `CTR-06`, `CTR-07`):
- **Core Controller / Resident Daemon**: Runs the `PlaybackOrchestrator` and holds instances of `VideoPlayerAdapter`.
- **Isolated Video Player Process (`kinetiwall-player-video.exe`)**:
  - Independent child process spawned with dedicated anonymous pipes (`stdin`/`stdout`).
  - No shell string execution: strictly line-delimited typed JSON messages defined in [`protocol.rs`](../../crates/kinetiwall-player-video/src/protocol.rs) (`MAX_LINE_BYTES = 64KB`).
  - Media decoding, Direct3D/D3D11VA presentation, and libmpv state management occur strictly inside this isolated process.
  - A crash or hang in libmpv/video decoder does **not** crash the KinetiWall core controller or UI.
- **Desktop Separation**: No desktop icon parenting (`WorkerW`/`Progman`) is performed in this step. Standalone decoding and ordinary surface rendering are decoupled from desktop attachment.

```text
Controller Process (PlaybackOrchestrator)
        │
        ▼ (VideoPlayerAdapter)
   [stdin pipe]   Line-delimited JSON (ToHost::Prepare, ToHost::Start, ToHost::Stop, ToHost::Shutdown)
        │
        ▼
Isolated Process (`kinetiwall-player-video.exe`)
   ├── Path Validation (validate_media_path: strict allowlist & regular file checks)
   ├── Dynamic libmpv FFI (`mpv-2.dll` / `libmpv-2.dll` via libloading)
   └── Engine loop (events & hardware acceleration detection)
        │
   [stdout pipe]  Line-delimited JSON (FromHost::Ack, FromHost::Report, FromHost::HwDec)
        │
        ▼
Controller Process (State confirmed only upon authoritative FromHost::Report events)
```

## 2. Dynamic libmpv Linkage (`ADR-0006`, `DEP-08`)
- **Linkage Model**: Pure dynamic runtime loading via `libloading` (`MpvFunctions`).
  - No static linkage to GPL/LGPL code.
  - Runtime dynamically looks up `mpv-2.dll`, `libmpv-2.dll`, `mpv-1.dll`, or `libmpv-1.dll` from PATH, current directory, or configured runtime directory.
  - Enables user replacement of the dynamic library without rebuilding the application, ensuring full **LGPL-2.1-or-later** compliance.
- **Hardware Acceleration**: Configured with `hwdec = "auto-safe"` (Direct3D 11 / D3D11VA). Emits authoritative `FromHost::HwDec { mode }` event based on `hwdec-current`.
- **Audio Policy**: Initialized with `mute = "yes"` and `loop-file = "inf"` matching live wallpaper operational constraints.

## 3. Playback Lifecycle & State Mapping (Step 31 Conformance)
- `adapter.prepare()`: Spawns the player process if not running, issues `ToHost::Prepare`.
- `adapter.start()`: Validates that asset reference exists in catalog lookup, sends `ToHost::Start`.
  - Acceptance is distinguished from actual playback: `start()` ack confirms file loading was accepted, while `PlayerReport::Playing` is only emitted when libmpv fires `MPV_EVENT_PLAYBACK_RESTART`.
- `adapter.stop()`: Issues `ToHost::Stop` and returns `StopAck::Stopped`.
- `adapter.cancel()`: Issues `ToHost::Stop` and returns `CancelAck::Accepted`.
- `adapter.shutdown()`: Issues `ToHost::Shutdown` and terminates child process and reader threads cleanly.

## 4. Verification and Deterministic Testing
- Unit & integration tests in `crates/kinetiwall-player-video/tests/player_video_tests.rs`:
  - Path safety & boundary enforcement (null bytes, invalid extensions, missing files).
  - Adapter capability and availability assertions.
  - Full end-to-end multi-process lifecycle (prepare -> start -> event polling -> confirmed playing -> stop -> shutdown).
  - Missing asset clean failure.
  - Cancellation flow.
- Verified on Windows `x86_64-pc-windows-msvc`.

