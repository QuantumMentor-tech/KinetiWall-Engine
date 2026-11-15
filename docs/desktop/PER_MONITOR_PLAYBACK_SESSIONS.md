# Per-Monitor Playback Sessions, Assignment Activation & Failure Isolation

## Architectural Overview

KinetiWall Step 44 introduces the **Per-Monitor Playback Session Coordination Subsystem** (`DisplaySessionCoordinator`). This layer bridges the per-monitor assignment domain model (Step 43) and native display topology snapshot engine (Step 42) with the validated desktop window attachment (Step 40) and surface synchronization infrastructure (Step 41).

```
+---------------------------------------------------------------------------------------------------+
|                                  DisplaySessionCoordinator                                        |
+---------------------------------------------------------------------------------------------------+
       |                                       |                                      |
       v                                       v                                      v
+-----------------------------+ +-----------------------------+ +-----------------------------+
|    Display Topology (S42)   | |  Assignment Service (S43)   | |  Attachment Coord (S40/S41) |
| - DisplayTopologySnapshot   | | - ResolvedDisplayAssignment | | - Atomic SetParent          |
| - Virtual screen bounds     | | - Intent vs. Presence check | | - Surface Sync Coalescing   |
| - Negative offsets preserved| | - Preserved on disconnect   | | - Style & rollback guards   |
+-----------------------------+ +-----------------------------+ +-----------------------------+
                                       |
                                       v
                     +-----------------------------------+
                     |       SessionPlan (Declarative)   |
                     | - CreateSession                   |
                     | - RetainSession (No-op)           |
                     | - UpdateGeometry                  |
                     | - ReconfigureSession              |
                     | - StopSession                     |
                     +-----------------------------------+
                                       |
                                       v
         +-----------------------------------------------------------+
         |               Independent Per-Monitor Sessions            |
         | +-----------------------+       +-----------------------+ |
         | | Session: Display 1    |       | Session: Display 2    | |
         | | - Uuid session_id     |       | - Uuid session_id     | |
         | | - Surface HWND 1      |       | - Surface HWND 2      | |
         | | - Bounds: [0, 0]      |       | - Bounds: [1920, 0]   | |
         | | - State: Active       |       | - State: Active       | |
         | +-----------------------+       +-----------------------+ |
         |            |                                |             |
         |   [FAILURE ISOLATION]              [FAILURE ISOLATION]    |
         | If Display 2 crashes, Display 1 remains rendering safely. |
         +-----------------------------------------------------------+
```

---

## 1. Core Principles & Mandates

1. **Independent Per-Monitor Execution**:
   - Every active connected display with a playable wallpaper is assigned a dedicated `DisplaySessionDescriptor`.
   - Each session owns its own native player surface window, lifecycle state, and generation counter.
   - Even if the user assigns the *exact same* wallpaper to multiple monitors, KinetiWall allocates distinct sessions and surface windows, ensuring separate rendering contexts and zero cross-talk.

2. **Declarative Reconciliation (`SessionPlan`)**:
   - Instead of imperatively tearing down and recreating players whenever desktop state changes, the coordinator compares active sessions with resolved assignments from `AssignmentService::resolve_all_displays` against the latest `DisplayTopologySnapshot`.
   - Produces a minimal delta plan:
     - `CreateSession`: Newly assigned or reconnected display.
     - `RetainSession`: Healthy session with unchanged assignment and geometry (zero flicker, zero OS mutations).
     - `UpdateGeometry`: Monitor resolution, DPI, or layout changed (bounds updated via Step 41 coalescing).
     - `ReconfigureSession`: Assigned wallpaper changed (surface re-hosted or re-instantiated).
     - `StopSession`: Monitor disconnected or assignment removed.

3. **Strict Failure Isolation**:
   - An initialization, hardware acceleration, or window attachment failure on Monitor 2 *never* halts or destabilizes Monitor 1.
   - Failures transition only the affected display's session to `DisplaySessionState::Failed { reason }`.
   - The coordinator records diagnostic telemetry and continues executing reconciliation for all other monitors.

4. **Preservation of User Intent on Disconnect**:
   - When a secondary monitor is unplugged or powered down, its runtime session is cleanly stopped, detached, and destroyed.
   - However, the saved assignment in `AssignmentService` remains persisted in `assignments.json`.
   - Once the monitor reconnects (recognized by stable hardware ID or device path), reconciliation detects the change and automatically reactivates the session.

5. **Stale Generation & Race Condition Prevention**:
   - Every session and coordinator iteration maintains a monotonic generation counter (`u64`).
   - Delayed OS callbacks, asynchronous messages, or out-of-order events from superseded plans are validated against `validate_generation` and rejected as stale.

---

## 2. Session Lifecycle State Machine

```
              +-------------------+
              |   Uninitialized   |
              +-------------------+
                        |
                        v
              +-------------------+
       +----->|     Starting      |
       |      +-------------------+
       |                |
       |     (attach & sync success)
       |                v
       |      +-------------------+       (suspend: sleep/lock)
       |      |      Active       |<=============================>+-------------------+
       |      +-------------------+                               |     Suspended     |
       |         |             |                                  +-------------------+
       |         |             | (detach for pause/reconfig)
       |         |             v
       |         |    +-------------------+
       |         |    |     Detached      |
       |         |    +-------------------+
(reconfig/retry) |             |
       |         |             +-------------------------+
       |         |                                       |
       |         v                                       v
       |      +-------------------+               +-------------------+
       +------| Failed { reason } |               |     Stopping      |
              +-------------------+               +-------------------+
                                                            |
                                                            v
                                                  +-------------------+
                                                  |      Stopped      |
                                                  +-------------------+
```

---

## 3. Win32 Multi-Surface Coordinate Handling

On Windows systems, desktop wallpaper attachment uses the `WorkerW` hierarchy spawned behind `SHELLDLL_DefView`.
- In a multi-monitor configuration, `WorkerW` typically covers the entire virtual desktop span (union of all monitor rectangles).
- Secondary displays placed to the left or above the primary monitor possess **negative screen coordinates** (e.g. `left: -1920, top: 0`).
- The player surface window for each session is positioned specifically at `(monitor_rect.left, monitor_rect.top, width, height)` relative to the desktop host window.
- `AttachmentCoordinator::synchronize_surface_geometry` ensures non-activating (`SWP_NOACTIVATE`) and non-reordering (`SWP_NOZORDER`) repositioning, preventing focus stealing and icon interference.

---

## 4. Diagnostics & Observability

`DisplaySessionCoordinator` maintains a bounded ring buffer of up to 100 `DisplaySessionDiagnosticEvent` records capturing:
- `session_id`: Unique Uuid of the affected session.
- `display_id`: Application-level display identifier.
- `timestamp_millis`: Wall-clock Unix timestamp.
- `event_type`: `Created`, `Activated`, `Retained`, `GeometryUpdated`, `Reconfigured`, `Suspended`, `Resumed`, `Detached`, `Failed`, or `Stopped`.
- `description`: Detailed human-readable description or error cause.

