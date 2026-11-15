# KinetiWall Playback Architecture & Lifecycle Contracts (Step 31)

Crate: [`crates/kinetiwall-playback`](../../crates/kinetiwall-playback). It contains contracts and an orchestrator only. It has **no** media decoding, `libmpv`, WebView2, process spawning, IPC transport, or Windows desktop code, and no new external dependencies.

## 1. Repository reality (audit)
- Cargo workspace had one member, `kinetiwall-catalog` (Rust 2021, `cargo 1.99`). `src-ui/` is a TypeScript SSR-testable UI layer. There is no `.git` directory.
- **Not implemented in code** (design docs only, `docs/process_architecture`, `docs/security_and_ipc`): core controller, named-pipe IPC (`CTR-01..12`), process supervisor, player executables, async runtime, media dependencies. Step 31 therefore adds contracts that those later steps will plug into; it does not create a controller, transport or executable.

```text
src-ui (UI)  --(future IPC: CTR-02 ApplyWallpaperToMonitor)-->  Core controller (not yet built)
                                                                       |
                                                             PlaybackOrchestrator  <-- trusted catalog lookup
                                                                       |
                                                             PlayerAdapter (trait)
                                              +------------------------+------------------------+
                                       Video player (future)    Web player (future)      other approved types
                                       isolated process          isolated process
```

## 2. Responsibilities
| Component | Owns | Does not own |
|---|---|---|
| Catalog | wallpaper identity, type, asset reference, availability | playback |
| `PlaybackOrchestrator` | request validation, session ids, state machine, duplicate rules, cancel/stop/shutdown sequencing | decoding, rendering, processes, UI |
| `PlayerAdapter` | backend handles, tasks, IPC links, process handles of *its* sessions; release in `stop`/`cancel`/`shutdown` | session registry, policy |
| Player process (future) | isolated runtime; reports `PlayerReport` | desktop attachment (separate Windows boundary) |
| Frontend | showing returned observed state | controlling decoders |

## 3. Contracts
- **Request** (`PlaybackRequest`): `request_id` (idempotency key), `wallpaper_id`, `wallpaper_type`, `target: DisplayTargetId(u32)`. IDs only, no paths or media bytes.
- **Capability / availability / authorization** are separate: `PlayerCapabilities` (type, cancellation, observed-state reporting) vs `PlayerAvailability` (runtime usable now) vs `PlaybackPolicy` (permitted). Catalog availability (last-known) is checked separately. No fallback between wallpaper types.
- **Errors**: `PlaybackErrorKind` + user-safe `user_message`; the `diagnostic` string is `#[serde(skip)]`, so OS text never reaches the UI contract.
- **Results**: `StartOutcome` (`Accepted | Duplicate | AlreadyActive | Rejected | Failed`), `StopOutcome`, `CancelOutcome`, `EventOutcome`, `ShutdownReport`. `Accepted` means the adapter accepted work; it never means playing.

## 4. Lifecycle
States: `Preparing → Ready → Starting → Playing → Stopping → Stopped`, plus terminal `Failed(kind)` and `Canceled`. `Paused` is deferred (`CTR-04`).
Valid transitions: Preparing→{Ready, Stopping, Failed, Canceled}; Ready→{Starting, Stopping, Failed, Canceled}; Starting→{Playing, Stopping, Failed, Canceled}; Playing→{Stopping, Failed}; Stopping→{Stopped, Failed}; terminal states never change. Everything else is refused (and logged).
- **Desired vs observed**: `SessionSnapshot.desired` (`Playing|Stopped`) is separate from `observed`. `Playing` is observed only after `PlayerReport::Playing` in `Starting`.
- **Failed is never rewritten**: cleaning a failed session (`cleanup_pending`) calls `adapter.stop` and returns `CleanedUpFailedSession`; the state stays `Failed`.
- **Stale events**: sessions get a fresh `PlaybackSessionId` each time; reports for unknown ids or terminal sessions are ignored. Invalid-for-state reports are ignored (`IgnoredInvalidTransition`). An unsolicited `Stopped` while live is `Failed(BackendExited)`.
- **Duplicates**: same `request_id` → `Duplicate` (returns existing session; failed/rejected-after-session requests are also remembered until `prune_finished`). Same wallpaper live on same target → `AlreadyActive`. Different wallpaper on an occupied target → `Rejected(TargetBusy)`; implicit replacement (CTR-02 "terminate then spawn") is deferred. Different targets are independent sessions. A `Failed` session holds its target until cleanup.
- **Cancel**: allowed in Preparing/Ready/Starting. `Accepted` ⇒ adapter guarantees no further results and releases resources; late events are ignored. `Unsupported` ⇒ falls back to a normal stop. Playing sessions are stopped, not canceled.
- **Timeouts**: timers belong to the future supervisor; it calls `handle_timeout(session, phase)`. Result is `Failed(Timeout)` with `outcome_uncertain = true`; reconcile with `stop_session`, never auto-retry. No timeout durations are chosen here.
- **Shutdown**: refuses new work, cancels/stops every live or cleanup-pending session once, then `adapter.shutdown`; failures are reported, nothing is force-killed; repeated calls are no-ops.

## 5. Concurrency
`PlaybackOrchestrator` takes `&mut self`: Rust ownership serializes all state changes. A controller should run it on one actor thread/task fed by channels (or an uncontended `Mutex`). Adapter methods must be bounded and non-blocking; confirmation is asynchronous via `PlayerReport`. Tested: eight threads submitting the same request create exactly one session.

## 6. Security / IPC requirements for later steps
Player reports are untrusted: the supervisor must validate schema, session id and expected state before calling `handle_event` (the orchestrator re-validates identity and transition). Reports cannot start processes or native commands. Reuse the Step 8 envelope/versioning/size limits; no new transport. No telemetry; logs use ids, kinds and states, not paths (`tracing`, no subscriber installed here).

## 7. Deferred (not implemented)
libmpv / WebView2 adapters; player-process supervisor and IPC wiring; timer ownership and timeout values; pause/resume (`CTR-04`); display validation and multi-monitor assignment; Progman/WorkerW attachment (render-ready vs attached stay separate concepts); process restart and Explorer-restart recovery; replacing a session on an occupied target; real policy implementation (`AllowAllPolicy` is a placeholder); persisted playback state. Mock tests prove contract behavior only, not real playback.

