# Policy Integration Test Harness, Fault Injection & Recovery Validation

## 1. Overview & Objectives
Step 58 of 90 establishes an end-to-end, deterministic policy integration test harness and fault-injection test suite in KinetiWall. Building directly upon:
- Centralized policy arbitration from Step 56 (`PolicyArbitrator`).
- Diagnostic explainability from Step 57 (`EffectivePlaybackDecision` & `PolicyExplanationCode`).
- Session coordination and multi-monitor management from Steps 42–47.

This integration layer exercises the full decision-to-execution pipeline and validates fault injection and recovery behavior without depending on live Windows state, real displays, or physical power transitions.

---

## 2. Test Harness Architecture (`crates/kinetiwall-desktop/tests/policy_integration_tests.rs`)

### 2.1 Component Structure
`PolicyIntegrationHarness` encapsulates the entire production stack using deterministic test doubles:
- **`MockDesktopWindowProvider` & `MockDesktopDisplayProvider`**: Provide synthetic dual-monitor topology (`DISPLAY1` at `0,0,1920,1080` and `DISPLAY2` at `1920,0,3840,1080`).
- **`TestCatalog`**: In-memory `WallpaperCatalogRepository` managing test wallpapers without touching disk media.
- **`AttachmentCoordinator` & `DisplaySessionCoordinator`**: The exact production session coordinators managing desktop window hierarchy and session life cycles.
- **`SmartPlaybackPolicyEvaluator`**: The actual production evaluator running fullscreen, lock/unlock, display power, and user inactivity awareness policies.

### 2.2 Production Seam: Fault Injection
Added `mark_session_failed(&mut self, display_id: &DisplayId, reason: &str)` to `DisplaySessionCoordinator`:
- Enables controlled injection of player crash or backend failure events (e.g. `DXGI_ERROR_DEVICE_RESET` or out-of-memory).
- Transitions the target session to `DisplaySessionState::Failed { reason }`.
- Preserves monitor isolation: failure on Display 1 does not affect Display 2.

---

## 3. Verified Scenarios & Test Suite

| Test Name | Validated Behavior | Result |
| :--- | :--- | :--- |
| `test_harness_manual_pause_wins_over_all_automatic_clears` | User manual pause overrides automatic policies (fullscreen, lock). All automatic triggers can clear, but manual pause preserves `Suspended` state until user explicit resume. | **Passed** |
| `test_fault_injection_backend_failure_isolated_to_failing_display` | Backend crash simulated on Display 1. Display 1 transitions to `Failed`, while Display 2 remains unharmed and `Active`. | **Passed** |
| `test_recovery_transition_to_static_fallback_and_recovery_to_active` | Low power/failure causes transition to `StaticFallback`. Player subsequently recovers and cleanly transitions back to `Active` unconstrained rendering. | **Passed** |
| `test_obsolete_session_events_cannot_corrupt_reconfigured_session` | When a session is reconfigured or replaced, late asynchronous callbacks from the obsolete session ID cannot mutate the active session. | **Passed** |
| `test_multi_monitor_span_vs_independent_isolation` | Fullscreen application on Display 2 suspends only Display 2 under `Independent` mode, while Display 1 continues unconstrained live playback. | **Passed** |
| `test_session_lifecycle_teardown_and_cleanup` | Orderly session stop releases surfaces and marks session `Stopped`, reflecting accurately in arbitration diagnostics. | **Passed** |

---

## 4. Verification Commands

```bash
# Run the focused policy integration test suite
cargo test -p kinetiwall-desktop --test policy_integration_tests

# Run arbitration unit and diagnostic tests
cargo test -p kinetiwall-desktop --test arbitration_tests

# Run the entire workspace test suite
cargo test --workspace

# Run frontend tests
npm test
```

