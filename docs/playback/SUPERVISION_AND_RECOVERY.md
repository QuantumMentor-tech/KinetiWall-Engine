# Unified Player Orchestration, Health Monitoring & Crash Recovery (Step 35)

## Overview

Step 35 establishes the centralized orchestration and process supervision layer for KinetiWall's live wallpaper playback subsystem. Built on top of the foundation created in Steps 31 (Player Abstractions), 32 (libmpv Video Player), 33 (WebView2 Web Player), and 34 (WASAPI Audio Capture), this supervisor coordinates backend runtimes, monitors process health, isolates and recovers from runtime failures, and coordinates audio-reactive pipelines.

## Architectural Principles

1. **Strict Separation of Desired State vs. Observed State:**
   - The supervisor explicitly tracks what the user/application requested (`DesiredState::Playing` vs `DesiredState::Stopped`) alongside the observed backend lifecycle state (`SessionState`).
   - If a backend process exits or crashes when `DesiredState` is `Stopped`, the event is classified as an expected shutdown and **never** triggers an automatic restart loop.

2. **Single-Authority Recovery:**
   - Recovery decisions are owned solely by `PlayerSupervisor`. Individual player adapters (`VideoPlayerAdapter`, `WebPlayerAdapter`) and child worker binaries do not make autonomous restart decisions.

3. **Instance Identity & Generation Tokens (`PlayerInstanceId`):**
   - Every active playback instance receives a unique `PlayerInstanceId` (UUID v4 token).
   - Late or queued IPC reports and exit notifications from dead/obsolete child instances carrying an outdated token are cleanly discarded without affecting newly spawned replacement instances.

4. **Failure Classification & Circuit-Breaker Backoff:**
   - Failures are classified into typed categories (`FailureClassification`):
     - `ProcessTerminated`: Unexpected exit code or termination. (Retryable)
     - `StartupTimeout` / `ShutdownTimeout`: Operation deadlines exceeded. (Retryable / Non-retryable)
     - `IpcDisconnection` / `BackendFailure`: Communication dropped. (Retryable)
     - `InvalidAsset`: File missing, corrupted, or unsupported. (Non-retryable)
     - `Unrecoverable`: Explicitly marked fatal failure. (Non-retryable)
   - The `RecoveryBudget` uses a rolling time window (`window_duration`, default 60s) and exponential backoff (`initial_backoff`, `max_backoff`, default 2.0x factor) capped at `max_retries` (default 3). If retries exceed the budget within the window, the circuit breaker trips, transitioning the session to `PlayerHealthStatus::Dead` and preventing crash loops.

5. **Audio-Reactive Pipeline Lifecycle Coordination:**
   - The `AudioCoordinator` trait decouples the playback supervisor from `kinetiwall-audio`.
   - When a web wallpaper requesting audio reactivity is started, the supervisor invokes `start_reactive_audio()`.
   - On stop, failure, or recovery, the supervisor invokes `stop_reactive_audio()`, guaranteeing that WASAPI loopback capture threads and audio buffers do not leak or run orphan.

## Component Architecture

```
+-----------------------------------------------------------------------+
|                           PlayerSupervisor                            |
|                                                                       |
|  - Tracks SupervisedTarget map: TargetId -> { desired, health, ... } |
|  - Owns PlaybackOrchestrator & Target Allocation                      |
|  - Evaluates RecoveryPolicy & RecoveryBudget                          |
|  - Triggers AudioCoordinator start/stop                               |
+-----------------------------------+-----------------------------------+
                                    |
                                    v
+-----------------------------------------------------------------------+
|                         PlaybackOrchestrator                          |
|                                                                       |
|  - Manages Session State Machine (Preparing, Playing, Stopped, etc.)  |
|  - Dispatches commands to PlayerAdapter trait implementations         |
|  - Polls adapter reports via poll_reports() & checks liveness          |
+-------------------+-------------------------------+-------------------+
                    |                               |
                    v                               v
+---------------------------------------+ +-----------------------------+
|          VideoPlayerAdapter           | |      WebPlayerAdapter       |
|                                       | |                             |
|  - Spawns kinetiwall-player-video     | | - Spawns kinetiwall-player- |
|  - Monitors child.try_wait()          | |   web                       |
|  - Emits BackendExited on death       | | - Monitors child.try_wait() |
|                                       | | - Emits BackendExited       |
+---------------------------------------+ +-----------------------------+
```

## Health State Transitions

- `PlayerHealthStatus::Healthy`: Process alive, responding, states consistent.
- `PlayerHealthStatus::Degraded`: Transient non-fatal anomalies (e.g. late report).
- `PlayerHealthStatus::Unresponsive`: Operation timed out, reconciliation pending.
- `PlayerHealthStatus::Dead`: Process exited unexpectedly and recovery budget exhausted or failure non-retryable.

## Verification & Test Coverage

All contracts and supervisor behaviors are verified with deterministic integration tests in `crates/kinetiwall-playback/tests/supervision_tests.rs`:
- `test_supervisor_start_and_normal_observation`: Validates startup, target registration, generation token assignment, and initial health.
- `test_supervisor_stop_target_and_cleanup`: Validates clean shutdown, target deallocation, and removal of supervision state.
- `test_supervisor_detects_process_crash_and_executes_recovery`: Simulates unexpected process crash, asserts automatic replacement instance launch with a new token and incremented retry count.
- `test_supervisor_recovery_circuit_breaker_budget_exhaustion`: Exceeds maximum retry budget (3 crashes in window), verifies circuit breaker trips to `Dead` without further restarts.
- `test_supervisor_non_retryable_failure_does_not_restart`: Injects unrecoverable error (`InvalidAsset`), verifies zero restart attempts.
- `test_supervisor_user_stop_prevents_recovery`: Stops player explicitly while child process terminates; verifies that zero recovery attempts occur.
- `test_audio_reactivity_lifecycle_coordination`: Asserts that `AudioCoordinator` starts loopback capture on play and halts capture on stop or recovery.
- `test_supervisor_shutdown_is_clean_and_idempotent`: Verifies multiple invocations of `shutdown()` are safe and leave no orphan state.

