# Desktop Surface Synchronization, Display-Change Handling & Safe Reattachment (Step 41)

## 1. Overview & Architecture

Step 41 establishes the dynamic synchronization layer for KinetiWall desktop wallpaper surfaces. Once a dedicated player surface window is attached to a desktop host window (such as a Windows `WorkerW` background layer behind `SHELLDLL_DefView` desktop icons), dynamic runtime changes—such as display resolution changes, per-monitor DPI transitions, parent host movement, or unexpected desktop host destruction—must be handled continuously and safely.

The synchronization system guarantees:
1. **Geometry Alignment**: The surface is kept matching the active target host bounds without gaps, black borders, or coordinate drift.
2. **Focus & Z-Order Preservation**: Positioning calls use `SWP_NOACTIVATE | SWP_NOZORDER | SWP_SHOWWINDOW` to guarantee that the wallpaper never steals focus from active applications or moves ahead of desktop icons.
3. **Anti-Thrashing & Coalescing**: Consecutive or identical display notifications are coalesced into a single no-op check, avoiding flickering and unnecessary Desktop Window Manager (DWM) composition cycles.
4. **Self-Generated Event Loop Guard**: An `is_synchronizing` re-entrancy guard ensures that native `WM_WINDOWPOSCHANGED` messages triggered by KinetiWall do not cause an infinite recursive resizing storm.
5. **Safe Host Recovery & Rediscovery**: If the parent `WorkerW` window disappears (e.g. during an Explorer shell refresh or Windows theme change), KinetiWall unparents the surface cleanly, performs fresh discovery and compatibility evaluation, and reattaches the surface with a strictly bounded retry budget (maximum 3 attempts).

---

## 2. Synchronization State Machine

The operational lifecycle of an attached wallpaper surface is tracked explicitly by `SurfaceSyncState`:

```
                +---------------------+
                |  NoSurfaceAttached  |
                +----------+----------+
                           | attach()
                           v
                +---------------------+
  +------------>|    Synchronized     |<------------+
  |             +----------+----------+             |
  |                        |                        |
  |             Event: New Bounds / Res             |
  |                        v                        |
  |             +---------------------+             |
  |             |    Synchronizing    |-------------+ (Success)
  |             +----------+----------+
  |                        |
  |                 Host Destroyed
  |                        v
  |             +---------------------+
  |             |   HostUnavailable   |
  |             +----------+----------+
  |                        |
  |                Recovery Attempt (< 3)
  |                        v
  |             +---------------------+
  |             | RecoveryInProgress  |-------------+ (Rediscovered & Reattached)
  |             +----------+----------+
  |                        |
  |                Budget Exhausted (>= 3)
  |                        v
  |             +---------------------+
  |             |SynchronizationFailed|
  |             +---------------------+
  |                        |
  |      detach() / Coordinator Shutdown
  v                        v
+-------------------------------------+
|       DetachedOrShuttingDown        |
+-------------------------------------+
```

- **`NoSurfaceAttached`**: Handle is not currently registered or attached.
- **`Synchronized`**: Verified attached, visible, matching target host bounds with confirmed generation.
- **`GeometryUpdatePending`**: External display change received, queued for synchronization.
- **`Synchronizing`**: Native Win32 repositioning (`SetWindowPos`) and post-verification in progress.
- **`HostValidationPending`**: Revalidation of the parent host handle is scheduled.
- **`HostUnavailable`**: The host window handle is invalid or was destroyed.
- **`RevalidationRequired`**: Invariant violation detected; requires state recheck before applying changes.
- **`RecoveryInProgress`**: Unparenting source window, running Step 39 discovery, and re-attaching via Step 40.
- **`SynchronizationFailed`**: Recovery attempts exhausted or non-recoverable error encountered.
- **`DetachedOrShuttingDown`**: Clean detachment completed or coordinator shutting down.

---

## 3. Surface Change Events (`SurfaceChangeEvent`)

External Windows messages and events mapped to synchronization triggers:

| Event Variant | Triggering Windows Event | Action Taken |
|---|---|---|
| `DisplayResolutionChanged { width, height, bits_per_pixel }` | `WM_DISPLAYCHANGE` | Computes target rect `(0, 0, width, height)` and resizes surface. |
| `DpiChanged { new_dpi, suggested_rect }` | `WM_DPICHANGED` | Uses `suggested_rect` or queries host window dimensions, then resizes. |
| `HostGeometryChanged { host_handle, new_bounds }` | `WM_WINDOWPOSCHANGED` (on host) | Validates host handle and resizes surface to match host bounds. |
| `HostWindowDestroyed(host_handle)` | `WM_DESTROY` (on host) | Initiates host rediscovery and safe reattachment protocol. |
| `ParentRelationshipLost(host_handle)` | Invariant check `GetParent(source) != host` | Initiates host rediscovery and safe reattachment protocol. |
| `ManualSyncRequested` | Coordinator / Diagnostics API | Re-evaluates host geometry and enforces surface alignment. |

---

## 4. Re-Entrancy & Anti-Thrashing Architecture

### 4.1 Coalescing Identical Notifications
When resolution changes occur, Windows often sends multiple rapid notifications (`WM_DISPLAYCHANGE`, `WM_SETTINGCHANGE`, `WM_WINDOWPOSCHANGED`).
```rust
if meta.last_verified_bounds == Some(target_bounds) && meta.state == SurfaceSyncState::Synchronized {
    return Ok(SurfaceSyncReport {
        source_handle: source,
        host_handle: meta.target_handle,
        player_instance_id: meta.player_instance_id,
        sync_generation: meta.sync_generation,
        previous_bounds: meta.last_verified_bounds,
        effective_bounds: target_bounds,
        mutation_applied: false, // SKIPPED: Zero Win32 mutation
        state: SurfaceSyncState::Synchronized,
        synced_at_millis: now_millis,
    });
}
```
If the requested bounds already match `last_verified_bounds`, the Win32 `SetWindowPos` call is completely skipped, returning `mutation_applied: false`.

### 4.2 Loop Prevention Guard
Calling `SetWindowPos` on a window causes Windows to deliver `WM_WINDOWPOSCHANGED` back to the window or its parent. If unmonitored, an application listening for geometry changes would repeatedly call `SetWindowPos` in an infinite loop.
The `is_synchronizing` flag acts as a critical re-entrancy guard:
```rust
if meta.is_synchronizing {
    tracing::debug!(source = %source, "Re-entrant synchronization event ignored");
    return Ok(SurfaceSyncReport {
        mutation_applied: false,
        ...
    });
}
```

---

## 5. Host Invalidation Recovery & Safe Reattachment

When a desktop host window is destroyed (e.g. Windows Explorer crashes or restarts, or user changes desktop personalization themes):
1. **Unparent Surface**: `SetParent(source, NULL)` is immediately executed to detach the live player window from the dead host, preventing illegal memory dereferences or orphaned native handles.
2. **Preserve Original State**: The true initial window state (styles, ex-styles, original parent) captured during initial attachment is preserved so eventual detachment still restores the original window correctly.
3. **Execute Step 39 Discovery**: `DesktopHostDiscovery::new(provider)` inspects the new desktop window hierarchy and classifies the shell topology.
4. **Evaluate Compatibility Policy**: `CompatibilityPolicy::evaluate_attachment_suitability` verifies that a valid `WallpaperHostCandidate` exists and that desktop icons (`SHELLDLL_DefView`) are not contained within the candidate.
5. **Re-Attach via Step 40**: A transactional attachment is executed onto the newly discovered `WorkerW` host.
6. **Bounded Budget**: Maximum 3 recovery attempts are allowed. If Explorer is in an unrecoverable crash loop, KinetiWall transitions to `SynchronizationFailed` and logs a structured diagnostic event instead of hammering the system.

---

## 6. Verification & Test Coverage

- **Unit Tests (`synchronization_tests.rs`)**:
  - `test_sync_no_op_when_geometry_unchanged`: Confirms no-op coalescing (`mutation_applied: false`).
  - `test_sync_applies_single_mutation_on_changed_bounds`: Confirms single mutation applied, generation incremented.
  - `test_sync_rejects_zero_or_negative_geometry`: Rejects zero/negative dimensions safely.
  - `test_sync_coalesces_rapid_resolution_events`: Proves rapid identical events trigger only 1 mutation.
  - `test_sync_reentrancy_anti_loop_guard`: Proves re-entrant events do not loop or panic.
  - `test_sync_display_change_event_dispatch`: Validates resolution and DPI event handling.
  - `test_sync_host_invalidation_and_safe_recovery`: Verifies live rediscovery and reattachment on host death.
  - `test_sync_recovery_budget_exhaustion`: Proves 3-attempt circuit breaker transitions to `SynchronizationFailed`.
  - `test_sync_shutdown_cancels_and_rejects_new_sync`: Verifies rejection of sync operations post-shutdown.
  - `test_sync_event_history_bounded`: Verifies bounded ring buffer (`MAX_SYNC_DIAGNOSTIC_EVENTS = 100`).
- **Native Integration Tests (`native_integration_tests.rs`)**:
  - `test_native_surface_synchronization_and_coalescing`: Real Win32 window creation, attachment, coalesced sync, dynamic geometry resizing, and clean teardown.

