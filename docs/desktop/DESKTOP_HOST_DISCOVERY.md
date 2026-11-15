# Windows Desktop Host Discovery, Shell Topology Detection & Compatibility Layer

**Module:** `crates/kinetiwall-desktop`  
**Phase:** Phase 7: Native Windows Desktop Hosting & Wallpaper Surface Integration  
**Step:** 39 of 90  
**Implementation Standard:** Safe Win32 FFI, Process-Isolated, Strictly Validated, Rust-First

---

## 1. Architectural Overview

The Windows desktop shell is not a simple drawing canvas; it is a complex, evolving hierarchy of Win32 windows managed by Windows Explorer (`explorer.exe`). In order to host live wallpapers behind desktop icons without destabilizing the shell or interfering with user input, KinetiWall implements a dedicated, robust desktop host discovery layer in `crates/kinetiwall-desktop`.

```
                       +----------------------------------+
                       |       KinetiWall Core            |
                       +-----------------+----------------+
                                         |
                                         v
                       +----------------------------------+
                       |      DesktopHostDiscovery        |
                       +-----------------+----------------+
                                         |
                                         v
                       +----------------------------------+
                       |      DesktopWindowProvider       |
                       |  (Trait: Native Win32 or Mock)   |
                       +-----------------+----------------+
                                         |
               +-------------------------+-------------------------+
               |                                                   |
               v                                                   v
   +-----------------------+                           +-----------------------+
   | NativeDesktopProvider |                           |  MockDesktopProvider  |
   | (Win32 FFI user32.dll)|                           |  (Deterministic Tests)|
   +-----------------------+                           +-----------------------+
```

### Key Separation of Concerns
1. **`kinetiwall-desktop::types`**: Strongly typed domain models including `NativeWindowHandle`, `WindowRect`, `CandidateRole`, `DesktopCandidateWindow`, `DesktopShellTopology`, and `TopologyClassification`.
2. **`kinetiwall-desktop::provider`**: Abstracted window discovery provider trait (`DesktopWindowProvider`). It cleanly isolates all `unsafe` Win32 API interactions inside `NativeDesktopWindowProvider` and provides an in-memory `MockDesktopWindowProvider` for deterministic testing.
3. **`kinetiwall-desktop::discovery`**: Multi-phase discovery algorithm implementing safe window traversal, icon carrier identification, Z-order candidate ordering, and bounded active probe dispatch.
4. **`kinetiwall-desktop::compatibility`**: Strict validation rules (`CompatibilityPolicy`) evaluating whether a classified topology and candidate window are suitable for future surface attachment review.

---

## 2. Win32 Window Hierarchy & Shell Topologies

Windows Explorer hosts the desktop icons inside a `SysListView32` control parented by a `SHELLDLL_DefView` window. Depending on the Windows version, system state, and whether a worker split has occurred, this hierarchy manifests in distinct topologies:

### 2.1 Classic Unsplit Desktop (`ClassicUnsplit`)
In a freshly booted system before any split message has been received by `Progman`:
```
Progman (Program Manager)
 └── SHELLDLL_DefView
      └── SysListView32 (Desktop Icons)
```
- **State**: The icon view is directly parented to `Progman`.
- **Wallpaper Safety**: Unsafe for direct wallpaper hosting. Reparenting a wallpaper window into `Progman` would either occlude desktop icons or be occluded by `SHELLDLL_DefView`.
- **Policy Evaluation**: Rejected by `CompatibilityPolicy` (`UnsplitTopologyRequiresSplit`). Must transition via active probe before attachment.

### 2.2 Standard WorkerW Split (`StandardWorkerWSplit`)
When `Progman` receives message `0x052C`, Windows Explorer creates or repurposes two `WorkerW` top-level windows:
```
WorkerW (Icon Carrier)
 └── SHELLDLL_DefView
      └── SysListView32 (Desktop Icons)

WorkerW (Wallpaper Host Candidate)   <-- Safe wallpaper target
 (Sibling positioned directly behind the Icon Carrier WorkerW)

Progman (Root)
```
- **State**: `SHELLDLL_DefView` has been reparented into an icon-carrier `WorkerW`. A sibling `WorkerW` is spawned immediately beneath it in the Z-order.
- **Wallpaper Safety**: Safe for attachment review. The sibling `WorkerW` sits behind the desktop icons and in front of the desktop background wallpaper.
- **Policy Evaluation**: Approved by `CompatibilityPolicy`.

### 2.3 Modern Multi-WorkerW Topology (`ModernMultiWorkerW`)
On modern Windows 10 and Windows 11 builds (e.g., multi-monitor virtual desktop configurations or Windows Feature Experience Pack updates), multiple `WorkerW` windows may exist:
```
WorkerW (Icon Carrier)
 └── SHELLDLL_DefView
      └── SysListView32

WorkerW (Primary Wallpaper Host)     <-- Selected candidate (closest sibling)

WorkerW (Secondary / Monitor 2)      <-- Tracked candidate

Progman
```
- **State**: Multiple `WorkerW` windows exist.
- **Resolution**: `DesktopHostDiscovery` inspects Z-order directly following the icon carrier, verifying process ID matches Explorer and dimensions span valid desktop coordinates.
- **Policy Evaluation**: Approved by `CompatibilityPolicy`.

### 2.4 Ambiguous or Degraded Topologies
- **`Ambiguous`**: Multiple unparented candidates exist with identical Z-order proximity or differing ownership.
- **`DesktopRootUnavailable`**: Non-interactive desktop station, locked service account, or Explorer shell is not running.
- **`ShellUnavailable`**: `SHELLDLL_DefView` cannot be located anywhere in the window station.
- **`UnknownTopology`**: Unexpected window configuration detected.
- **Safety Invariant**: All degraded topologies are rejected by `CompatibilityPolicy`. KinetiWall never guesses or forces attachment.

---

## 3. Passive Read-Only Discovery vs. Bounded Active Probe

A fundamental safety requirement of KinetiWall is strictly separating passive inspection from active shell state modification:

### 3.1 Passive Discovery (Default)
- **Flag**: `DesktopDiscoveryOptions { allow_active_probe: false, .. }`
- **Behavior**: Uses purely read-only Win32 APIs (`FindWindowW`, `FindWindowExW`, `EnumWindows`, `GetClassNameW`, `GetWindowTextW`, `GetWindowRect`, `IsWindowVisible`).
- **Guarantee**: Zero messages are posted or sent to Explorer. Desktop state is left completely untouched.
- **Result**: Accurately classifies whether the shell is already split (`StandardWorkerWSplit`) or still in classic form (`ClassicUnsplit`).

### 3.2 Bounded Active Probe
- **Flag**: `DesktopDiscoveryOptions { allow_active_probe: true, probe_timeout_ms: 1000, probe_message_id: 0x052C, .. }`
- **Behavior**: When passive discovery detects `ClassicUnsplit`, and active probe is explicitly allowed, `DesktopHostDiscovery` sends:
  ```rust
  provider.send_message_timeout(
      progman_handle,
      0x052C,
      0x00000000,
      0x00000000,
      SMTO_NORMAL | SMTO_ABORTIFHUNG,
      probe_timeout_ms,
  )
  ```
- **Safety Safeguards**:
  1. Uses `SendMessageTimeoutW` with `SMTO_ABORTIFHUNG` to prevent blocking the calling thread if Explorer is unresponsive.
  2. Bounded by a configurable timeout (default 1000ms).
  3. Immediately re-enumerates top-level windows to verify if the WorkerW split successfully occurred.
  4. If the probe times out or Explorer fails to split, discovery safely falls back to `ClassicUnsplit` without crashing.

---

## 4. Candidate Validation and Verification Rules

Before any candidate window is recommended or accepted for attachment, it must satisfy all verification gates defined in `CompatibilityPolicy`:

1. **Handle Liveness**: Must pass `provider.is_window(hwnd)`.
2. **Class Name Match**: Window class must be exactly `"WorkerW"` (case-insensitive) and not `"Progman"` or `"SHELLDLL_DefView"`.
3. **Role Differentiation**: Candidate role must be `CandidateRole::WallpaperHostCandidate`. If the candidate contains `SHELLDLL_DefView` (i.e. it is the `IconHostCarrier`), it is **strictly rejected** to prevent desktop icons from being destroyed or hidden.
4. **Process Ownership**: Process ID must be non-zero and match the process hosting the shell (`explorer.exe`).
5. **Thread ID**: Thread ID must be non-zero.
6. **Visibility**: Candidate window must be visible (`IsWindowVisible`).
7. **Spatial Bounds**: Bounding rect (`GetWindowRect`) must have non-zero width and height (`width > 0 && height > 0`).
8. **Stale Handle Protection**: `DesktopHostDiscovery::revalidate_candidate` verifies that the HWND is still valid, still has class `"WorkerW"`, and still belongs to the expected PID immediately prior to any subsequent operation.

---

## 5. Error Taxonomy

The module provides typed error handling via `DiscoveryError`:
- `InvalidHandle(NativeWindowHandle)`: Handle does not exist or failed `IsWindow`.
- `WindowNotFound(String)`: Named window or class could not be found.
- `Win32Error(String)`: Win32 API returned an OS error code or failed.
- `Timeout`: Window message timed out (e.g. `SendMessageTimeoutW`).
- `ExplorerHung`: Explorer was detected as hung during message dispatch.
- `AmbiguousHierarchy(String)`: Conflicting or unresolvable window layout.
- `TopologyNotSupported(String)`: Observed topology cannot host wallpapers.
- `AccessDenied(String)`: Window station or security boundary denied access.

---

## 6. Testing Strategy

The crate features comprehensive automated tests with zero mock leaks:
- **Synthetic Unit Tests (`tests/discovery_tests.rs`)**:
  - `test_topology_classic_unsplit_passive_discovery`: Validates classic unsplit detection without active probe.
  - `test_topology_active_probe_split_transition`: Tests active probe triggering mock WorkerW creation and split transition.
  - `test_topology_standard_workerw_split`: Tests standard split resolution and candidate selection.
  - `test_topology_modern_multi_workerw_resolution`: Tests multi-WorkerW disambiguation based on Z-order.
  - `test_topology_ambiguous_workerw_candidates`: Validates ambiguous candidate isolation and rejection.
  - `test_topology_desktop_root_unavailable`: Tests graceful error handling when Explorer/Progman is missing.
  - `test_candidate_revalidation_and_stale_handle_detection`: Validates stale handle and mutated class rejection.
  - `test_compatibility_policy_rejects_icon_carrier_as_host`: Verifies that icon-carrier WorkerW is never recommended as a host.
- **Live Windows Integration Tests (`tests/native_integration_tests.rs`)**:
  - `test_native_desktop_passive_discovery`: Runs real Win32 discovery against the host OS desktop without altering window styles.
  - `test_native_desktop_window_enumeration_validity`: Validates real HWND enumeration, PID/TID extraction, and handle validity.
  - `test_native_candidate_revalidation_real_windows`: Tests real handle revalidation and bogus handle rejection.
  - `test_native_compatibility_policy_truthful_evaluation`: Evaluates live host topology with zero assumptions or forced bypasses.

---

## 7. Guidelines for Phase 7 Step Progression

1. **Step 39 (This Step)**: Discovery and topology classification only. No reparenting, no live attachments, no icon manipulation.
2. **Step 40 (Wallpaper Surface Attachment & Detachment)**: Will consume `DesktopCandidateWindow` validated by `CompatibilityPolicy`, applying safe Win32 parenting, message filtering, and clean detachment guards.
3. **Step 41 (Multi-Monitor Topology & Coordinate Mapping)**: Will extend candidate coordinate bounds across multi-monitor virtual desktop surfaces (`EnumDisplayMonitors`).

