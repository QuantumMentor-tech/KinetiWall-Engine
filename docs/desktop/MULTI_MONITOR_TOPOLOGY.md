# Multi-Monitor Enumeration, Stable Display Identity & Display Topology Modeling (Step 42)

## 1. Overview & Architectural Role

Step 42 establishes the multi-monitor discovery and topology modeling subsystem within `crates/kinetiwall-desktop`. While Steps 39–41 focused on discovering Windows desktop hosts (`WorkerW`/`Progman`), attaching dedicated player surfaces, and synchronizing surface geometry on a single target display, Step 42 provides the foundational inventory, geometry modeling, and identity resolution required for multi-display environments.

The subsystem operates under strict architectural principles:
1. **Purely Observational**: Read-only queries only. Never changes display resolution, refresh rate, orientation, scaling, primary-display selection, or desktop registry configurations.
2. **Handle Transience vs. Stable Identity**: Native Win32 `HMONITOR` handles are transient and valid only within the current snapshot generation. Permanent or cross-session display identity is provided by stable, deterministic `DisplayId` strings derived from hardware PnP and OS device strings.
3. **Geometry Integrity & Negative Coordinates**: In heterogeneous and multi-monitor setups, monitors often possess negative coordinates in virtual screen space (e.g. secondary displays positioned to the left of or above the primary monitor). These values are preserved faithfully without coordinate truncation.
4. **Coherent, Versioned Snapshots**: A `DisplayTopologySnapshot` captures an atomic point-in-time state of all active displays, primary status, virtual desktop bounding bounds, and topology classification, versioned with an incrementing generation counter.
5. **Change Detection & Anti-Thrashing**: Changes are calculated as a `DisplayChangeDelta` distinguishing meaningful modifications (displays added/removed/resized, primary change) from identical notifications.
6. **Separation from Wallpaper Placement**: This subsystem only observes and models displays. It does not assign wallpapers, spawn player windows, or execute multi-monitor placement (which belongs to subsequent roadmap steps).

---

## 2. Core Concepts & Taxonomy

| Concept | Representation | Description |
|---|---|---|
| **Logical Monitor** | `NativeMonitorHandle` (`HMONITOR`) | An OS-exposed desktop rendering target. Transient and valid only for the active snapshot. |
| **Stable Display Identity** | `DisplayId` | Deterministic, application-level identifier (e.g. `disp_DISPLAY1_MONITOR_DEL4182_...`) resilient to handle churn. |
| **Display Geometry** | `DisplayGeometry` | Full monitor bounds (`monitor_rect`) and usable work area (`work_area_rect`) in virtual screen coordinates. |
| **Display Descriptor** | `DisplayDescriptor` | Complete validated descriptor uniting stable ID, native handle, device name, friendly name, hardware ID, and geometry. |
| **Topology Type** | `DisplayTopologyType` | Semantic classification: `SingleDisplay`, `ExtendedDesktop`, `CloneOrMirrored`, `ComplexOrMixed`, `UnknownOrUnsupported`. |
| **Topology Snapshot** | `DisplayTopologySnapshot` | Immutable, generation-versioned capture of the complete display environment. |
| **Change Delta** | `DisplayChangeDelta` | Explicit delta tracking added/removed/modified displays and primary/geometry/topology transitions. |

---

## 3. Windows Native APIs Used

All native queries are performed safely through `NativeDesktopDisplayProvider`:

1. **`EnumDisplayMonitors` (`user32.dll`)**:
   - Enumerates all logical desktop monitors intersecting the virtual screen.
   - Collects transient `HMONITOR` handles.
2. **`GetMonitorInfoW` (`user32.dll`)**:
   - Populates `MONITORINFOEXW` (`cbSize = 104`).
   - Retrieves full virtual-screen rectangle (`rcMonitor`), work-area rectangle (`rcWork`), primary flag (`MONITORINFOF_PRIMARY`), and device name string (`szDevice`, e.g. `\\.\DISPLAY1`).
3. **`EnumDisplayDevicesW` (`user32.dll`)**:
   - Queries `DISPLAY_DEVICEW` (`cb = 840`) for the adapter and attached monitor.
   - Extracts friendly name (`DeviceString`), hardware PnP ID (`DeviceID`), and state flags (`DISPLAY_DEVICE_ATTACHED_TO_DESKTOP`, `DISPLAY_DEVICE_MIRRORING_DRIVER`).
4. **`QueryDisplayConfig` (`user32.dll`)**:
   - Queries `QDC_ONLY_ACTIVE_PATHS` with bounded allocations.
   - Identifies active source/target paths to detect hardware-level cloned or mirrored configurations.
   - Falls back gracefully to coordinate-based geometry analysis when unavailable (e.g. in remote desktop sessions or virtualized test harnesses).

---

## 4. Stable Display Identity Strategy

Native handles like `HMONITOR(0x00010001)` change upon resolution change, driver reset, monitor reconnection, or application restart. Treating them as keys causes configuration loss and incorrect surface attachment.

`DisplaySnapshotManager` resolves identity through deterministic matching:
1. **Device Name + Hardware PnP ID**: When available (e.g. `\\.\DISPLAY1` with `MONITOR\DEL4182\1`), an application-level key `disp_DISPLAY1_MONITOR_DEL4182_1` is generated.
2. **Duplicate Device Disambiguation**: Two identical monitors with identical hardware strings are separated deterministically by device name and connection index.
3. **Historical Snapshot Correlation**: When `refresh()` is called, known display identities are matched against previous snapshots, guaranteeing that an attached or tracked monitor retains its exact `DisplayId` across display-change events.

---

## 5. Geometry Semantics & Negative Coordinates

The Windows desktop coordinate space places `(0, 0)` at the top-left of the **primary** monitor.
- If a secondary 1920x1080 display is arranged to the **left** of the primary display, its bounds are `left: -1920, top: 0, right: 0, bottom: 1080`.
- If a secondary display is arranged **above** the primary, its bounds have negative `top` and `bottom`.
- The `virtual_desktop_bounds` is the minimal bounding rectangle enclosing all active monitors:
  ```rust
  virtual_bounds = virtual_bounds.union(&monitor.monitor_rect);
  ```
- Geometry validation enforces:
  - `width > 0 && height > 0`
  - `work_area_rect` is geometrically contained within `monitor_rect`
  - Zero-width, zero-height, or inverted rectangles are rejected with `DisplayDiscoveryError::InvalidGeometry`.

---

## 6. Topology Classification

`DisplaySnapshotManager::classify_topology` classifies the active layout:
- **`SingleDisplay`**: Exactly 1 active monitor.
- **`CloneOrMirrored`**: Multiple monitors sharing identical coordinate rectangles or confirmed by `QueryDisplayConfig` path sources.
- **`ExtendedDesktop`**: Two or more monitors with non-overlapping bounds in virtual desktop space.
- **`ComplexOrMixed`**: Mixed overlapping or irregular multi-monitor configurations.
- **`UnknownOrUnsupported`**: Zero displays or unresolvable native state.

---

## 7. Verification & Test Evidence

- **Unit Tests (`display_tests.rs`)**: 10 tests passing covering single monitor, dual extended layout with negative coordinates, cloned/mirrored topology, stable identity preservation across refresh, change delta detection, heterogeneous resolutions (4K + 1440p), zero-geometry rejection, and empty inventory rejection.
- **Native Integration Tests (`native_integration_tests.rs`)**: `test_native_display_discovery_and_topology` live test passing against actual Windows operating system, enumerating active displays, validating primary resolution, confirming virtual desktop bounds, and verifying identity preservation across consecutive refreshes.
- **Total Test Suite**: 169 tests passing across all workspace crates with zero regressions.

