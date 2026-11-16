# Unified Policy Arbitration, Conflict Resolution & Deterministic Playback Decisions

## 1. Architectural Mandate & Central Principle
Step 56 of 90 establishes authoritative, conflict-free policy arbitration across all supported playback policies in KinetiWall.

> **Central Principle:**
> "Policies express constraints and preferences. One authoritative orchestrator determines and applies the effective playback configuration. Individual policies must not independently fight over the same player."

Prior to Step 56, each subsystem (fullscreen detection, power saving, workstation lock, display power, user inactivity, adaptive quality, dynamic budgeting) maintained individual evaluation rules and communicated directly or semi-directly with session control. Step 56 centralizes this arbitration into a single deterministic evaluator: `PolicyArbitrator`.

---

## 2. Constraint Taxonomy & Precedence Hierarchy

Constraints are categorized strictly by semantic role:

1. **`HardSafety`**:
   - Application shutdown, session unregistration, backend unrecoverable crash, or explicit user manual pause (`PauseReason::UserManual`).
   - Manual user pauses strictly survive and override all automatic policies; no automatic event (e.g. window closing, unlock, wake) can clear a manual pause.
2. **`AutomaticSuspension`**:
   - Event-driven automatic pause policies:
     - `Fullscreen`: Foreground window occupying display.
     - `WorkstationLocked`: Windows desktop locked (`WTS_SESSION_LOCK`).
     - `DisplayPowerOff`: Display panel powered off (`GUID_CONSOLE_DISPLAY_STATE`).
     - `UserInactivity`: System input idle past configured threshold (`GetLastInputInfo`).
     - `PowerSaving` / `SystemSuspended`: AC to battery or OS sleep/hibernation.
     - `RemoteSession`: RDP/remote connection optimization.
   - **Resolution**: If **any** automatic pause reason is present, the session must be suspended. Resumption to live playback requires that **all** active pause reasons clear.
   - **Fallback Presentation**: If static fallback is enabled and a valid source exists, the session transitions to `DesiredSessionState::StaticFallback` instead of `Suspended`.
3. **`ResourceQuality`**:
   - Framerate pacing and cadence throttling (`adaptive_fps_cap`, `budget_fps_cap`).
   - **Composition**: The arbitrator computes the most restrictive limit:
     $$\text{Effective FPS} = \min(\text{adaptive\_cap}, \text{budget\_cap}, \text{canonical\_fps})$$
4. **`UserPreference`**:
   - Baseline target framerate, fallback preferences, and display assignment rules.

---

## 3. Pure Arbitration Contract

`PolicyArbitrator::arbitrate(input: &SessionArbitrationInput) -> EffectivePlaybackDecision` is a pure function:
- **Zero side-effects**: It reads constraints and state without modifying coordinators or emitting IPC events.
- **Deterministic**: Given the identical `SessionArbitrationInput`, it produces an identical `EffectivePlaybackDecision`.
- **Structured Explanation**: Provides structured `PolicyExplanationCode` entries explaining exactly why the decision was taken (e.g. `ManualPauseActive`, `FullscreenActive`, `WorkstationLocked`, `ResourceBudgetPressure`, `AdaptiveQualityPacing`, `UnconstrainedActive`).

---

## 4. Transition Deduplication & Command Minimization

To eliminate state thrashing, player window flickering, IPC storms, and unnecessary player restarts:
- The arbitrator compares `desired_state` against `current_actual_state`.
- `requires_transition` is set to `false` when the backend player already matches the desired state.
- `PolicyArbitrator::reconcile_decision` executes commands against `DisplaySessionCoordinator` only when `requires_transition == true`.

---

## 5. Multi-Monitor Layout Awareness

- **Independent Mode**: Each display session evaluates independent constraints (e.g., fullscreen on Display 2 pauses only Display 2, while Display 1 continues live playback).
- **Span & Clone Modes**: When topologies share player hosts or render spanning buffers, arbitration synchronizes decisions deterministically without command duplication.

