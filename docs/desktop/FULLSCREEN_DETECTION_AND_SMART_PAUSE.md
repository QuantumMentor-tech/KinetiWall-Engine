# KinetiWall Foreground Application Awareness, Fullscreen Detection & Smart Pause/Resume

**Phase:** Phase 8, Intelligent Playback Policies & System-Aware Optimization  
**Step:** 48 of 90  
**Target Platform:** Windows 10 & Windows 11  
**Category:** Foreground Window Detection, Fullscreen Classification, Playback Policy Evaluation, Per-Display Pause/Resume, and Resource Optimization  

---

## 1. Architectural Overview

Step 48 introduces the intelligent playback policy system for KinetiWall, enabling event-driven foreground application awareness, geometry-accurate fullscreen detection, and debounced smart pause/resume across independent and grouped display sessions.

```
┌─────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                     DesktopForegroundObserver                                           │
│  - Queries GetForegroundWindow via DesktopWindowProvider                                                │
│  - Strips invisible drop-shadow margins using DwmGetWindowAttribute (DWMWA_EXTENDED_FRAME_BOUNDS)       │
│  - Inspects IsIconic (minimized) & DWMWA_CLOAKED (virtual desktops / background apps)                   │
│  - PRIVACY MANDATE: Collects strictly handle, PID, class name, & bounds (NO titles, URLs, or keylogs)   │
└────────────────────────────────────────────────────┬────────────────────────────────────────────────────┘
                                                     │ ForegroundObservation
                                                     ▼
┌─────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                       FullscreenClassifier                                              │
│  - Distinguishes borderless/exclusive fullscreen from ordinary maximized windows conforming to work area│
│  - Excludes KinetiWall UI/player processes and shell classes (WorkerW, Progman, Shell_TrayWnd)          │
│  - Classifies: NotFullscreen | ProbableFullscreen | FullscreenSpanning | MaximizedWorkArea | Excluded   │
└────────────────────────────────────────────────────┬────────────────────────────────────────────────────┘
                                                     │ FullscreenClassification
                                                     ▼
┌─────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                  SmartPlaybackPolicyEvaluator                                           │
│  - Anti-flapping debouncing (default: 250ms threshold) before applying pause actions                     │
│  - Independent pause reasons (PauseReason::Fullscreen vs PauseReason::UserManual)                       │
│  - Reversible resume: manual pauses are strictly respected when fullscreen applications exit            │
│  - Configurable pause modes: PerDisplay (unaffected monitors continue) vs AllDisplays                  │
│  - Bounded diagnostic history (MAX_POLICY_DIAGNOSTIC_EVENTS = 256)                                       │
└────────────────────────────────────────────────────┬────────────────────────────────────────────────────┘
                                                     │ Suspend / Resume Commands
                                                     ▼
┌─────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                  DisplaySessionCoordinator                                              │
│  - Suspends and resumes native player session backends (Video / Web) per display                        │
│  - Integrates seamlessly with MultiMonitorLayoutCoordinator (Independent, Span, Clone)                  │
└─────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

---

## 2. Geometry-Based Fullscreen Classification vs. Maximization

Ordinary maximized desktop applications (such as web browsers, code editors, and file managers) conform strictly to the monitor's `work_area_rect`, leaving the taskbar visible. In contrast, exclusive or borderless fullscreen games and media players cover the entire `monitor_rect`, including the taskbar area.

1. **Extended Frame Bounds**: Standard Win32 `GetWindowRect` includes invisible 7–8px drop-shadow resize padding on Windows 10/11. The classifier queries `DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, ...)` to obtain true visible client/frame edges.
2. **Work Area Discrimination**: If window bounds conform to `work_area_rect` (within 4px tolerance) while the taskbar occupies space (`monitor_rect != work_area_rect`), the window is classified as `MaximizedWorkArea` rather than `ProbableFullscreen`.
3. **Excluded Shell Elements**: Windows desktop infrastructure (e.g. `Shell_TrayWnd`, `Shell_SecondaryTrayWnd`, `WorkerW`, `Progman`, `ShellDll_DefView`) and KinetiWall's own application processes are excluded to avoid self-pause feedback loops.
4. **Minimized and Cloaked Suppression**: Windows that are minimized (`IsIconic`) or cloaked by DWM (e.g. on inactive virtual desktops or background UWP suspensions) are classified as `NotFullscreen`.

---

## 3. Independent Pause Reasons & Reversibility

To prevent unwanted auto-resumptions, KinetiWall tracks pause causes using an independent set of reasons per display (`HashSet<PauseReason>`):

- `PauseReason::Fullscreen { display_id }`: Applied automatically by the smart playback policy.
- `PauseReason::UserManual`: Applied explicitly when the user pauses wallpaper playback.
- `PauseReason::PowerSaving`: Reserved for future power-state integration.

**Reversibility Guarantee:** When a fullscreen application exits or loses focus, `PauseReason::Fullscreen` is removed. The wallpaper session is resumed **only if no other pause reasons remain active**. If a user had manually paused a display session before starting a game, exiting the game will leave the session safely paused.

---

## 4. Multi-Monitor Presentation Modes & Policy Interactions

| Presentation Mode | Fullscreen on Display 1 (`PerDisplay` Setting) | Fullscreen on Display 1 (`AllDisplays` Setting) |
| :--- | :--- | :--- |
| **Independent** | Display 1 pauses; Display 2 continues uninterrupted. | Both Display 1 and Display 2 pause. |
| **Clone** | Both displays pause (synchronized shared playback). | Both displays pause. |
| **Span** | Entire spanned virtual desktop session pauses. | Entire spanned virtual desktop session pauses. |

---

## 5. Privacy-First Mandate

KinetiWall's foreground awareness operates strictly locally with zero external network transmission:
- **No Window Titles**: Window captions, document filenames, and tab titles are never queried or stored.
- **No Text or Keystrokes**: Zero input hooking (`WH_KEYBOARD_LL`, raw input) or keyboard interception.
- **No Screenshots**: Zero GDI/DXGI frame captures or visual pixel sampling of foreground applications.
- **Minimal Metadata**: Strictly retains `HWND`, process ID, thread ID, window class name, bounds, and minimization flags in volatile memory.

---

## 6. Frontend & User Interface Integration

The multi-monitor management view (`DisplaysView.ts`) exposes user controls under **Smart Playback Policy**:
- **Switch Toggle**: "Smart Pause on Fullscreen Apps" enables or disables automated pausing.
- **Instant Reconciliation**: Turning off smart pause immediately releases active fullscreen pause reasons and resumes affected displays.
- **Privacy Notice**: Transparently informs users of the zero-title, zero-keystroke privacy guarantees.

