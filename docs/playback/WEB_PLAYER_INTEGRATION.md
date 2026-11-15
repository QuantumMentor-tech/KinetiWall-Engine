# KinetiWall Interactive Web Wallpaper Player & WebView2 Isolation Architecture (`Step 33`)

Crate: [`crates/kinetiwall-player-web`](../../crates/kinetiwall-player-web)  
Security Specification: [`docs/security_and_ipc/WEBVIEW2_SECURITY_SPECIFICATION.md`](../security_and_ipc/WEBVIEW2_SECURITY_SPECIFICATION.md)

## 1. Process Architecture & Sandboxing Boundaries
In alignment with KinetiWall's process isolation architecture (`ADR-0006`, `CTR-02`, `CTR-05`, `CTR-06`, `CTR-07`) and the WebView2 Security Specification:
- **Core Controller / Resident Daemon**: Runs the `PlaybackOrchestrator` and manages instances of `WebPlayerAdapter`.
- **Isolated Web Player Process (`kinetiwall-player-web.exe`)**:
  - Independent child process spawned with dedicated anonymous standard I/O pipes (`stdin`/`stdout`).
  - No shell string execution: strictly typed line-delimited JSON messages defined in [`protocol.rs`](../../crates/kinetiwall-player-web/src/protocol.rs) (`MAX_LINE_BYTES = 64KB`).
  - HTML5, Canvas, WebGL, CSS, and JS execution occur strictly inside this isolated child process and WebView2's sandboxed rendering processes.
  - A crash, memory leak, or infinite JavaScript loop does **not** crash the KinetiWall core controller or UI shell.
- **Desktop Separation**: No desktop icon parenting (`WorkerW`/`Progman`) or input hooks are performed in this step. Standalone rendering and surface initialization are decoupled from desktop attachment.

```text
Controller Process (PlaybackOrchestrator)
        │
        ▼ (WebPlayerAdapter)
   [stdin pipe]   Line-delimited JSON (ToHost::Prepare, ToHost::Start, ToHost::Stop, ToHost::Shutdown)
        │
        ▼
Isolated Process (`kinetiwall-player-web.exe`)
   ├── Path & Containment Validation (validate_web_wallpaper_path: root directory boundary)
   ├── WebView2 Dynamic Runtime Detection (Microsoft Edge WebView2 Evergreen)
   ├── Isolated User Data Folder (%LOCALAPPDATA%\KinetiWall\WebView2_Data)
   ├── Virtual Host Mapping (`https://kinetiwall.local/` <-> root asset directory)
   ├── Navigation Lockdown & Boundary Enforcement (`is_uri_permitted`)
   └── Engine Lifecycle & Event Loop
        │
   [stdout pipe]  Line-delimited JSON (FromHost::Ack, FromHost::Report, FromHost::SecurityNotice)
        │
        ▼
Controller Process (State confirmed only upon authoritative FromHost::Report events)
```

## 2. WebView2 Security Controls & Sandbox Lockdown
To ensure that local interactive web wallpapers cannot compromise user files or the host system:
1. **Virtual Host Name Mapping**:
   - Web assets are served exclusively via virtual host name mapping: `SetVirtualHostNameToFolderMapping("kinetiwall.local", asset_root_dir, COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_DENY)`.
   - Access via `file:///` protocols is strictly forbidden to prevent path traversal outside the wallpaper package.
2. **Navigation Lockdown**:
   - `NavigationStarting` and `FrameNavigationStarting` event handlers intercept all navigation requests.
   - Any URI outside `https://kinetiwall.local/*` or `about:blank` is denied (`Cancel = true`) and reported to the host via `FromHost::SecurityNotice`.
   - Popups and new windows are blocked via `NewWindowRequested` (`Handled = true`).
3. **Privilege Boundary & Bridge Omission**:
   - **No Tauri Bridges**: Neither Tauri internal APIs nor native host bridges (`postMessage` / `AddWebMessageCallback` / COM bridges) are exposed to the web wallpaper DOM.
   - Web content executes as unprivileged client-side code without filesystem, network, or OS control.
4. **Isolated User Data Folder**:
   - Dedicated data directory under `%LOCALAPPDATA%\KinetiWall\WebView2_Data`.
   - Separate from the main application shell and other browser sessions, preventing cross-origin data leakage.
5. **Runtime Detection**:
   - Dynamic detection via `detect_webview2_runtime()` inspecting standard system paths (`Program Files` / `Program Files (x86)`).
   - If WebView2 is absent, `WebPlayerAdapter::is_available()` cleanly reports `false` without crashing.

## 3. Playback Lifecycle & State Mapping (Step 31 Conformance)
- `adapter.capabilities()`: Returns `supports_type = WallpaperType::Web`, `supports_pause = false`, `supports_seamless_loop = true`.
- `adapter.prepare()`: Launches the `kinetiwall-player-web.exe` child process if not running, sends `ToHost::Prepare`.
- `adapter.start()`: Validates asset path, computes the root directory containment boundary, sends `ToHost::Start`.
  - Acceptance is distinguished from actual playback: `start()` ack confirms command receipt, while `PlayerReport::Playing` is only emitted when the engine completes DOM/content load.
- `adapter.stop()`: Issues `ToHost::Stop` and returns `StopAck::Stopped`.
- `adapter.cancel()`: Issues `ToHost::Stop` and returns `CancelAck::Accepted`.
- `adapter.shutdown()`: Issues `ToHost::Shutdown` and terminates the child process and I/O pump cleanly.

## 4. Verification and Deterministic Testing
- Unit & integration tests in [`crates/kinetiwall-player-web/tests/player_web_tests.rs`](../../crates/kinetiwall-player-web/tests/player_web_tests.rs):
  - `test_security_uri_permission_boundaries`: Verifies whitelist filtering (`https://kinetiwall.local/` allowed; internet, `file:///`, and `javascript:` schemes blocked).
  - `test_web_player_adapter_capabilities_and_availability`: Validates adapter contracts and runtime availability check.
  - `test_web_path_validation_and_containment`: Verifies asset validation, containment directory computation, and malformed path rejection.
  - `test_web_player_adapter_cancel_flow`: Verifies graceful transition during cancellation.
  - `test_web_player_adapter_missing_asset_fails_cleanly`: Confirms non-existent asset files trigger immediate, clean failure without hung processes.
  - `test_web_player_adapter_full_lifecycle_with_isolated_process`: Spawns real `kinetiwall-player-web.exe` process over anonymous pipes, verifies handshake, prepare, start, playing event reception, stop, and clean shutdown.
- Tested and verified on Windows 10/11 (`x86_64-pc-windows-msvc`).

