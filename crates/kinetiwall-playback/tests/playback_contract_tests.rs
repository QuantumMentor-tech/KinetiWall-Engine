use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use kinetiwall_catalog::{
    AssetReference, AvailabilityState, CatalogLifecycle, LocalJsonCatalogRepository,
    WallpaperCatalogRepository, WallpaperId, WallpaperMetadata, WallpaperRecord,
    WallpaperSourceKind, WallpaperType,
};
use kinetiwall_playback::mock::{MockHandle, MockPlayerAdapter, MockScript};
use kinetiwall_playback::*;

struct FakeLookup(HashMap<WallpaperId, PlaybackAssetInfo>);

impl WallpaperPlaybackLookup for FakeLookup {
    fn playback_info(&self, id: WallpaperId) -> Option<PlaybackAssetInfo> {
        self.0.get(&id).cloned()
    }
}

struct Fixture {
    orch: PlaybackOrchestrator,
    video: MockHandle,
    lookup: FakeLookup,
    video_id: WallpaperId,
    web_id: WallpaperId,
    missing_id: WallpaperId,
}

fn info(t: WallpaperType, a: AvailabilityState) -> PlaybackAssetInfo {
    PlaybackAssetInfo {
        wallpaper_type: t,
        availability: a,
    }
}

fn fixture() -> Fixture {
    let mut orch = PlaybackOrchestrator::new();
    let (adapter, video) = MockPlayerAdapter::new(MockScript::happy(WallpaperType::Video));
    orch.register_adapter(Box::new(adapter)).unwrap();
    let (video_id, web_id, missing_id) =
        (WallpaperId::new(), WallpaperId::new(), WallpaperId::new());
    let mut m = HashMap::new();
    m.insert(
        video_id,
        info(WallpaperType::Video, AvailabilityState::Available),
    );
    m.insert(
        web_id,
        info(WallpaperType::Web, AvailabilityState::Available),
    );
    m.insert(
        missing_id,
        info(WallpaperType::Video, AvailabilityState::Missing),
    );
    Fixture {
        orch,
        video,
        lookup: FakeLookup(m),
        video_id,
        web_id,
        missing_id,
    }
}

fn req(id: WallpaperId, t: WallpaperType, display: u32) -> PlaybackRequest {
    PlaybackRequest {
        request_id: PlaybackRequestId::new(),
        wallpaper_id: id,
        wallpaper_type: t,
        target: DisplayTargetId(display),
        config: None,
    }
}

fn accepted(o: StartOutcome) -> (PlaybackSessionId, SessionState) {
    match o {
        StartOutcome::Accepted { session_id, state } => (session_id, state),
        other => panic!("expected Accepted, got {other:?}"),
    }
}

fn rejected_kind(o: StartOutcome) -> PlaybackErrorKind {
    match o {
        StartOutcome::Rejected { error } => error.kind,
        other => panic!("expected Rejected, got {other:?}"),
    }
}

#[test]
fn accepted_start_is_not_playing_until_backend_confirms() {
    let mut f = fixture();
    let (sid, state) = accepted(
        f.orch
            .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0)),
    );
    assert_eq!(state, SessionState::Starting);
    let snap = f.orch.session(sid).unwrap();
    assert_eq!(snap.desired, DesiredState::Playing);
    assert_eq!(snap.observed, SessionState::Starting);
    assert_eq!(
        f.orch.handle_event(sid, PlayerReport::Playing),
        EventOutcome::Applied {
            state: SessionState::Playing
        }
    );
    assert_eq!(f.orch.session(sid).unwrap().observed, SessionState::Playing);
}

#[test]
fn request_validation_and_capability_availability_authorization_are_distinct() {
    let mut f = fixture();
    // Nil request id.
    let mut r = req(f.video_id, WallpaperType::Video, 0);
    r.request_id = PlaybackRequestId::from_uuid(uuid::Uuid::nil());
    assert_eq!(
        rejected_kind(f.orch.start_playback(&f.lookup, r)),
        PlaybackErrorKind::InvalidRequest
    );
    // Unknown wallpaper.
    assert_eq!(
        rejected_kind(
            f.orch
                .start_playback(&f.lookup, req(WallpaperId::new(), WallpaperType::Video, 0))
        ),
        PlaybackErrorKind::InvalidRequest
    );
    // Type mismatch vs catalog.
    assert_eq!(
        rejected_kind(
            f.orch
                .start_playback(&f.lookup, req(f.video_id, WallpaperType::Web, 0))
        ),
        PlaybackErrorKind::InvalidRequest
    );
    // Capability: no Web adapter registered; no silent fallback to the video adapter.
    assert_eq!(
        rejected_kind(
            f.orch
                .start_playback(&f.lookup, req(f.web_id, WallpaperType::Web, 0))
        ),
        PlaybackErrorKind::UnsupportedCapability
    );
    assert_eq!(f.video.count("prepare"), 0);
    // Asset availability.
    assert_eq!(
        rejected_kind(
            f.orch
                .start_playback(&f.lookup, req(f.missing_id, WallpaperType::Video, 0))
        ),
        PlaybackErrorKind::AssetUnavailable
    );
    // Backend availability.
    f.video.script(|s| {
        s.availability = PlayerAvailability::Unavailable {
            reason: "runtime missing".into(),
        }
    });
    assert_eq!(
        rejected_kind(
            f.orch
                .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0))
        ),
        PlaybackErrorKind::BackendUnavailable
    );
    f.video
        .script(|s| s.availability = PlayerAvailability::Available);
    // Adapter that cannot report observed state is not usable.
    f.video
        .script(|s| s.capabilities.reports_observed_state = false);
    assert_eq!(
        rejected_kind(
            f.orch
                .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0))
        ),
        PlaybackErrorKind::UnsupportedCapability
    );
    f.video
        .script(|s| s.capabilities.reports_observed_state = true);
    assert!(
        f.orch.sessions().is_empty(),
        "rejections must not create sessions"
    );
}

#[test]
fn policy_denial_is_authorization_not_capability() {
    struct Deny;
    impl PlaybackPolicy for Deny {
        fn authorize(&self, _: &PlaybackRequest) -> Result<(), String> {
            Err("blocked".into())
        }
    }
    let mut f = fixture();
    let mut orch = PlaybackOrchestrator::with_policy(Box::new(Deny));
    let (a, h) = MockPlayerAdapter::new(MockScript::happy(WallpaperType::Video));
    orch.register_adapter(Box::new(a)).unwrap();
    assert_eq!(
        rejected_kind(orch.start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0))),
        PlaybackErrorKind::PolicyDenied
    );
    assert_eq!(h.count("prepare"), 0);
    let _ = &mut f;
}

#[test]
fn duplicate_registration_is_rejected() {
    let mut f = fixture();
    let (a, _) = MockPlayerAdapter::new(MockScript::happy(WallpaperType::Video));
    assert!(f.orch.register_adapter(Box::new(a)).is_err());
}

#[test]
fn duplicate_request_and_same_wallpaper_do_not_launch_second_session() {
    let mut f = fixture();
    let r = req(f.video_id, WallpaperType::Video, 0);
    let (sid, _) = accepted(f.orch.start_playback(&f.lookup, r.clone()));
    // Same request id → Duplicate.
    assert!(
        matches!(f.orch.start_playback(&f.lookup, r), StartOutcome::Duplicate { session_id, .. } if session_id == sid)
    );
    // New request id, same wallpaper+target → AlreadyActive.
    assert!(matches!(
        f.orch.start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0)),
        StartOutcome::AlreadyActive { session_id, .. } if session_id == sid
    ));
    assert_eq!(f.video.count("prepare"), 1);
    assert_eq!(f.video.count("start"), 1);
    // Different wallpaper on the busy target → TargetBusy (no implicit replacement in Step 31).
    let other = WallpaperId::new();
    f.lookup.0.insert(
        other,
        info(WallpaperType::Video, AvailabilityState::Available),
    );
    assert_eq!(
        rejected_kind(
            f.orch
                .start_playback(&f.lookup, req(other, WallpaperType::Video, 0))
        ),
        PlaybackErrorKind::TargetBusy
    );
}

#[test]
fn independent_targets_get_independent_sessions_and_stop_is_scoped() {
    let mut f = fixture();
    let (a, _) = accepted(
        f.orch
            .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0)),
    );
    let (b, _) = accepted(
        f.orch
            .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 1)),
    );
    assert_ne!(a, b);
    f.orch.handle_event(a, PlayerReport::Playing);
    f.orch.handle_event(b, PlayerReport::Playing);
    assert_eq!(f.orch.stop_session(a), StopOutcome::Stopped);
    assert_eq!(f.orch.session(a).unwrap().observed, SessionState::Stopped);
    assert_eq!(f.orch.session(b).unwrap().observed, SessionState::Playing);
}

#[test]
fn stop_is_idempotent_and_frees_target_for_new_session() {
    let mut f = fixture();
    let (a, _) = accepted(
        f.orch
            .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0)),
    );
    assert_eq!(f.orch.stop_session(a), StopOutcome::Stopped);
    assert_eq!(f.orch.stop_session(a), StopOutcome::AlreadyStopped);
    assert_eq!(f.video.count("stop"), 1);
    let (b, _) = accepted(
        f.orch
            .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0)),
    );
    assert_ne!(a, b, "new session must get a new id");
    assert_eq!(
        f.orch.stop_session(PlaybackSessionId::new()),
        StopOutcome::UnknownSession
    );
}

#[test]
fn preparation_failure_marks_failed_without_cleanup_and_frees_target() {
    let mut f = fixture();
    f.video
        .script(|s| s.prepare = Err(PlaybackErrorKind::PreparationFailed));
    let out = f
        .orch
        .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0));
    let StartOutcome::Failed { session_id, error } = out else {
        panic!("expected Failed")
    };
    assert_eq!(error.kind, PlaybackErrorKind::PreparationFailed);
    let snap = f.orch.session(session_id).unwrap();
    assert_eq!(
        snap.observed,
        SessionState::Failed(PlaybackErrorKind::PreparationFailed)
    );
    assert!(!snap.cleanup_pending);
    // User message is safe; diagnostic never serialized.
    let json = serde_json::to_string(&error).unwrap();
    assert!(!json.contains("scripted failure"));
    f.video.script(|s| s.prepare = Ok(PrepareProgress::Ready));
    accepted(
        f.orch
            .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0)),
    );
}

#[test]
fn startup_failure_requires_cleanup_and_stays_failed() {
    let mut f = fixture();
    f.video
        .script(|s| s.start = Err(PlaybackErrorKind::StartupFailed));
    let StartOutcome::Failed { session_id, .. } = f
        .orch
        .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0))
    else {
        panic!()
    };
    let snap = f.orch.session(session_id).unwrap();
    assert_eq!(
        snap.observed,
        SessionState::Failed(PlaybackErrorKind::StartupFailed)
    );
    assert!(snap.cleanup_pending);
    // Target held until cleanup.
    f.video.script(|s| s.start = Ok(()));
    assert_eq!(
        rejected_kind(
            f.orch
                .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0))
        ),
        PlaybackErrorKind::TargetBusy
    );
    assert_eq!(
        f.orch.stop_session(session_id),
        StopOutcome::CleanedUpFailedSession
    );
    let snap = f.orch.session(session_id).unwrap();
    assert!(
        matches!(snap.observed, SessionState::Failed(_)),
        "failed must not be rewritten as stopped"
    );
    assert!(!snap.cleanup_pending);
    assert_eq!(f.orch.stop_session(session_id), StopOutcome::AlreadyStopped);
    accepted(
        f.orch
            .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0)),
    );
}

#[test]
fn failure_is_isolated_to_one_session() {
    let mut f = fixture();
    let (a, _) = accepted(
        f.orch
            .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0)),
    );
    let (b, _) = accepted(
        f.orch
            .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 1)),
    );
    f.orch.handle_event(b, PlayerReport::Playing);
    f.orch.handle_event(
        a,
        PlayerReport::Failed {
            kind: PlaybackErrorKind::BackendExited,
        },
    );
    assert!(matches!(
        f.orch.session(a).unwrap().observed,
        SessionState::Failed(PlaybackErrorKind::BackendExited)
    ));
    assert_eq!(f.orch.session(b).unwrap().observed, SessionState::Playing);
}

#[test]
fn pending_prepare_cancellation_and_late_events() {
    let mut f = fixture();
    f.video.script(|s| s.prepare = Ok(PrepareProgress::Pending));
    let (sid, state) = accepted(
        f.orch
            .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0)),
    );
    assert_eq!(state, SessionState::Preparing);
    assert_eq!(f.video.count("start"), 0);
    assert_eq!(f.orch.stop_session(sid), StopOutcome::Canceled);
    assert_eq!(
        f.orch.session(sid).unwrap().observed,
        SessionState::Canceled
    );
    // Delayed Ready/Playing arriving after cancellation must not resurrect the session or start.
    assert!(matches!(
        f.orch.handle_event(sid, PlayerReport::Ready),
        EventOutcome::IgnoredTerminalSession { .. }
    ));
    assert!(matches!(
        f.orch.handle_event(sid, PlayerReport::Playing),
        EventOutcome::IgnoredTerminalSession { .. }
    ));
    assert_eq!(f.video.count("start"), 0);
    assert_eq!(
        f.orch.cancel_session(sid),
        CancelOutcome::NotCancellable {
            state: SessionState::Canceled
        }
    );
}

#[test]
fn pending_prepare_completes_then_starts() {
    let mut f = fixture();
    f.video.script(|s| s.prepare = Ok(PrepareProgress::Pending));
    let (sid, _) = accepted(
        f.orch
            .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0)),
    );
    assert_eq!(
        f.orch.handle_event(sid, PlayerReport::Ready),
        EventOutcome::Applied {
            state: SessionState::Starting
        }
    );
    assert_eq!(f.video.count("start"), 1);
}

#[test]
fn cancel_unsupported_falls_back_to_stop() {
    let mut f = fixture();
    f.video.script(|s| {
        s.prepare = Ok(PrepareProgress::Pending);
        s.cancel = Ok(CancelAck::Unsupported);
    });
    let (sid, _) = accepted(
        f.orch
            .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0)),
    );
    let out = f.orch.cancel_session(sid);
    assert_eq!(
        out,
        CancelOutcome::FellBackToStop {
            stop: Box::new(StopOutcome::Stopped)
        }
    );
    assert_eq!(f.orch.session(sid).unwrap().observed, SessionState::Stopped);
}

#[test]
fn playing_session_cannot_be_canceled_only_stopped() {
    let mut f = fixture();
    let (sid, _) = accepted(
        f.orch
            .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0)),
    );
    f.orch.handle_event(sid, PlayerReport::Playing);
    assert_eq!(
        f.orch.cancel_session(sid),
        CancelOutcome::NotCancellable {
            state: SessionState::Playing
        }
    );
}

#[test]
fn invalid_transitions_stale_and_unsolicited_reports_are_handled() {
    let mut f = fixture();
    let (sid, _) = accepted(
        f.orch
            .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0)),
    );
    // Ready report while Starting is invalid; state unchanged.
    assert!(matches!(
        f.orch.handle_event(sid, PlayerReport::Ready),
        EventOutcome::IgnoredInvalidTransition {
            state: SessionState::Starting,
            ..
        }
    ));
    assert_eq!(
        f.orch.session(sid).unwrap().observed,
        SessionState::Starting
    );
    // Unknown / obsolete session id.
    assert_eq!(
        f.orch
            .handle_event(PlaybackSessionId::new(), PlayerReport::Playing),
        EventOutcome::IgnoredUnknownSession
    );
    // Obsolete session's event cannot touch a newer session on the same target.
    f.orch.stop_session(sid);
    let (new_sid, _) = accepted(
        f.orch
            .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0)),
    );
    assert!(matches!(
        f.orch.handle_event(sid, PlayerReport::Playing),
        EventOutcome::IgnoredTerminalSession { .. }
    ));
    assert_eq!(
        f.orch.session(new_sid).unwrap().observed,
        SessionState::Starting
    );
    // Unsolicited Stopped while live = backend exited.
    f.orch.handle_event(new_sid, PlayerReport::Playing);
    f.orch.handle_event(new_sid, PlayerReport::Stopped);
    assert!(matches!(
        f.orch.session(new_sid).unwrap().observed,
        SessionState::Failed(PlaybackErrorKind::BackendExited)
    ));
}

#[test]
fn transition_table_matches_documented_rules() {
    use SessionState::*;
    let all = [
        Preparing,
        Ready,
        Starting,
        Playing,
        Stopping,
        Stopped,
        Failed(PlaybackErrorKind::Internal),
        Canceled,
    ];
    for t in [Stopped, Canceled, Failed(PlaybackErrorKind::Timeout)] {
        for n in &all {
            assert!(!t.can_transition_to(n), "{t:?} is terminal");
        }
    }
    assert!(Starting.can_transition_to(&Playing));
    assert!(
        !Ready.can_transition_to(&Playing),
        "playing requires starting first"
    );
    assert!(!Preparing.can_transition_to(&Playing));
    assert!(!Playing.can_transition_to(&Canceled));
    assert!(!Playing.can_transition_to(&Starting));
}

#[test]
fn timeouts_mark_uncertain_outcome_and_require_reconciliation() {
    let mut f = fixture();
    let (sid, _) = accepted(
        f.orch
            .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0)),
    );
    // Wrong phase is ignored.
    assert!(matches!(
        f.orch.handle_timeout(sid, PendingPhase::Prepare),
        EventOutcome::IgnoredInvalidTransition { .. }
    ));
    assert!(matches!(
        f.orch.handle_timeout(sid, PendingPhase::Start),
        EventOutcome::Applied {
            state: SessionState::Failed(PlaybackErrorKind::Timeout)
        }
    ));
    let snap = f.orch.session(sid).unwrap();
    assert!(snap.outcome_uncertain && snap.cleanup_pending);
    // A late Playing after timeout is ignored; reconcile via stop, no automatic retry.
    assert!(matches!(
        f.orch.handle_event(sid, PlayerReport::Playing),
        EventOutcome::IgnoredTerminalSession { .. }
    ));
    assert_eq!(f.video.count("start"), 1);
    assert_eq!(
        f.orch.stop_session(sid),
        StopOutcome::CleanedUpFailedSession
    );
}

#[test]
fn stop_failure_is_reported_and_cleanup_stays_pending() {
    let mut f = fixture();
    let (sid, _) = accepted(
        f.orch
            .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0)),
    );
    f.orch.handle_event(sid, PlayerReport::Playing);
    f.video
        .script(|s| s.stop = Err(PlaybackErrorKind::StopFailed));
    assert!(matches!(
        f.orch.stop_session(sid),
        StopOutcome::Failed { .. }
    ));
    let snap = f.orch.session(sid).unwrap();
    assert_eq!(
        snap.observed,
        SessionState::Failed(PlaybackErrorKind::StopFailed)
    );
    assert!(snap.cleanup_pending);
    f.video.script(|s| s.stop = Ok(StopAck::Stopped));
    assert_eq!(
        f.orch.stop_session(sid),
        StopOutcome::CleanedUpFailedSession
    );
}

#[test]
fn shutdown_with_pending_operations_is_bounded_and_idempotent() {
    let mut f = fixture();
    f.video.script(|s| s.prepare = Ok(PrepareProgress::Pending));
    let (pending, _) = accepted(
        f.orch
            .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0)),
    );
    f.video.script(|s| s.prepare = Ok(PrepareProgress::Ready));
    let (playing, _) = accepted(
        f.orch
            .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 1)),
    );
    f.orch.handle_event(playing, PlayerReport::Playing);
    let report = f.orch.shutdown();
    assert_eq!(report.sessions_released, 2);
    assert!(report.session_failures.is_empty() && report.adapter_failures.is_empty());
    assert_eq!(
        f.orch.session(pending).unwrap().observed,
        SessionState::Canceled
    );
    assert_eq!(
        f.orch.session(playing).unwrap().observed,
        SessionState::Stopped
    );
    assert_eq!(f.video.count("shutdown"), 1);
    // New work refused; repeated shutdown does nothing.
    assert_eq!(
        rejected_kind(
            f.orch
                .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 2))
        ),
        PlaybackErrorKind::ShuttingDown
    );
    assert!(f.orch.shutdown().already_shut_down);
    assert_eq!(f.video.count("shutdown"), 1);
}

#[test]
fn shutdown_reports_adapter_and_session_failures_without_panicking() {
    let mut f = fixture();
    let (sid, _) = accepted(
        f.orch
            .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0)),
    );
    f.video.script(|s| {
        s.stop = Err(PlaybackErrorKind::StopFailed);
        s.shutdown = Err(PlaybackErrorKind::Internal);
    });
    let report = f.orch.shutdown();
    assert_eq!(report.session_failures.len(), 1);
    assert_eq!(report.session_failures[0].0, sid);
    assert_eq!(
        report.adapter_failures,
        vec![(WallpaperType::Video, PlaybackErrorKind::Internal)]
    );
}

#[test]
fn concurrent_identical_requests_create_exactly_one_session() {
    let f = fixture();
    let (orch, lookup, id) = (Arc::new(Mutex::new(f.orch)), Arc::new(f.lookup), f.video_id);
    let shared = req(id, WallpaperType::Video, 0);
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let (o, l, r) = (orch.clone(), lookup.clone(), shared.clone());
            std::thread::spawn(move || o.lock().unwrap().start_playback(&*l, r))
        })
        .collect();
    let outcomes: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let accepted_count = outcomes
        .iter()
        .filter(|o| matches!(o, StartOutcome::Accepted { .. }))
        .count();
    assert_eq!(accepted_count, 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| matches!(o, StartOutcome::Duplicate { .. }))
            .count(),
        7
    );
    assert_eq!(f.video.count("prepare"), 1);
    assert_eq!(orch.lock().unwrap().sessions().len(), 1);
}

#[test]
fn prune_removes_only_finished_sessions_without_pending_cleanup() {
    let mut f = fixture();
    let (a, _) = accepted(
        f.orch
            .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 0)),
    );
    let (b, _) = accepted(
        f.orch
            .start_playback(&f.lookup, req(f.video_id, WallpaperType::Video, 1)),
    );
    f.orch.stop_session(a);
    f.orch.handle_event(
        b,
        PlayerReport::Failed {
            kind: PlaybackErrorKind::BackendExited,
        },
    );
    assert_eq!(f.orch.prune_finished(), 1);
    assert!(f.orch.session(a).is_none());
    assert!(
        f.orch.session(b).is_some(),
        "failed session with pending cleanup is retained"
    );
}

#[test]
fn contracts_serialize_without_paths_or_diagnostics() {
    let r = req(WallpaperId::new(), WallpaperType::Web, 3);
    let json = serde_json::to_string(&r).unwrap();
    assert!(!json.contains('\\') && !json.contains("path"));
    let back: PlaybackRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(back, r);
    let st = serde_json::to_string(&SessionState::Failed(PlaybackErrorKind::Timeout)).unwrap();
    assert!(st.contains("failed") && st.contains("timeout"));
}

#[test]
fn real_catalog_repository_works_as_trusted_lookup() {
    let dir = std::env::temp_dir().join(format!("kw_play_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut repo = LocalJsonCatalogRepository::open(dir.clone()).unwrap();
    let id = WallpaperId::new();
    let mut rec = WallpaperRecord::new(
        id,
        WallpaperType::Video,
        AssetReference {
            source_kind: WallpaperSourceKind::ExternalDirectFile,
            entrypoint: dir.join("a.mp4").to_string_lossy().into_owned(),
        },
        WallpaperMetadata::new("Clip"),
        CatalogLifecycle::now(false),
    );
    rec.availability = AvailabilityState::Available;
    repo.create(rec).unwrap();

    let mut orch = PlaybackOrchestrator::new();
    let (a, _) = MockPlayerAdapter::new(MockScript::happy(WallpaperType::Video));
    orch.register_adapter(Box::new(a)).unwrap();
    accepted(orch.start_playback(&repo, req(id, WallpaperType::Video, 0)));
    assert_eq!(
        rejected_kind(orch.start_playback(&repo, req(WallpaperId::new(), WallpaperType::Video, 1))),
        PlaybackErrorKind::InvalidRequest
    );
    let _ = std::fs::remove_dir_all(dir);
}
