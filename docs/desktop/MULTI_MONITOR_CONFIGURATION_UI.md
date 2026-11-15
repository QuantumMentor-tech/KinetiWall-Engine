# KinetiWall Multi-Monitor Configuration UI & Assignment Management

**Phase:** Phase 7, Native Windows Desktop Hosting & Wallpaper Surface Integration  
**Step:** 46 of 90  
**Target Platform:** Windows 10 & Windows 11  
**Category:** UI/UX Integration, Multi-Monitor Configuration, Display Selection, Assignment Management, and Runtime Status  

---

## 1. Executive Architecture Summary

Step 46 connects KinetiWall's native multi-monitor display enumeration (Step 42), per-monitor wallpaper assignment persistence (Step 43), runtime session orchestration (Step 44), and layout presentation modes (Step 45) to the frontend user interface.

The design adheres strictly to the **Neon Glass** design system, maintains accessible HTML semantics (`role="radiogroup"`, `role="dialog"`, `aria-modal="true"`, keyboard navigation), guarantees zero renderer access to raw Win32 HWND/HMONITOR handles, and implements truthful runtime status reporting.

```
┌────────────────────────────────────────────────────────────────────────┐
│                        KinetiWall AppShell                             │
│  ┌───────────────────────┬──────────────────────────────────────────┐  │
│  │ Navigation Rail       │ DisplaysView (Active Route)              │  │
│  │  • Dashboard          │  ┌────────────────────────────────────┐  │  │
│  │  • Wallpaper Library  │  │ LayoutModeSelector                 │  │  │
│  │  • Displays [Active]  │  │  [Independent] [Span] [Clone]      │  │  │
│  │  • Settings [Disabled]│  └────────────────────────────────────┘  │  │
│  │                       │  ┌────────────────────────────────────┐  │  │
│  │                       │  │ DisplayArrangementView (Canvas)    │  │  │
│  │                       │  │  [Mon 1 (Primary)]  [Mon 2]        │  │  │
│  │                       │  └────────────────────────────────────┘  │  │
│  │                       │  ┌────────────────────────────────────┐  │  │
│  │                       │  │ Connected Monitor Cards Grid       │  │  │
│  │                       │  │  • Card 1: 2560x1440, Live Active  │  │  │
│  │                       │  │  • Card 2: 1920x1080, Unassigned   │  │  │
│  │                       │  └────────────────────────────────────┘  │  │
│  └───────────────────────┴──────────────────────────────────────────┘  │
└────────────────────────────────────▲───────────────────────────────────┘
                                     │ Reactive State Subscriptions
┌────────────────────────────────────┴───────────────────────────────────┐
│              DisplayStore (Sequence-Guarded State & Rollback)          │
│              - snapshot, assignments, sessions, savedMode, activeMode   │
└────────────────────────────────────▲───────────────────────────────────┘
                                     │ Typed DTOs & Coordinate Normalization
┌────────────────────────────────────┴───────────────────────────────────┐
│                    DisplayIpcClient & Transport                        │
└────────────────────────────────────▲───────────────────────────────────┘
                                     │ Tauri IPC Boundary
┌────────────────────────────────────┴───────────────────────────────────┐
│        Rust kinetiwall-desktop Backend (Steps 39, 40, 41, 42, 43, 44, 45)│
└────────────────────────────────────────────────────────────────────────┘
```

---

## 2. UI Route Integration & Navigation Rails

1. **Centralized Navigation Registry (`src-ui/routes.ts`):**
   - The route `displays` is marked `isAvailable: true`, enabling the "Displays" link in the shell navigation rail.
   - Retains disabled guardrails for unimplemented routes (`settings`).
2. **AppShell Controller (`src-ui/components/shell/AppShell.ts`):**
   - Accepts an optional `DisplayStore` or initializes an in-memory transport fallback.
   - Automatically invokes `displayStore.refreshTopology()` upon first navigation to the `displays` route.
   - Routes rendering to `DisplaysView` when `routerState.activeRoute === 'displays'`.
3. **Dashboard Quick Navigation (`src-ui/views/DashboardView.ts`):**
   - Displays a dedicated "Displays & Layout" summary card with a direct "Manage Displays" button to navigate directly to the configuration view.

---

## 3. Typed IPC Boundary & Transport Layer (`src-ui/services/displayIpcClient.ts`)

- **Strict Separation of Concerns:**
  The frontend never calls Win32 windowing or monitor APIs. All data exchanges pass through typed DTOs:
  - `DisplayTopologySnapshotDto`: Contains sanitized device names, friendly names, geometry, primary flag, and virtual desktop bounding box.
  - `ResolvedDisplayAssignmentDto`: Contains resolution outcome (`valid_explicit`, `valid_global_fallback`, `unassigned`, etc.) and effective title.
  - `MultiMonitorStatusDto`: Contains saved presentation mode, active presentation mode, and running session descriptors.
- **Coordinate Sanitization:**
  Extracts geometry and ensures safe parsing of virtual coordinates, including monitors with negative virtual desktop coordinates (`left < 0`).
- **In-Memory Mock Transport (`createMockDisplayIpcTransport`):**
  Provides a fully functional dual-monitor mock transport for unit testing and headless SSR validation without requiring a live Windows desktop or Tauri backend.

---

## 4. Reactive State Management (`src-ui/stores/displayStore.ts`)

- **State Model:**
  ```typescript
  export interface DisplayStoreState {
    snapshot: DisplayTopologySnapshotDto | null;
    resolvedAssignments: ResolvedDisplayAssignmentDto[];
    sessions: DisplaySessionStatusDto[];
    savedMode: MultiMonitorLayoutMode;
    activeMode: MultiMonitorLayoutMode | null;
    selectedDisplayId: DisplayId | null;
    isLoading: boolean;
    isSaving: boolean;
    isApplying: boolean;
    error: string | null;
    lastUpdatedMillis: number;
    isPickerOpen: boolean;
    pickerTargetDisplayId: DisplayId | null;
  }
  ```
- **Sequence Monotonicity:**
  Uses a monotonic `nextRequestId` counter to prevent stale or out-of-order IPC responses from overwriting newer user interactions.
- **Transactional Rollback:**
  If setting a layout mode fails at the host level, the store immediately rolls back `savedMode` to its prior state and surfaces the failure in `error`.
- **Display Selection & Picker Management:**
  Tracks the currently selected monitor and handles opening/closing the wallpaper assignment picker modal.

---

## 5. Visual Display Arrangement Canvas (`src-ui/components/domain/DisplayArrangementView.ts`)

- **Negative Coordinate Normalization:**
  In multi-monitor setups, monitors placed to the left or above the primary monitor have negative desktop coordinates (e.g. `left: -1920`). The canvas normalizes these coordinates by computing:
  $$\text{left}_{\text{canvas}} = \text{offsetX} + (\text{display.left} - \text{minX}) \times \text{scale}$$
  $$\text{top}_{\text{canvas}} = \text{offsetY} + (\text{display.top} - \text{minY}) \times \text{scale}$$
  This maps all monitors cleanly to a `[0, canvasWidth]` and `[0, canvasHeight]` space.
- **Visual Features:**
  - Index number badge (`1`, `2`, `3`).
  - Primary monitor indicator star (`★`).
  - Active session dot (`●`) with pulse animation when wallpaper is actively playing.
  - Friendly display name and native resolution (`2560×1440`).
  - Assigned wallpaper title snippet.
  - Interactive selection outline (Neon Cyan border `#00F0FF`).
  - Full keyboard accessibility (`Enter` and `Space` to select, `aria-selected` attribute).

---

## 6. Presentation Layout Mode Selector (`src-ui/components/domain/LayoutModeSelector.ts`)

Presents the three layout modes established in Step 45:
1. **Independent:** Distinct wallpaper assigned to each monitor.
2. **Span Virtual Desktop:** Stretches a wide wallpaper across all monitors.
3. **Clone / Duplicate:** Duplicates the primary wallpaper across all monitors.

### Safeguards & Truthful Reporting:
- **Single-Display Safeguard:**
  When only one display is detected (`displayCount <= 1`), multi-monitor modes (`span` and `clone`) are disabled and visually flagged with `⚠ Requires 2 or more connected displays`.
- **Honest Playback Synchronization Disclaimer:**
  For `clone` mode, the UI explicitly notes:  
  *“ℹ Best-effort software synchronization (hardware genlock not claimed).”*  
  This prevents any false implication of hardware-level genlock or tearing-free cross-GPU synchronization.
- **Pending Changes Indicator:**
  Displays a warning badge when `activeMode !== currentMode` indicating that changes require clicking "Apply Layout".

---

## 7. Display Card Component (`src-ui/components/domain/DisplayCard.ts`)

Each connected monitor is represented by a detailed card displaying:
- **Header:** Friendly name (e.g. `LG UltraGear 27"`), device path (`\\.\DISPLAY1`), and primary badge.
- **Session Status Badge:**
  - `● Live Wallpaper Active` (emerald green) when session is running.
  - `⏳ Starting Playback` (cyan) when initiating.
  - `✕ Playback Failed: [reason]` (magenta/red) on crash or error.
  - `⚠ Host Detached` (warning amber) when WorkerW host is invalid.
  - `■ Playback Stopped` or `○ No Session` (neutral).
- **Specs Grid:** Native resolution (`2560 × 1440`), virtual coordinates `(0, 0)`, and DPI scale factor (`100%`).
- **Assignment Section:**
  - Displays assigned wallpaper title or *"No wallpaper assigned"*.
  - Notes assignment source (*"Explicit assignment"* vs *"Global fallback"*).
  - "Change Wallpaper..." button opening the accessible picker modal.
  - "Clear" button (available when explicit assignment is set) to remove the per-monitor assignment.

---

## 8. Accessible Wallpaper Picker Modal (`src-ui/components/domain/WallpaperPickerModal.ts`)

- **Accessible Dialog:**
  - Uses `role="dialog"`, `aria-modal="true"`, and `aria-labelledby="wallpaper-picker-title"`.
  - Backdrop clicks or Escape key close the dialog.
- **Catalog Availability Enforcement:**
  - Surfaces registered wallpapers from `CatalogStore`.
  - Available wallpapers display an enabled "Assign" button.
  - Wallpapers with missing or inaccessible assets display an *"Unavailable"* badge and have disabled action buttons.

---

## 9. Verification & Automated Test Matrix

The frontend test suite (`node --test src-ui/tests/*.test.ts`) and build verification (`src-ui/scripts/verifyBuild.ts`) verify all contracts:

| Test Suite | Tests | Result | Notes |
|:---|:---:|:---:|:---|
| `multiMonitorUi.test.ts` | 10 | **PASS** | Tests IPC mapping, store subscriptions, layout selector, canvas normalization, display cards, modal accessibility, and AppShell routing. |
| `audioReactivityUi.test.ts` | 4 | **PASS** | Audio store opt-in, parameter clamping, and reactive panel. |
| `catalogManagementUi.test.ts` | 4 | **PASS** | Metadata editing, relinking, and catalog removal safeguards. |
| `libraryBrowsing.test.ts` | 6 | **PASS** | Navigation from dashboard to library, responsive grid, and disabled route guardrails. |
| `librarySearchFilterSort.test.ts` | 4 | **PASS** | Search filtering, type filtering, and deterministic sorting. |
| `wallpaperDetailsPanel.test.ts` | 3 | **PASS** | Metadata inspection, path redaction, and focus restoration. |
| **Total Frontend** | **31** | **PASS** | **100% pass rate** across all 6 test suites. |
| **Build Verification** | 1 | **PASS** | SSR, DOM escaping, token invariants, and route navigation. |
| **Rust Backend Workspace** | **208** | **PASS** | All 6 crates compile and pass tests cleanly. |

