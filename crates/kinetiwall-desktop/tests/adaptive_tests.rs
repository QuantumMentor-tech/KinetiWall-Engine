//! Comprehensive test suite for resource-aware adaptive playback quality,
//! frame pacing, and performance budgets (Step 51).

use std::collections::{HashMap, HashSet};

use kinetiwall_desktop::{
    AdaptiveAction, AdaptiveBackendCapability, AdaptiveDiagnosticEventType,
    AdaptivePlaybackPolicyEvaluator, AdaptiveQualityLevel, AdaptiveQualitySettings, DisplayId,
    DisplaySessionState, MultiMonitorLayoutMode, PauseReason, PlaybackResourceObservation,
    PressureState, ResourceMetricKind, ResourceMetricUnit, ResourceObservationScope,
    ResourceSignalQuality, SustainedPressureDetector, DEFAULT_RECOVERY_DWELL_THRESHOLD_MS,
    DEFAULT_SUSTAINED_PRESSURE_THRESHOLD_MS, MAX_ADAPTIVE_DIAGNOSTICS,
};

// -------------------------------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------------------------------

#[test]
fn test_signal_validation_and_freshness() {
    let now = 100_000;
    let did = DisplayId::new("DISPLAY_PRIMARY");

    // 1. Valid fresh observation
    let valid_obs = PlaybackResourceObservation::new_valid(
        "mpv_cpu_sampler",
        ResourceMetricKind::ProcessCpuPercent,
        45.5,
        ResourceMetricUnit::Percent,
        ResourceObservationScope::DisplaySession(did.clone()),
        now,
        2000,
    );
    assert_eq!(valid_obs.quality, ResourceSignalQuality::Valid);
    assert!(valid_obs.is_fresh(now + 1000));
    // Expired observation
    assert!(!valid_obs.is_fresh(now + 3000));

    // 2. Unavailable measurement placeholder
    let unavail = PlaybackResourceObservation::new_unavailable(
        "mpv_cpu_sampler",
        ResourceMetricKind::ProcessCpuPercent,
        ResourceObservationScope::DisplaySession(did),
        now,
    );
    assert_eq!(unavail.quality, ResourceSignalQuality::Unavailable);
    assert!(!unavail.is_fresh(now));
}

#[test]
fn test_sustained_pressure_detector_spike_rejection() {
    let mut detector = SustainedPressureDetector::new();
    let settings = AdaptiveQualitySettings::default();
    let did = DisplayId::new("DISPLAY_1");
    let now = 10_000;

    // Normal baseline (30% CPU)
    detector.record_sample(
        PlaybackResourceObservation::new_valid(
            "cpu",
            ResourceMetricKind::ProcessCpuPercent,
            30.0,
            ResourceMetricUnit::Percent,
            ResourceObservationScope::DisplaySession(did.clone()),
            now,
            5000,
        ),
        now,
    );
    assert_eq!(detector.evaluate(&settings, now), PressureState::Normal);

    // Single isolated spike to 95% CPU (now + 500ms)
    detector.record_sample(
        PlaybackResourceObservation::new_valid(
            "cpu",
            ResourceMetricKind::ProcessCpuPercent,
            95.0,
            ResourceMetricUnit::Percent,
            ResourceObservationScope::DisplaySession(did.clone()),
            now + 500,
            5000,
        ),
        now + 500,
    );

    // Isolated spike MUST NOT trigger SustainedPressure!
    let evaluated = detector.evaluate(&settings, now + 500);
    assert_eq!(evaluated, PressureState::Normal);
    assert_eq!(detector.current_state(), PressureState::Normal);
}

#[test]
fn test_sustained_pressure_detector_sustained_trigger() {
    let mut detector = SustainedPressureDetector::new();
    let settings = AdaptiveQualitySettings::default();
    let did = DisplayId::new("DISPLAY_1");
    let start_time = 10_000;

    // Sustained high CPU (85% CPU) sampled across 3500ms (threshold is 3000ms)
    for offset in [0, 1000, 2000, 3100] {
        let t = start_time + offset;
        detector.record_sample(
            PlaybackResourceObservation::new_valid(
                "cpu",
                ResourceMetricKind::ProcessCpuPercent,
                85.0,
                ResourceMetricUnit::Percent,
                ResourceObservationScope::DisplaySession(did.clone()),
                t,
                5000,
            ),
            t,
        );
        let st = detector.evaluate(&settings, t);
        if offset < DEFAULT_SUSTAINED_PRESSURE_THRESHOLD_MS {
            assert_eq!(st, PressureState::Normal);
        } else {
            assert_eq!(st, PressureState::SustainedPressure);
        }
    }
}

#[test]
fn test_sustained_pressure_detector_hysteresis_recovery() {
    let mut detector = SustainedPressureDetector::new();
    let settings = AdaptiveQualitySettings::default();
    let did = DisplayId::new("DISPLAY_1");
    let start_time = 10_000;

    // Establish sustained pressure
    for offset in [0, 1000, 2000, 3500] {
        let t = start_time + offset;
        detector.record_sample(
            PlaybackResourceObservation::new_valid(
                "cpu",
                ResourceMetricKind::ProcessCpuPercent,
                85.0,
                ResourceMetricUnit::Percent,
                ResourceObservationScope::DisplaySession(did.clone()),
                t,
                5000,
            ),
            t,
        );
        let _ = detector.evaluate(&settings, t);
    }
    assert_eq!(detector.current_state(), PressureState::SustainedPressure);

    // CPU drops to 25% (below recovery threshold of 40%), but only for 1000ms
    let drop_time = start_time + 4000;
    detector.record_sample(
        PlaybackResourceObservation::new_valid(
            "cpu",
            ResourceMetricKind::ProcessCpuPercent,
            25.0,
            ResourceMetricUnit::Percent,
            ResourceObservationScope::DisplaySession(did.clone()),
            drop_time,
            5000,
        ),
        drop_time,
    );
    // Hysteresis requirement (5000ms) not met yet! Must remain SustainedPressure
    assert_eq!(
        detector.evaluate(&settings, drop_time),
        PressureState::SustainedPressure
    );

    // Sustained low CPU for 5500ms
    let recovery_time = drop_time + DEFAULT_RECOVERY_DWELL_THRESHOLD_MS + 500;
    detector.record_sample(
        PlaybackResourceObservation::new_valid(
            "cpu",
            ResourceMetricKind::ProcessCpuPercent,
            25.0,
            ResourceMetricUnit::Percent,
            ResourceObservationScope::DisplaySession(did),
            recovery_time,
            5000,
        ),
        recovery_time,
    );

    // Now recovery threshold is satisfied
    assert_eq!(
        detector.evaluate(&settings, recovery_time),
        PressureState::Normal
    );
}

#[test]
fn test_sustained_pressure_detector_unknown_handling() {
    let mut detector = SustainedPressureDetector::new();
    let settings = AdaptiveQualitySettings::default();

    // No samples recorded -> evaluated state is Unknown, NEVER treated as zero pressure
    assert_eq!(detector.evaluate(&settings, 5000), PressureState::Unknown);
}

#[test]
fn test_adaptive_evaluator_opt_in_disabled_by_default() {
    // Default settings: enabled == false
    let settings = AdaptiveQualitySettings::default();
    assert!(!settings.enabled);

    let mut evaluator = AdaptivePlaybackPolicyEvaluator::new(settings);
    let did = DisplayId::new("DISPLAY_PRIMARY");
    let cap = AdaptiveBackendCapability::for_video(true);
    let active_reasons = HashSet::new();

    evaluator.register_session(&did, Some(60), 1000);

    // Record high CPU
    evaluator.record_observation(
        PlaybackResourceObservation::new_valid(
            "cpu",
            ResourceMetricKind::ProcessCpuPercent,
            90.0,
            ResourceMetricUnit::Percent,
            ResourceObservationScope::DisplaySession(did.clone()),
            2000,
            5000,
        ),
        2000,
    );

    let decision = evaluator.evaluate_session(
        &did,
        &cap,
        &DisplaySessionState::Active,
        &active_reasons,
        None,
        5000,
    );

    // When disabled: strictly returns NoChange and maintains Normal
    assert_eq!(decision.action, AdaptiveAction::NoChange);
    assert_eq!(decision.target_level, AdaptiveQualityLevel::Normal);
    assert_eq!(decision.effective_fps, Some(60));
}

#[test]
fn test_adaptive_evaluator_downgrades_to_reduced_under_pressure() {
    let mut settings = AdaptiveQualitySettings::default();
    settings.enabled = true; // Opted in
    settings.target_fps_floor = 30;

    let mut evaluator = AdaptivePlaybackPolicyEvaluator::new(settings);
    let did = DisplayId::new("DISPLAY_PRIMARY");
    let cap = AdaptiveBackendCapability::for_video(true);
    let active_reasons = HashSet::new();

    evaluator.register_session(&did, Some(60), 1000);

    // Feed sustained pressure over 3500ms
    for t in [1000, 2000, 3000, 4500] {
        evaluator.record_observation(
            PlaybackResourceObservation::new_valid(
                "cpu",
                ResourceMetricKind::ProcessCpuPercent,
                88.0,
                ResourceMetricUnit::Percent,
                ResourceObservationScope::DisplaySession(did.clone()),
                t,
                5000,
            ),
            t,
        );
    }

    let decision = evaluator.evaluate_session(
        &did,
        &cap,
        &DisplaySessionState::Active,
        &active_reasons,
        None,
        5000,
    );

    assert_eq!(decision.action, AdaptiveAction::ApplyAdjustment);
    assert_eq!(decision.target_level, AdaptiveQualityLevel::Reduced);
    assert_eq!(decision.effective_fps, Some(30));

    // Verify diagnostic was logged
    let diagnostics = evaluator.get_diagnostics();
    assert!(diagnostics.iter().any(|d| d.event_type == AdaptiveDiagnosticEventType::AdjustmentApplied));
}

#[test]
fn test_cooldown_suppresses_rapid_quality_oscillation() {
    let mut settings = AdaptiveQualitySettings::default();
    settings.enabled = true;
    settings.adjustment_cooldown_ms = 4000;

    let mut evaluator = AdaptivePlaybackPolicyEvaluator::new(settings);
    let did = DisplayId::new("DISPLAY_PRIMARY");
    let cap = AdaptiveBackendCapability::for_video(true);
    let active_reasons = HashSet::new();

    evaluator.register_session(&did, Some(60), 1000);

    // Apply first adjustment at t = 5000
    for t in [1000, 2500, 4000, 5000] {
        evaluator.record_observation(
            PlaybackResourceObservation::new_valid(
                "cpu",
                ResourceMetricKind::ProcessCpuPercent,
                90.0,
                ResourceMetricUnit::Percent,
                ResourceObservationScope::DisplaySession(did.clone()),
                t,
                5000,
            ),
            t,
        );
    }
    let dec1 = evaluator.evaluate_session(
        &did,
        &cap,
        &DisplaySessionState::Active,
        &active_reasons,
        None,
        5000,
    );
    assert_eq!(dec1.action, AdaptiveAction::ApplyAdjustment);

    // Attempt second adjustment (power policy requests MinimumEligible) only 1000ms later (t = 6000), while cooldown is 4000ms
    let power_eval = kinetiwall_desktop::PowerPolicyEvaluation {
        should_pause: true,
        prefer_static_fallback: false,
        trigger_reason: Some("Battery Saver Active".into()),
    };
    let dec2 = evaluator.evaluate_session(
        &did,
        &cap,
        &DisplaySessionState::Active,
        &active_reasons,
        Some(&power_eval),
        6000,
    );
    // Cooldown suppresses the adjustment and returns HoldCooldown
    assert_eq!(dec2.action, AdaptiveAction::HoldCooldown);
}

#[test]
fn test_canonical_settings_preserved_during_override() {
    let mut settings = AdaptiveQualitySettings::default();
    settings.enabled = true;
    settings.target_fps_floor = 30;

    let mut evaluator = AdaptivePlaybackPolicyEvaluator::new(settings);
    let did = DisplayId::new("DISPLAY_PRIMARY");
    let cap = AdaptiveBackendCapability::for_video(true);
    let active_reasons = HashSet::new();

    // Canonical FPS is 60
    evaluator.register_session(&did, Some(60), 1000);

    // Apply reduction
    for t in [1000, 2500, 4000, 5000] {
        evaluator.record_observation(
            PlaybackResourceObservation::new_valid(
                "cpu",
                ResourceMetricKind::ProcessCpuPercent,
                85.0,
                ResourceMetricUnit::Percent,
                ResourceObservationScope::DisplaySession(did.clone()),
                t,
                5000,
            ),
            t,
        );
    }
    evaluator.evaluate_session(
        &did,
        &cap,
        &DisplaySessionState::Active,
        &active_reasons,
        None,
        5000,
    );

    let current = evaluator.get_effective_override(&did).unwrap();
    assert_eq!(current.canonical_fps, Some(60));
    assert_eq!(current.effective_fps, Some(30));
    assert_eq!(current.level, AdaptiveQualityLevel::Reduced);

    // Now disable optimization -> restores canonical 60 FPS
    evaluator.update_settings(AdaptiveQualitySettings {
        enabled: false,
        ..Default::default()
    });

    let dec_restored = evaluator.evaluate_session(
        &did,
        &cap,
        &DisplaySessionState::Active,
        &active_reasons,
        None,
        10000,
    );
    assert_eq!(dec_restored.action, AdaptiveAction::RestoreSettings);
    assert_eq!(dec_restored.effective_fps, Some(60));
    assert_eq!(dec_restored.target_level, AdaptiveQualityLevel::Normal);
}

#[test]
fn test_backend_capability_gate_skips_unsupported() {
    let mut settings = AdaptiveQualitySettings::default();
    settings.enabled = true;

    let mut evaluator = AdaptivePlaybackPolicyEvaluator::new(settings);
    let did = DisplayId::new("DISPLAY_1");
    // Unsupported capability
    let cap = AdaptiveBackendCapability {
        supports_framerate_cap: false,
        min_supported_fps: 30,
        max_supported_fps: 60,
        supports_render_cadence_throttling: false,
        supports_visualizer_downsampling: false,
        can_restore_accurately: false,
        is_available: true,
    };
    let active_reasons = HashSet::new();

    evaluator.register_session(&did, Some(60), 1000);

    let decision = evaluator.evaluate_session(
        &did,
        &cap,
        &DisplaySessionState::Active,
        &active_reasons,
        None,
        5000,
    );

    assert_eq!(decision.action, AdaptiveAction::NoChange);
    assert!(decision.reason.contains("does not support"));
}

#[test]
fn test_policy_interlock_respects_manual_pause_and_fullscreen() {
    let mut settings = AdaptiveQualitySettings::default();
    settings.enabled = true;

    let mut evaluator = AdaptivePlaybackPolicyEvaluator::new(settings);
    let did = DisplayId::new("DISPLAY_1");
    let cap = AdaptiveBackendCapability::for_video(true);

    evaluator.register_session(&did, Some(60), 1000);

    // Active pause reason: Fullscreen
    let mut active_reasons = HashSet::new();
    active_reasons.insert(PauseReason::Fullscreen {
        display_id: did.clone(),
    });

    let decision = evaluator.evaluate_session(
        &did,
        &cap,
        &DisplaySessionState::Suspended,
        &active_reasons,
        None,
        5000,
    );

    // Throttling suppressed and session remains paused
    assert_eq!(decision.action, AdaptiveAction::NoChange);
    assert!(decision.reason.contains("suppressed"));
}

#[test]
fn test_static_fallback_sessions_suppress_throttling() {
    let mut settings = AdaptiveQualitySettings::default();
    settings.enabled = true;

    let mut evaluator = AdaptivePlaybackPolicyEvaluator::new(settings);
    let did = DisplayId::new("DISPLAY_1");
    let cap = AdaptiveBackendCapability::for_video(true);
    let active_reasons = HashSet::new();

    evaluator.register_session(&did, Some(60), 1000);

    // Session is in StaticFallback
    let state = DisplaySessionState::StaticFallback {
        fallback_source: "Catalog Poster".into(),
    };

    let decision = evaluator.evaluate_session(
        &did,
        &cap,
        &state,
        &active_reasons,
        None,
        5000,
    );

    // Still image session does not need and does not receive throttling
    assert_eq!(decision.action, AdaptiveAction::NoChange);
    assert!(decision.reason.contains("static fallback"));
}

#[test]
fn test_multi_monitor_clone_mode_synchronized_decision() {
    let mut settings = AdaptiveQualitySettings::default();
    settings.enabled = true;
    settings.target_fps_floor = 30;

    let mut evaluator = AdaptivePlaybackPolicyEvaluator::new(settings);
    let did1 = DisplayId::new("DISPLAY_PRIMARY");
    let did2 = DisplayId::new("DISPLAY_SECONDARY");
    let cap = AdaptiveBackendCapability::for_video(true);

    evaluator.register_session(&did1, Some(60), 1000);
    evaluator.register_session(&did2, Some(60), 1000);

    // Sustained pressure on primary display
    for t in [1000, 2500, 4000, 5000] {
        evaluator.record_observation(
            PlaybackResourceObservation::new_valid(
                "cpu",
                ResourceMetricKind::ProcessCpuPercent,
                88.0,
                ResourceMetricUnit::Percent,
                ResourceObservationScope::DisplaySession(did1.clone()),
                t,
                5000,
            ),
            t,
        );
    }

    let displays = vec![did1.clone(), did2.clone()];
    // Mock session coordinator and empty pause reasons
    let provider = std::sync::Arc::new(kinetiwall_desktop::MockDesktopWindowProvider::new());
    let attach_coord = std::sync::Arc::new(std::sync::Mutex::new(
        kinetiwall_desktop::AttachmentCoordinator::new(provider.clone()),
    ));
    let coord = kinetiwall_desktop::DisplaySessionCoordinator::with_default_factory(
        provider,
        attach_coord,
    );
    let active_reasons = HashMap::new();

    let results = evaluator.evaluate_layout_group(
        &displays,
        MultiMonitorLayoutMode::Clone,
        &cap,
        &coord,
        &active_reasons,
        None,
        5000,
    );

    assert_eq!(results.len(), 2);
    // Secondary display matches primary display target level and FPS
    assert_eq!(
        results.get(&did1).unwrap().target_level,
        results.get(&did2).unwrap().target_level
    );
    assert_eq!(
        results.get(&did1).unwrap().effective_fps,
        results.get(&did2).unwrap().effective_fps
    );
}

#[test]
fn test_multi_monitor_independent_isolation() {
    let mut settings = AdaptiveQualitySettings::default();
    settings.enabled = true;
    settings.target_fps_floor = 30;

    let mut evaluator = AdaptivePlaybackPolicyEvaluator::new(settings);
    let did1 = DisplayId::new("DISPLAY_1");
    let did2 = DisplayId::new("DISPLAY_2");
    let cap = AdaptiveBackendCapability::for_video(true);

    evaluator.register_session(&did1, Some(60), 1000);
    evaluator.register_session(&did2, Some(60), 1000);

    // Sustained pressure recorded ONLY for Display 1
    for t in [1000, 2500, 4000, 5000] {
        evaluator.record_observation(
            PlaybackResourceObservation::new_valid(
                "cpu",
                ResourceMetricKind::ProcessCpuPercent,
                88.0,
                ResourceMetricUnit::Percent,
                ResourceObservationScope::DisplaySession(did1.clone()),
                t,
                5000,
            ),
            t,
        );
    }

    // Display 2 has low CPU recorded
    evaluator.record_observation(
        PlaybackResourceObservation::new_valid(
            "cpu",
            ResourceMetricKind::ProcessCpuPercent,
            20.0,
            ResourceMetricUnit::Percent,
            ResourceObservationScope::DisplaySession(did2.clone()),
            5000,
            5000,
        ),
        5000,
    );

    let displays = vec![did1.clone(), did2.clone()];
    let provider = std::sync::Arc::new(kinetiwall_desktop::MockDesktopWindowProvider::new());
    let attach_coord = std::sync::Arc::new(std::sync::Mutex::new(
        kinetiwall_desktop::AttachmentCoordinator::new(provider.clone()),
    ));
    let coord = kinetiwall_desktop::DisplaySessionCoordinator::with_default_factory(
        provider,
        attach_coord,
    );
    let active_reasons = HashMap::new();

    let results = evaluator.evaluate_layout_group(
        &displays,
        MultiMonitorLayoutMode::Independent,
        &cap,
        &coord,
        &active_reasons,
        None,
        5000,
    );

    assert_eq!(results.len(), 2);
    // Display 1 is downgraded to Reduced due to sustained pressure
    assert_eq!(
        results.get(&did1).unwrap().target_level,
        AdaptiveQualityLevel::Reduced
    );
    assert_eq!(results.get(&did1).unwrap().effective_fps, Some(30));

    // Display 2 remains at Normal because pressure is isolated
    assert_eq!(
        results.get(&did2).unwrap().target_level,
        AdaptiveQualityLevel::Normal
    );
    assert_eq!(results.get(&did2).unwrap().effective_fps, Some(60));
}

#[test]
fn test_bounded_diagnostic_event_history() {
    let settings = AdaptiveQualitySettings::default();
    let mut evaluator = AdaptivePlaybackPolicyEvaluator::new(settings);

    for i in 0..(MAX_ADAPTIVE_DIAGNOSTICS + 20) {
        evaluator.record_diagnostic(
            None,
            AdaptiveDiagnosticEventType::CooldownActive,
            format!("Diagnostic event {}", i),
        );
    }

    assert!(evaluator.get_diagnostics().len() <= MAX_ADAPTIVE_DIAGNOSTICS);
}
