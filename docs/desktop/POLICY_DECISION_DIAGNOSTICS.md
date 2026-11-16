# Policy Decision Diagnostics, Explainability & Troubleshooting

## 1. Overview & Architectural Role
Step 57 of 90 establishes a comprehensive, privacy-preserving, and non-intrusive diagnostic and explainability layer for KinetiWall. Building directly upon the unified policy arbitration system introduced in Step 56, this layer:
- **Observes and explains existing decisions**: It never decides playback outcomes; instead, it observes the authoritative decisions produced by `PolicyArbitrator`.
- **Separates critical state concepts**: Clearly distinguishes between observed system conditions, winning policy outcomes, intended desired state, and actual backend confirmed state.
- **Surfaces truthful diagnostic insights**: Highlights divergence when the backend is reconciling a desired transition, and prevents representing missing or transitioning states as confirmed healthy states.
- **Enforces privacy boundaries**: Strips sensitive file paths, window titles, document contents, and hardware serials; diagnostic records are stored in bounded, in-memory ring buffers with zero remote telemetry.

---

## 2. Core Diagnostic Data Models

### 2.1 Structured Decision & State Discrepancy (`crates/kinetiwall-desktop/src/arbitration.rs`)
```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectivePlaybackDecision {
    pub display_id: DisplayId,
    pub session_id: Option<DisplaySessionId>,
    pub desired_state: DesiredSessionState,
    pub actual_state: DisplaySessionState,
    pub effective_fps_limit: Option<u32>,
    pub active_constraints: Vec<PauseReason>,
    pub explanation_codes: Vec<PolicyExplanationCode>,
    pub primary_explanation: String,
    pub primary_reason_code: String,
    pub is_state_divergent: bool,
    pub requires_transition: bool,
    pub evaluated_at_millis: u64,
}
```

### 2.2 Stable Reason Codes & Human-Readable Explanations
| Explanation Code | Machine-Readable Reason Code | Human-Readable Explanation |
| :--- | :--- | :--- |
| `ManualPauseActive` | `MANUAL_PAUSE_ACTIVE` | "Playback manually paused by user" |
| `SessionTerminated` | `SESSION_TERMINATED` | "Playback session is terminated or stopped" |
| `BackendFailure` | `BACKEND_FAILURE` | "Playback backend encountered an unrecoverable error" |
| `FullscreenActive` | `FULLSCREEN_ACTIVE` | "Suspended due to active foreground fullscreen application" |
| `WorkstationLocked` | `WORKSTATION_LOCKED` | "Suspended because Windows workstation is locked" |
| `DisplayPowerOff` | `DISPLAY_POWER_OFF` | "Suspended because display panel is powered off" |
| `UserInactivityIdle` | `USER_INACTIVITY_IDLE` | "Suspended due to user inactivity (session idle)" |
| `PowerSavingActive` | `POWER_SAVING_ACTIVE` | "Suspended by system power policy on battery/low power" |
| `SystemSuspended` | `SYSTEM_SUSPENDED` | "Suspended because operating system entered sleep/hibernation" |
| `RemoteSessionOptimized` | `REMOTE_SESSION_OPTIMIZED` | "Optimized/suspended for Remote Desktop connection" |
| `ResourceBudgetPressure` | `RESOURCE_BUDGET_PRESSURE` | "Framerate throttled due to high aggregate system load" |
| `AdaptiveQualityPacing` | `ADAPTIVE_QUALITY_PACING` | "Adaptive quality pacing active to maintain performance" |
| `StaticFallbackPresented` | `STATIC_FALLBACK_PRESENTED` | "Displaying preserved static wallpaper fallback" |
| `UnconstrainedActive` | `UNCONSTRAINED_ACTIVE` | "All constraints cleared; playback active" |
| `UnknownStatePreserved` | `UNKNOWN_STATE_PRESERVED` | "Unknown state; conservative baseline maintained" |

---

## 3. Bounded Event Retention & Diagnostics
- **Ring Buffer**: Bounded to `MAX_ARBITRATION_DIAGNOSTICS = 128` events in memory.
- **Transition-Driven**: Diagnostics are updated on policy reconciliation and state transitions, avoiding high-frequency polling allocation loops.
- **Persistence Boundary**: In-memory only; no raw OS logs, battery telemetry, or file dumps are persisted across sessions.

---

## 4. UI Presentation & Troubleshooting Surface
- **`DisplayCard`**: Shows a dedicated policy diagnostics badge indicating:
  - Stable reason code (e.g. `FULLSCREEN_ACTIVE`, `UNCONSTRAINED_ACTIVE`).
  - Clear user-facing explanation.
  - State reconciliation notice (`⚠️ Reconciling transition: desired=..., backend=...`) whenever actual backend state has not yet caught up with desired state.
- **`DisplayStore`**: Maintains `policyDecisions: Record<DisplayId, EffectivePlaybackDecisionDto>` ensuring reactive UI updates when policies arbitrate without full reloads.

