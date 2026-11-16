//! Comprehensive test suite for dynamic resource budgeting, multi-session load balancing,
//! and optimization arbitration (Step 52).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use kinetiwall_desktop::{
    AdaptiveBackendCapability, AdaptiveQualityLevel, AggregateBudgetState,
    ApplicationResourceBudget, ArbitrationAction, AttachmentCoordinator,
    BudgetDiagnosticEventType, DisplayId, DisplaySessionCoordinator, DisplaySessionId,
    MockDesktopWindowProvider, MultiMonitorLayoutMode, PauseReason,
    ResourceBudgetArbiter, TrackedResourceId, MAX_BUDGET_DIAGNOSTICS,
};

fn create_mock_coordinator() -> DisplaySessionCoordinator {
    let provider = Arc::new(MockDesktopWindowProvider::new());
    let attach_coord = Arc::new(Mutex::new(AttachmentCoordinator::new(provider.clone())));
    DisplaySessionCoordinator::with_default_factory(provider, attach_coord)
}

#[test]
fn test_inventory_registration_and_accounting() {
    let mut arbiter = ResourceBudgetArbiter::new(ApplicationResourceBudget::default());
    let did = DisplayId::new("DISPLAY_1");
    let sid = DisplaySessionId::new();
    let cap = AdaptiveBackendCapability::for_video(true);
    let now = 10_000;

    arbiter.register_dedicated_session(
        did.clone(),
        sid,
        1,
        cap,
        Some(60),
        Some(1234),
        now,
    );

    let res_id = TrackedResourceId::for_session(&sid);
    let res = arbiter.get_resource(&res_id);
    assert!(res.is_some());
    let res = res.unwrap();
    assert_eq!(res.session_generation, 1);
    assert_eq!(res.effective_override.canonical_fps, Some(60));
    assert_eq!(res.effective_override.level, AdaptiveQualityLevel::Normal);

    // Unregister
    arbiter.unregister_display(&did);
    assert!(arbiter.get_resource(&res_id).is_none());
}

#[test]
fn test_shared_group_accounting_no_double_counting() {
    let mut arbiter = ResourceBudgetArbiter::new(ApplicationResourceBudget::default());
    let did1 = DisplayId::new("DISPLAY_1");
    let did2 = DisplayId::new("DISPLAY_2");
    let cap = AdaptiveBackendCapability::for_video(true);
    let now = 10_000;

    // Register a Span group sharing PID 5000 across two monitors
    arbiter.register_shared_group(
        "span_desktop",
        MultiMonitorLayoutMode::Span,
        did1.clone(),
        vec![did2.clone()],
        1,
        cap,
        Some(60),
        Some(5000),
        now,
    );

    // Record metrics for primary display (40% CPU)
    arbiter.record_process_metrics(&did1, 40.0, Some(250_000_000), now);

    let workload = arbiter.compute_workload(now);
    assert_eq!(workload.total_sessions_count, 1);
    assert_eq!(workload.measured_sessions_count, 1);
    assert_eq!(workload.unknown_sessions_count, 0);
    assert!(workload.is_coverage_complete);
    // Verified CPU must be counted ONCE (40.0%), never doubled (80.0%)!
    assert_eq!(workload.aggregate_cpu_percent, 40.0);

    // Unregistering display 2 should not delete the shared group because display 1 still references it
    arbiter.unregister_display(&did2);
    let shared_id = TrackedResourceId::for_shared_group("span_desktop");
    assert!(arbiter.get_resource(&shared_id).is_some());

    // Unregistering display 1 removes the final reference
    arbiter.unregister_display(&did1);
    assert!(arbiter.get_resource(&shared_id).is_none());
}

#[test]
fn test_unknown_and_stale_measurement_handling() {
    let mut arbiter = ResourceBudgetArbiter::new(ApplicationResourceBudget::default());
    let did = DisplayId::new("DISPLAY_1");
    let sid = DisplaySessionId::new();
    let cap = AdaptiveBackendCapability::for_video(true);
    let now = 10_000;

    arbiter.register_dedicated_session(did.clone(), sid, 1, cap, Some(60), None, now);

    // Before any observations: workload coverage is incomplete and unknown count is 1
    let workload = arbiter.compute_workload(now);
    assert_eq!(workload.total_sessions_count, 1);
    assert_eq!(workload.measured_sessions_count, 0);
    assert_eq!(workload.unknown_sessions_count, 1);
    assert!(!workload.is_coverage_complete);
    assert_eq!(workload.aggregate_cpu_percent, 0.0);

    // Enable budget
    let mut budget = ApplicationResourceBudget::default();
    budget.enabled = true;
    arbiter.update_budget(budget);

    let coord = create_mock_coordinator();
    let pause_reasons = HashMap::new();

    // With 0 verified measurements, arbitration defers due to incomplete coverage
    let actions = arbiter.arbitrate(&coord, &pause_reasons, None, now);
    assert_eq!(arbiter.current_state(), AggregateBudgetState::CoverageIncomplete);
    assert!(matches!(actions.first(), Some(ArbitrationAction::CoverageIncompleteDefer { .. })));
}

#[test]
fn test_budget_opt_in_disabled_by_default() {
    let budget = ApplicationResourceBudget::default();
    assert!(!budget.enabled, "Budget management must be opt-in (disabled) by default!");

    let mut arbiter = ResourceBudgetArbiter::new(budget);
    assert_eq!(arbiter.current_state(), AggregateBudgetState::Disabled);

    let coord = create_mock_coordinator();
    let pause_reasons = HashMap::new();
    let actions = arbiter.arbitrate(&coord, &pause_reasons, None, 10_000);

    assert_eq!(actions.len(), 1);
    assert!(matches!(actions.first(), Some(ArbitrationAction::NoChange { .. })));
}

#[test]
fn test_sustained_pressure_arbitrates_reduction() {
    let mut budget = ApplicationResourceBudget::default();
    budget.enabled = true;
    budget.pressure_threshold_cpu_percent = 70.0;
    budget.sustained_pressure_threshold_ms = 3000;
    let mut arbiter = ResourceBudgetArbiter::new(budget);

    let did = DisplayId::new("DISPLAY_1");
    let sid = DisplaySessionId::new();
    let cap = AdaptiveBackendCapability::for_video(true);
    let start_time = 10_000;

    arbiter.register_dedicated_session(did.clone(), sid, 1, cap, Some(60), Some(2000), start_time);

    let coord = create_mock_coordinator();
    let pause_reasons = HashMap::new();

    // High CPU (85%) recorded continuously over 3500ms
    for t in [start_time, start_time + 1000, start_time + 2000, start_time + 3200] {
        arbiter.record_process_metrics(&did, 85.0, Some(300_000_000), t);
        let actions = arbiter.arbitrate(&coord, &pause_reasons, None, t);

        if t < start_time + 3000 {
            // Transient duration: state is ApproachingBudget, no reduction yet
            assert_eq!(arbiter.current_state(), AggregateBudgetState::ApproachingBudget);
            assert!(matches!(actions.first(), Some(ArbitrationAction::NoChange { .. })));
        } else {
            // Sustained threshold met (3200ms >= 3000ms): emits reduction!
            assert_eq!(arbiter.current_state(), AggregateBudgetState::OptimizationActive);
            assert_eq!(actions.len(), 1);
            match &actions[0] {
                ArbitrationAction::ApplySessionReduction {
                    display_id,
                    target_level,
                    target_fps,
                    ..
                } => {
                    assert_eq!(display_id, &did);
                    assert_eq!(*target_level, AdaptiveQualityLevel::Reduced);
                    assert_eq!(*target_fps, Some(30));
                }
                other => panic!("Expected ApplySessionReduction, got {:?}", other),
            }
        }
    }
}

#[test]
fn test_spike_rejection_does_not_arbitrate() {
    let mut budget = ApplicationResourceBudget::default();
    budget.enabled = true;
    budget.pressure_threshold_cpu_percent = 70.0;
    budget.sustained_pressure_threshold_ms = 3000;
    let mut arbiter = ResourceBudgetArbiter::new(budget);

    let did = DisplayId::new("DISPLAY_1");
    let sid = DisplaySessionId::new();
    let cap = AdaptiveBackendCapability::for_video(true);
    let start_time = 10_000;

    arbiter.register_dedicated_session(did.clone(), sid, 1, cap, Some(60), Some(2000), start_time);
    let coord = create_mock_coordinator();
    let pause_reasons = HashMap::new();

    // Single 500ms spike to 95% CPU
    arbiter.record_process_metrics(&did, 95.0, Some(300_000_000), start_time);
    let actions = arbiter.arbitrate(&coord, &pause_reasons, None, start_time + 500);

    assert_eq!(arbiter.current_state(), AggregateBudgetState::ApproachingBudget);
    assert!(matches!(actions.first(), Some(ArbitrationAction::NoChange { .. })));
}

#[test]
fn test_heaviest_session_prioritized_first() {
    let mut budget = ApplicationResourceBudget::default();
    budget.enabled = true;
    budget.target_aggregate_cpu_percent = 50.0;
    budget.pressure_threshold_cpu_percent = 70.0;
    budget.sustained_pressure_threshold_ms = 1000;
    budget.max_simultaneous_adjustments = 1; // Adjust only one session per cycle
    let mut arbiter = ResourceBudgetArbiter::new(budget);

    let did1 = DisplayId::new("DISPLAY_LIGHT");
    let sid1 = DisplaySessionId::new();
    let did2 = DisplayId::new("DISPLAY_HEAVY");
    let sid2 = DisplaySessionId::new();

    let cap = AdaptiveBackendCapability::for_video(true);
    let now = 10_000;

    arbiter.register_dedicated_session(did1.clone(), sid1, 1, cap.clone(), Some(60), Some(1001), now);
    arbiter.register_dedicated_session(did2.clone(), sid2, 1, cap, Some(60), Some(1002), now);

    // Display 1 uses 15% CPU, Display 2 uses 65% CPU -> Aggregate = 80% CPU (> 70.0% threshold)
    arbiter.record_process_metrics(&did1, 15.0, None, now);
    arbiter.record_process_metrics(&did2, 65.0, None, now);

    let coord = create_mock_coordinator();
    let pause_reasons = HashMap::new();

    let _ = arbiter.arbitrate(&coord, &pause_reasons, None, now);

    arbiter.record_process_metrics(&did1, 15.0, None, now + 1200);
    arbiter.record_process_metrics(&did2, 65.0, None, now + 1200);
    let actions = arbiter.arbitrate(&coord, &pause_reasons, None, now + 1200);

    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ArbitrationAction::ApplySessionReduction { display_id, target_level, .. } => {
            // Heaviest consumer (DISPLAY_HEAVY) MUST be prioritized for reduction!
            assert_eq!(display_id, &did2);
            assert_eq!(*target_level, AdaptiveQualityLevel::Reduced);
        }
        other => panic!("Expected ApplySessionReduction for DISPLAY_HEAVY, got {:?}", other),
    }

    // DISPLAY_LIGHT must remain untouched at Normal
    let res_light = arbiter.get_resource(&TrackedResourceId::for_session(&sid1)).unwrap();
    assert_eq!(res_light.effective_override.level, AdaptiveQualityLevel::Normal);
}

#[test]
fn test_shared_span_emits_shared_reduction() {
    let mut budget = ApplicationResourceBudget::default();
    budget.enabled = true;
    budget.pressure_threshold_cpu_percent = 70.0;
    budget.sustained_pressure_threshold_ms = 1000;
    let mut arbiter = ResourceBudgetArbiter::new(budget);

    let did1 = DisplayId::new("DISPLAY_1");
    let did2 = DisplayId::new("DISPLAY_2");
    let cap = AdaptiveBackendCapability::for_video(true);
    let now = 10_000;

    arbiter.register_shared_group(
        "span_desktop",
        MultiMonitorLayoutMode::Span,
        did1.clone(),
        vec![did2.clone()],
        1,
        cap,
        Some(60),
        Some(3000),
        now,
    );

    arbiter.record_process_metrics(&did1, 85.0, None, now);
    let coord = create_mock_coordinator();
    let pause_reasons = HashMap::new();

    let _ = arbiter.arbitrate(&coord, &pause_reasons, None, now);
    let actions = arbiter.arbitrate(&coord, &pause_reasons, None, now + 1500);

    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ArbitrationAction::ApplySharedReduction {
            primary_display_id,
            affected_displays,
            target_level,
            ..
        } => {
            assert_eq!(primary_display_id, &did1);
            assert_eq!(affected_displays, &vec![did2.clone()]);
            assert_eq!(*target_level, AdaptiveQualityLevel::Reduced);
        }
        other => panic!("Expected ApplySharedReduction, got {:?}", other),
    }
}

#[test]
fn test_recovery_dwell_restores_canonical_settings() {
    let mut budget = ApplicationResourceBudget::default();
    budget.enabled = true;
    budget.target_aggregate_cpu_percent = 50.0;
    budget.pressure_threshold_cpu_percent = 70.0;
    budget.recovery_threshold_cpu_percent = 40.0;
    budget.sustained_pressure_threshold_ms = 1000;
    budget.recovery_dwell_threshold_ms = 5000;
    budget.adjustment_cooldown_ms = 1000;
    let mut arbiter = ResourceBudgetArbiter::new(budget);

    let did = DisplayId::new("DISPLAY_1");
    let sid = DisplaySessionId::new();
    let cap = AdaptiveBackendCapability::for_video(true);
    let start_time = 10_000;

    arbiter.register_dedicated_session(did.clone(), sid, 1, cap, Some(60), Some(2000), start_time);
    let coord = create_mock_coordinator();
    let pause_reasons = HashMap::new();

    // Trigger reduction at t = 11_500
    arbiter.record_process_metrics(&did, 85.0, None, start_time);
    let _ = arbiter.arbitrate(&coord, &pause_reasons, None, start_time);
    arbiter.record_process_metrics(&did, 85.0, None, start_time + 1500);
    let actions = arbiter.arbitrate(&coord, &pause_reasons, None, start_time + 1500);
    assert!(matches!(actions.first(), Some(ArbitrationAction::ApplySessionReduction { .. })));

    // CPU drops to low recovery level (25.0%) at t = 15_000 (after cooldown)
    let low_time = 15_000;
    arbiter.record_process_metrics(&did, 25.0, None, low_time);
    let _ = arbiter.arbitrate(&coord, &pause_reasons, None, low_time);

    // During recovery dwell (e.g. at +2000ms, elapsed 2000 < 5000)
    arbiter.record_process_metrics(&did, 25.0, None, low_time + 2000);
    let actions_dwell = arbiter.arbitrate(&coord, &pause_reasons, None, low_time + 2000);
    assert_eq!(arbiter.current_state(), AggregateBudgetState::Recovering);
    assert!(matches!(actions_dwell.first(), Some(ArbitrationAction::NoChange { .. })));

    // After full dwell completion (at +5500ms, elapsed 5500 >= 5000)
    arbiter.record_process_metrics(&did, 25.0, None, low_time + 5500);
    let actions_recovered = arbiter.arbitrate(&coord, &pause_reasons, None, low_time + 5500);
    assert_eq!(actions_recovered.len(), 1);
    match &actions_recovered[0] {
        ArbitrationAction::RestoreSession { display_id, canonical_fps, .. } => {
            assert_eq!(display_id, &did);
            assert_eq!(*canonical_fps, Some(60));
        }
        other => panic!("Expected RestoreSession, got {:?}", other),
    }
}

#[test]
fn test_cooldown_prevents_rapid_oscillation() {
    let mut budget = ApplicationResourceBudget::default();
    budget.enabled = true;
    budget.pressure_threshold_cpu_percent = 70.0;
    budget.sustained_pressure_threshold_ms = 1000;
    budget.adjustment_cooldown_ms = 4000;
    let mut arbiter = ResourceBudgetArbiter::new(budget);

    let did = DisplayId::new("DISPLAY_1");
    let sid = DisplaySessionId::new();
    let cap = AdaptiveBackendCapability::for_video(true);
    let now = 10_000;

    arbiter.register_dedicated_session(did.clone(), sid, 1, cap, Some(60), Some(2000), now);
    let coord = create_mock_coordinator();
    let pause_reasons = HashMap::new();

    // Trigger reduction at now + 1500
    arbiter.record_process_metrics(&did, 85.0, None, now);
    let _ = arbiter.arbitrate(&coord, &pause_reasons, None, now);
    let actions = arbiter.arbitrate(&coord, &pause_reasons, None, now + 1500);
    assert!(matches!(actions.first(), Some(ArbitrationAction::ApplySessionReduction { .. })));

    // Immediate subsequent arbitration call 1000ms later (cooldown is 4000ms)
    let actions_cd = arbiter.arbitrate(&coord, &pause_reasons, None, now + 2500);
    assert_eq!(actions_cd.len(), 1);
    assert!(matches!(actions_cd.first(), Some(ArbitrationAction::HoldCooldown { .. })));
}

#[test]
fn test_policy_interlocks_respect_manual_pause_and_fallback() {
    let mut budget = ApplicationResourceBudget::default();
    budget.enabled = true;
    budget.pressure_threshold_cpu_percent = 70.0;
    budget.sustained_pressure_threshold_ms = 1000;
    let mut arbiter = ResourceBudgetArbiter::new(budget);

    let did = DisplayId::new("DISPLAY_1");
    let sid = DisplaySessionId::new();
    let cap = AdaptiveBackendCapability::for_video(true);
    let now = 10_000;

    arbiter.register_dedicated_session(did.clone(), sid, 1, cap, Some(60), Some(2000), now);
    let coord = create_mock_coordinator();

    // Manual pause active for DISPLAY_1
    let mut pause_reasons = HashMap::new();
    let mut reasons_set = HashSet::new();
    reasons_set.insert(PauseReason::UserManual);
    pause_reasons.insert(did.clone(), reasons_set);

    arbiter.record_process_metrics(&did, 85.0, None, now);
    let _ = arbiter.arbitrate(&coord, &pause_reasons, None, now);
    let actions = arbiter.arbitrate(&coord, &pause_reasons, None, now + 1500);

    // Paused session cannot be targeted for optimization! Produces TargetInfeasible.
    assert_eq!(arbiter.current_state(), AggregateBudgetState::TargetInfeasible);
    assert!(matches!(actions.first(), Some(ArbitrationAction::TargetInfeasible { .. })));
}

#[test]
fn test_target_infeasible_when_all_at_minimum_floor() {
    let mut budget = ApplicationResourceBudget::default();
    budget.enabled = true;
    budget.target_aggregate_cpu_percent = 50.0;
    budget.pressure_threshold_cpu_percent = 70.0;
    budget.sustained_pressure_threshold_ms = 1000;
    budget.adjustment_cooldown_ms = 1000;
    let mut arbiter = ResourceBudgetArbiter::new(budget);

    let did = DisplayId::new("DISPLAY_1");
    let sid = DisplaySessionId::new();
    let cap = AdaptiveBackendCapability::for_video(true);
    let now = 10_000;

    arbiter.register_dedicated_session(did.clone(), sid, 1, cap, Some(60), Some(2000), now);
    let coord = create_mock_coordinator();
    let pause_reasons = HashMap::new();

    // First reduction: to Reduced
    arbiter.record_process_metrics(&did, 85.0, None, now);
    let _ = arbiter.arbitrate(&coord, &pause_reasons, None, now);
    arbiter.record_process_metrics(&did, 85.0, None, now + 1500);
    let act1 = arbiter.arbitrate(&coord, &pause_reasons, None, now + 1500);
    assert!(matches!(act1.first(), Some(ArbitrationAction::ApplySessionReduction { target_level: AdaptiveQualityLevel::Reduced, .. })));

    // Second reduction to MinimumEligible after cooldown
    arbiter.record_process_metrics(&did, 85.0, None, now + 3500);
    let act2 = arbiter.arbitrate(&coord, &pause_reasons, None, now + 3500);
    assert!(matches!(act2.first(), Some(ArbitrationAction::ApplySessionReduction { target_level: AdaptiveQualityLevel::MinimumEligible, .. })));

    // Third evaluation under pressure: already at floor -> TargetInfeasible!
    arbiter.record_process_metrics(&did, 85.0, None, now + 5500);
    let act3 = arbiter.arbitrate(&coord, &pause_reasons, None, now + 5500);
    assert_eq!(arbiter.current_state(), AggregateBudgetState::TargetInfeasible);
    assert!(matches!(act3.first(), Some(ArbitrationAction::TargetInfeasible { .. })));
}

#[test]
fn test_unsupported_backend_skipped() {
    let mut budget = ApplicationResourceBudget::default();
    budget.enabled = true;
    budget.pressure_threshold_cpu_percent = 70.0;
    budget.sustained_pressure_threshold_ms = 1000;
    let mut arbiter = ResourceBudgetArbiter::new(budget);

    let did = DisplayId::new("DISPLAY_UNSUPPORTED");
    let sid = DisplaySessionId::new();
    // Capability with NO framerate cap and NO render cadence throttling
    let mut cap = AdaptiveBackendCapability::for_video(true);
    cap.supports_framerate_cap = false;
    cap.supports_render_cadence_throttling = false;
    let now = 10_000;

    arbiter.register_dedicated_session(did.clone(), sid, 1, cap, Some(60), Some(2000), now);
    let coord = create_mock_coordinator();
    let pause_reasons = HashMap::new();

    arbiter.record_process_metrics(&did, 85.0, None, now);
    let _ = arbiter.arbitrate(&coord, &pause_reasons, None, now);
    let actions = arbiter.arbitrate(&coord, &pause_reasons, None, now + 1500);

    assert_eq!(arbiter.current_state(), AggregateBudgetState::TargetInfeasible);
    assert!(matches!(actions.first(), Some(ArbitrationAction::TargetInfeasible { .. })));
}

#[test]
fn test_disabling_budget_restores_all_active_overrides() {
    let mut budget = ApplicationResourceBudget::default();
    budget.enabled = true;
    budget.pressure_threshold_cpu_percent = 70.0;
    budget.sustained_pressure_threshold_ms = 1000;
    let mut arbiter = ResourceBudgetArbiter::new(budget);

    let did = DisplayId::new("DISPLAY_1");
    let sid = DisplaySessionId::new();
    let cap = AdaptiveBackendCapability::for_video(true);
    let now = 10_000;

    arbiter.register_dedicated_session(did.clone(), sid, 1, cap, Some(60), Some(2000), now);
    let coord = create_mock_coordinator();
    let pause_reasons = HashMap::new();

    arbiter.record_process_metrics(&did, 85.0, None, now);
    let _ = arbiter.arbitrate(&coord, &pause_reasons, None, now);
    let _ = arbiter.arbitrate(&coord, &pause_reasons, None, now + 1500);

    // Disable budget
    let mut disabled_budget = ApplicationResourceBudget::default();
    disabled_budget.enabled = false;
    arbiter.update_budget(disabled_budget);

    let actions = arbiter.arbitrate(&coord, &pause_reasons, None, now + 2000);
    assert_eq!(arbiter.current_state(), AggregateBudgetState::Disabled);
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ArbitrationAction::RestoreSession { display_id, canonical_fps, .. } => {
            assert_eq!(display_id, &did);
            assert_eq!(*canonical_fps, Some(60));
        }
        other => panic!("Expected RestoreSession, got {:?}", other),
    }
}

#[test]
fn test_bounded_diagnostic_event_history() {
    let mut arbiter = ResourceBudgetArbiter::new(ApplicationResourceBudget::default());

    for i in 0..(MAX_BUDGET_DIAGNOSTICS + 30) {
        arbiter.record_diagnostic(
            BudgetDiagnosticEventType::OptimizationArbitrated,
            format!("Diagnostic event {}", i),
            Some(50.0),
            Some(1.0),
        );
    }

    let diagnostics = arbiter.get_diagnostics();
    assert_eq!(diagnostics.len(), MAX_BUDGET_DIAGNOSTICS);
    assert!(diagnostics.last().unwrap().message.contains("Diagnostic event 157"));
}
