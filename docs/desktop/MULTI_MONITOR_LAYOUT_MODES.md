# Multi-Monitor Layout Modes, Wallpaper Spanning & Synchronized Clone Playback

## Architectural Overview

Step 45 introduces the **Multi-Monitor Presentation Mode Subsystem** (`MultiMonitorLayoutCoordinator` and `LayoutPlanResolver`). This layer coordinates the layout presentation of wallpaper surfaces across multiple connected displays, supporting **Independent**, **Span**, and **Clone** modes with validated geometry, honest synchronization reporting, and transactional reversibility.

```
+---------------------------------------------------------------------------------------------------+
|                                  MultiMonitorLayoutCoordinator                                    |
+---------------------------------------------------------------------------------------------------+
       |                                       |                                      |
       v                                       v                                      v
+-----------------------------+ +-----------------------------+ +-----------------------------+
|    Display Topology (S42)   | |  Assignment Service (S43)   | |  Session Coordinator (S44) |
| - Snapshot generation       | | - Saved per-display intent  | | - Independent per-display   |
| - Virtual screen bounds     | | - Layout mode preference    | |   lifecycle management      |
| - Negative offsets preserved| | - Persistent assignments    | | - Failure isolation         |
+-----------------------------+ +-----------------------------+ +-----------------------------+
                                       |
                                       v
                     +-----------------------------------+
                     |       MultiMonitorLayoutPlan      |
                     | - selected_mode vs effective_mode |
                     | - virtual_desktop_bounds          |
                     | - viewport_mappings (0,0 relative)|
                     | - surface_plans                   |
                     | - authoritative_wallpaper_id      |
                     | - validation_outcome              |
                     +-----------------------------------+
                                       |
         +-----------------------------+-----------------------------+
         |                             |                             |
         v                             v                             v
+------------------+          +------------------+          +------------------+
|   INDEPENDENT    |          |       SPAN       |          |      CLONE       |
| - 1 surface/disp |          | - 1 surface covers|         | - Multiple surfs |
| - Distinct walls |          |   virtual desktop|          | - Same wallpaper |
| - Baseline flow  |          | - Unified bounds |          | - BestEffort sync|
+------------------+          +------------------+          +------------------+
```

---

## 1. The Three Presentation Modes

### 1.1 Independent Mode (Baseline)
- **Concept**: Each connected monitor acts as an independent canvas displaying its own assigned wallpaper (or global fallback).
- **Surface Allocation**: Exactly 1 native player surface per connected display, positioned to that display's `monitor_rect`.
- **Assignment Mapping**: Direct 1-to-1 lookup against `AssignmentService`.
- **Failure Isolation**: An error or crash on Display 2 does not affect Display 1.
- **Delta Reconciliation**: Unchanged sessions are retained without restarting or flickering.

### 1.2 Span Mode (Virtual Desktop Canvas)
- **Concept**: A single continuous wallpaper presentation across the bounding rectangle of participating displays.
- **Virtual Desktop Geometry**:
  - The bounding rectangle is computed as the geometric union of all participating displays' monitor rectangles.
  - Negative screen coordinates (e.g. secondary monitor placed to the left at `left: -1920, top: 0`) are preserved.
  - Displays with heterogeneous resolutions, aspect ratios, or vertical offsets are enclosed safely.
- **Surface Allocation**: A single native surface window sized to `virtual_desktop_bounds`, positioned at `(virtual_desktop_bounds.left, virtual_desktop_bounds.top)` on the desktop host candidate (`WorkerW`).
- **Authoritative Wallpaper**: The wallpaper assigned to the primary display (or the first available assignment) is selected as the authoritative asset for the entire span.
- **Preservation of Intent**: Switching to Span mode does *not* overwrite or erase the individual per-monitor assignments saved in `AssignmentStoreData`. Returning to Independent mode cleanly restores each monitor's individual wallpaper.

### 1.3 Clone Mode (Replicated Presentation)
- **Concept**: The same wallpaper is displayed across all participating displays.
- **Surface Allocation**: Multiple independent native player surfaces (one per monitor), each sized and positioned to its respective monitor bounds.
- **Authoritative Wallpaper Selection**:
  - `CloneSourcePolicy::PrimaryDisplay` (default): Uses the primary display's assigned wallpaper across all displays.
  - `CloneSourcePolicy::ExplicitWallpaper(id)`: Uses a designated wallpaper ID for the group.
  - `CloneSourcePolicy::StrictMatch`: Requires all participating monitors to already match; reports `ConflictingCloneAssignments` if contradictory.
- **Playback Synchronization**:
  - **Visual Cloning vs. Frame Genlock**: Visual cloning ensures identical content and configuration across surfaces. Temporal frame lock requires specialized hardware genlock or tightly synchronized media clocks.
  - KinetiWall truthfully advertises `PlaybackSyncLevel::BestEffort` (concurrent start with timeline alignment) rather than claiming non-existent frame-perfect genlock.
- **Failure Isolation**: If one display session crashes or disconnects in Clone mode, remaining displays continue playing uninterrupted.

---

## 2. Mode State Hierarchy

To prevent false state reporting, KinetiWall distinguishes between:
1. **Saved Mode (`saved_mode`)**: Persisted preference in `AssignmentStoreData`.
2. **Requested Mode (`requested_mode`)**: Mode currently being evaluated or requested for activation.
3. **Effective Mode (`effective_mode`)**: The mode permitted by the current validated topology (e.g. single-monitor systems naturally simplify to `Independent` with `SingleDisplayOnly` outcome).
4. **Active Mode (`active_mode`)**: The mode whose runtime transition has completed, surfaces attached, bounds verified, and playback started.

---

## 3. Transactional Mode Transitions & Rollback

Mode changes follow a strict multi-stage transition pipeline:
1. **Plan Evaluation**: Resolve `MultiMonitorLayoutPlan` using `LayoutPlanResolver`.
2. **Validation Guard**: If the plan is not activatable (e.g. conflicting assignments, missing assets, invalid bounds), transition is rejected *before* any OS windows are mutated.
3. **Execution**:
   - Deactivate obsolete sessions (e.g. stopping individual surfaces when entering Span, or stopping the Span surface when entering Independent).
   - Activate required replacement surfaces via `DisplaySessionCoordinator`.
4. **Rollback on Failure**: If any surface creation or attachment fails, the coordinator rolls back to the previous mode and restores healthy sessions.
5. **State Commitment**: `active_mode` is updated only after all native surface checks pass.

