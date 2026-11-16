# KinetiWall Resource-Aware Adaptive Playback Quality, Frame Pacing & Performance Budgets

**Phase:** Phase 8, Intelligent Playback Policies & System-Aware Optimization  
**Step:** 51 of 90  
**Target Platform:** Windows 10 & Windows 11  
**Category:** Resource Monitoring, Performance Budgets, Frame Pacing, Hysteresis, and Lossless Settings Restoration  

---

## 1. Architectural Overview

Step 51 introduces a deterministic, opt-in adaptive quality management layer to KinetiWall. Under sustained host resource pressure (e.g. elevated application CPU utilization, frame rendering delays, or battery-saver constraints), KinetiWall evaluates affected playback sessions and safely throttles supported rendering cadences. When conditions clear, the system losslessly restores user-selected canonical settings without introducing stutter, configuration corruption, or rapid oscillation.

```
┌─────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                     Resource Signal Observations                                        │
│  - PlaybackResourceObservation: ProcessCpuPercent, WorkingSetBytes, RenderFrameTimeMs, DroppedFrames    │
│  - Scope: Application-level, DisplaySession-level, Process-level (PID attribution)                      │
│  - Data Quality: Valid, Degraded, Unavailable (Stale/missing samples NEVER treated as zero pressure)    │
└────────────────────────────────────────────────────┬────────────────────────────────────────────────────┘
                                                     │ Bounded Sample Sliding Window
                                                     ▼
┌─────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                   Sustained Pressure & Hysteresis Detector                              │
│  - Spike Rejection: Short-lived spikes (e.g. brief app launches) do NOT trigger downgrades              │
│  - Entry Window: Requires sustained pressure above 75% for >= 3000ms                                    │
│  - Hysteresis Recovery: Requires continuous dwell below 40% for >= 5000ms before returning to Normal   │
└────────────────────────────────────────────────────┬────────────────────────────────────────────────────┘
                                                     │ PressureState (Normal, SustainedPressure, Unknown)
                                                     ▼
┌─────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                 Central Deterministic Policy Evaluator                                  │
│  - Settings: Opt-in (disabled by default for existing users; enabled = false)                           │
│  - Canonical vs Effective: Desired settings are canonical; optimizer overrides are temporary in-memory  │
│  - Cooldown Guard: Minimum 4000ms cooldown prevents rapid quality flapping                              │
│  - Safe Floor: Minimum target frame rate floor (clamped in [15, 60] FPS; default 30 FPS)                │
└────────────────────────────────────────────────────┬────────────────────────────────────────────────────┘
                                                     │ AdaptivePolicyDecision
                                                     ▼
┌─────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                 Backend Capability Gate & Policy Interlocks                             │
│  - Video (libmpv): Supports frame-rate capping and render cadence throttling                            │
│  - Web (WebView2): Supports visualizer downsampling; strictly NO arbitrary script or DOM throttling     │
│  - Fullscreen/Manual Pause: Throttling suppressed for paused sessions; clearing pressure NEVER resumes  │
│  - Static Fallback: Still image sessions (Step 50) recognized and bypass redundant throttling           │
│  - Multi-Monitor: Independent isolation, Clone synchronous match, Span unified presentation             │
└─────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

---

## 2. Resource Signal Model

KinetiWall defines a typed, privacy-preserving observation model for operational signals:

- **Attribution Scopes:**
  - `ResourceObservationScope::Application`: Global desktop engine resource signals.
  - `ResourceObservationScope::DisplaySession(DisplayId)`: Sourced and attributed to a specific display's active player session.
  - `ResourceObservationScope::Process(u32)`: Sourced from a specific child player host PID (via Win32 `K32GetProcessMemoryInfo` / `GetProcessTimes`).
- **Metric Categories:**
  - `ProcessCpuPercent`: Normalized CPU percentage across cores (0.0 to 100.0%).
  - `ProcessWorkingSetBytes`: Physical RAM assigned to the player process.
  - `RenderFrameTimeMs`: Render duration per frame in milliseconds.
  - `DroppedFramesCount`: Count of late/dropped frames in observation interval.
  - `RenderCadenceFps`: Presentation refresh cadence.
- **Freshness & Quality Semantics:**
  - Every sample carries `validity_window_millis`. Samples older than their validity window are discarded as stale.
  - Signals carry `ResourceSignalQuality` (`Valid`, `Degraded`, `Unavailable`).
  - **Unknown Handling:** If measurements become unavailable or stale, the detector transitions to `PressureState::Unknown`. The engine **never substitutes zero** or infers zero pressure from missing data.

---

## 3. Sustained Pressure Detection & Hysteresis

To prevent intrusive quality flapping during normal transient computer activity (e.g. opening a browser tab, file loading), the engine uses asymmetric thresholds and duration windows:

1. **Transient Spike Rejection:** A sample spike above the pressure threshold starts an observation timer (`high_pressure_since`). If pressure drops before `sustained_pressure_threshold_ms` (default 3000ms), no adjustment is made.
2. **Hysteresis Entry Threshold:** Sustained CPU $\ge 75\%$ for $\ge 3000\text{ ms}$ transitions state to `SustainedPressure`.
3. **Hysteresis Recovery Threshold:** To restore quality toward `Normal`, CPU must drop below $40\%$ and dwell continuously for at least `recovery_dwell_threshold_ms` (default 5000ms). Any intermediate reading above $40\%$ resets the dwell timer.
4. **Adjustment Cooldown:** Successive quality adjustments enforce a minimum cooldown interval (`adjustment_cooldown_ms`, default 4000ms). During cooldown, adjustments return `AdaptiveAction::HoldCooldown`.

---

## 4. Backend Capability Matrix

KinetiWall advertises truthful runtime capabilities per media backend:

| Capability Control | Video (`libmpv`) | Web (`WebView2`) | Static Fallback (Step 50) |
|---|:---:|:---:|:---:|
| Dynamic Frame Rate Capping | **Yes** (15 - 60 FPS) | **No** (unsupported) | N/A (still image) |
| Render Cadence Throttling | **Yes** | **Yes** (visualizer cadence) | N/A (still image) |
| Visualizer Downsampling | **No** | **Yes** (smoothing / sample rate) | N/A (still image) |
| Lossless Accurate Restoration | **Yes** | **Yes** | **Yes** |
| Arbitrary DOM / Script Throttling | N/A | **No** (strictly prohibited) | N/A |

---

## 5. Separation of Canonical User Preferences from Effective Overrides

A core reliability mandate is that temporary optimizations must never corrupt user settings:
- **Canonical Configuration:** The user's chosen frame rate, scaling, and display assignment are stored persistently and remain canonical.
- **Effective Override:** The optimizer calculates an in-memory `EffectiveQualityOverride` (`Normal`, `Reduced`, `MinimumEligible`).
- **User Modification While Throttled:** If the user adjusts playback settings while optimization is active, the canonical configuration updates immediately, and the optimizer re-evaluates effective values against the new canonical target.
- **Restoration:** When optimization is disabled or resource pressure clears, `evaluate_session` issues `AdaptiveAction::RestoreSettings`, reverting the effective frame rate directly to the canonical configuration.

---

## 6. Multi-Policy Interlocks

The adaptive quality layer coordinates with existing policies:
1. **Fullscreen & Manual Pause (Step 48):** If a session has `PauseReason::UserManual` or `PauseReason::Fullscreen`, quality throttling is suppressed (`AdaptiveAction::NoChange`). Clearing resource pressure **never resumes** a paused session.
2. **Power Policy (Step 49):** If battery saver or low-battery constraints are active, `allow_power_policy_coordination` allows the evaluator to command `MinimumEligible` quality tier to maximize host energy conservation.
3. **Static Fallback (Step 50):** If a session is in `DisplaySessionState::StaticFallback`, it is already displaying a still image and does not require render throttling.

---

## 7. Multi-Monitor Presentations

- **Independent Mode:** Each monitor evaluates its own session-specific resource observations. High load on Display 1 will not degrade Display 2 if Display 2's load remains normal.
- **Clone Mode:** Cloned secondary displays follow the authoritative primary monitor's decision synchronously, preventing frame pacing desynchronization.
- **Span Mode:** A single unified virtual desktop presentation receives a single coordinated adaptive quality decision across all participating monitors.

---

## 8. Verification & Test Suite

The adaptive quality subsystem is validated by 15 dedicated automated tests in `crates/kinetiwall-desktop/tests/adaptive_tests.rs`:

1. `test_signal_validation_and_freshness`: Validates observation freshness, expiration, and scopes.
2. `test_sustained_pressure_detector_spike_rejection`: Proves isolated spikes do not trigger downgrades.
3. `test_sustained_pressure_detector_sustained_trigger`: Validates 3000ms sustained pressure entry.
4. `test_sustained_pressure_detector_hysteresis_recovery`: Validates 5000ms low-pressure dwell requirement.
5. `test_sustained_pressure_detector_unknown_handling`: Proves missing metrics yield `Unknown`, not zero pressure.
6. `test_adaptive_evaluator_opt_in_disabled_by_default`: Enforces strict opt-in default behavior.
7. `test_adaptive_evaluator_downgrades_to_reduced_under_pressure`: Verifies transition to `Reduced` tier.
8. `test_cooldown_suppresses_rapid_quality_oscillation`: Verifies `HoldCooldown` prevents quality flapping.
9. `test_canonical_settings_preserved_during_override`: Verifies lossless return to canonical user FPS.
10. `test_backend_capability_gate_skips_unsupported`: Confirms unsupported backends are safely skipped.
11. `test_policy_interlock_respects_manual_pause_and_fullscreen`: Confirms pause states are preserved.
12. `test_static_fallback_sessions_suppress_throttling`: Proves still-image sessions bypass throttling.
13. `test_multi_monitor_clone_mode_synchronized_decision`: Verifies synchronous clone decisions.
14. `test_multi_monitor_independent_isolation`: Proves per-monitor failure and load isolation.
15. `test_bounded_diagnostic_event_history`: Verifies 128-event diagnostic ring log bounds.

