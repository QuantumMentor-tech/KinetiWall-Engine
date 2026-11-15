//! Contract & Integration tests for `WebPlayerAdapter` and `kinetiwall-player-web`.

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use kinetiwall_catalog::WallpaperId;
use kinetiwall_playback::{
    CancelAck, DisplayTargetId, PlaybackErrorKind, PlaybackSessionId, PlayerAdapter,
    PlayerAvailability, SessionContext, StopAck,
};
use kinetiwall_player_web::validate::{is_uri_permitted, validate_web_wallpaper_path};
use kinetiwall_player_web::WebPlayerAdapter;

fn make_temp_web_wallpaper(name: &str, content: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("kw_web_test_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let file_path = dir.join(name);
    let mut f = File::create(&file_path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
    file_path
}

#[test]
fn test_web_path_validation_and_containment() {
    // Null byte rejection
    assert_eq!(
        validate_web_wallpaper_path("C:\\wallpapers\\index\0.html"),
        Err(PlaybackErrorKind::InvalidRequest)
    );

    // Non-html rejection
    assert_eq!(
        validate_web_wallpaper_path("C:\\windows\\system32\\calc.exe"),
        Err(PlaybackErrorKind::InvalidRequest)
    );

    // Missing file rejection
    assert_eq!(
        validate_web_wallpaper_path("C:\\nonexistent_dir_12345\\index.html"),
        Err(PlaybackErrorKind::AssetUnavailable)
    );

    // Valid HTML wallpaper
    let valid_path = make_temp_web_wallpaper(
        "index.html",
        "<!DOCTYPE html><html><body><h1>Interactive Wallpaper</h1></body></html>",
    );
    let validated = validate_web_wallpaper_path(valid_path.to_str().unwrap()).unwrap();
    assert_eq!(validated.entrypoint, valid_path);
    assert_eq!(validated.asset_root, valid_path.parent().unwrap());
    let _ = std::fs::remove_dir_all(valid_path.parent().unwrap());
}

#[test]
fn test_security_uri_permission_boundaries() {
    // Permitted: local virtual origin & blank
    assert!(is_uri_permitted("about:blank"));
    assert!(is_uri_permitted("https://kinetiwall.local/"));
    assert!(is_uri_permitted("https://kinetiwall.local/index.html"));
    assert!(is_uri_permitted(
        "https://kinetiwall.local/shaders/fragment.glsl"
    ));

    // Prohibited: internet, file protocols, custom schemes
    assert!(!is_uri_permitted("https://google.com"));
    assert!(!is_uri_permitted("http://malicious.site/script.js"));
    assert!(!is_uri_permitted(
        "file:///C:/Users/Administrator/secret.txt"
    ));
    assert!(!is_uri_permitted("javascript:alert(1)"));
}

#[test]
fn test_web_player_adapter_capabilities_and_availability() {
    let adapter = WebPlayerAdapter::new(|_| None);
    let caps = adapter.capabilities();
    assert_eq!(caps.wallpaper_type, kinetiwall_catalog::WallpaperType::Web);
    assert!(caps.supports_cancellation);
    assert!(caps.reports_observed_state);
    assert_eq!(adapter.availability(), PlayerAvailability::Available);
}

#[test]
fn test_web_player_adapter_full_lifecycle_with_isolated_process() {
    let web_path = make_temp_web_wallpaper(
        "index.html",
        "<!DOCTYPE html><html><head><script>console.log('Canvas Init');</script></head><body><canvas id='c'></canvas></body></html>",
    );
    let web_path_str = web_path.to_string_lossy().into_owned();

    let wallpaper_id = WallpaperId::new();
    let session_id = PlaybackSessionId::new();
    let ctx = SessionContext::new(
        session_id,
        kinetiwall_playback::PlaybackRequestId::new(),
        wallpaper_id,
        DisplayTargetId(0),
    );

    let p = web_path_str.clone();
    let mut adapter = WebPlayerAdapter::new(move |wid| {
        if wid == wallpaper_id {
            Some(p.clone())
        } else {
            None
        }
    });

    // 1. Prepare
    let prep_res = adapter.prepare(&ctx);
    assert!(prep_res.is_ok());

    // 2. Start
    let start_res = adapter.start(&ctx);
    assert!(start_res.is_ok());

    // Poll events from host
    let mut saw_playing = false;
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let events = adapter.poll_events();
        for ev in events {
            if let kinetiwall_player_web::FromHost::Report { report, .. } = ev {
                if report == kinetiwall_playback::PlayerReport::Playing {
                    saw_playing = true;
                }
            }
        }
        if saw_playing {
            break;
        }
    }
    assert!(
        saw_playing,
        "Expected Playing report from isolated web host"
    );

    // 2b. Dispatch bounded audio visualization frame over IPC
    let frame = kinetiwall_audio::VisualizerFrame {
        sequence: 1,
        timestamp_ms: 500,
        rms: 0.65,
        peak: 0.85,
        frequency_bands: [0.1, 0.4, 0.7, 0.9, 0.5, 0.3, 0.2, 0.1],
        is_silent: false,
    };
    assert!(adapter.send_visualizer_frame(ctx.session_id, frame).is_ok());

    // 3. Stop
    let stop_res = adapter.stop(&ctx);
    assert_eq!(stop_res, Ok(StopAck::Stopped));

    // 4. Shutdown
    assert!(adapter.shutdown().is_ok());

    let _ = std::fs::remove_dir_all(web_path.parent().unwrap());
}

#[test]
fn test_web_player_adapter_missing_asset_fails_cleanly() {
    let wallpaper_id = WallpaperId::new();
    let ctx = SessionContext::new(
        PlaybackSessionId::new(),
        kinetiwall_playback::PlaybackRequestId::new(),
        wallpaper_id,
        DisplayTargetId(0),
    );

    let mut adapter = WebPlayerAdapter::new(|_| None);
    assert!(adapter.prepare(&ctx).is_ok());

    // Start with missing asset
    let err = adapter.start(&ctx).unwrap_err();
    assert_eq!(err.kind, PlaybackErrorKind::AssetUnavailable);
    assert!(adapter.shutdown().is_ok());
}

#[test]
fn test_web_player_adapter_cancel_flow() {
    let wallpaper_id = WallpaperId::new();
    let ctx = SessionContext::new(
        PlaybackSessionId::new(),
        kinetiwall_playback::PlaybackRequestId::new(),
        wallpaper_id,
        DisplayTargetId(0),
    );

    let mut adapter = WebPlayerAdapter::new(|_| None);
    assert!(adapter.prepare(&ctx).is_ok());
    assert_eq!(adapter.cancel(&ctx), Ok(CancelAck::Accepted));
    assert!(adapter.shutdown().is_ok());
}
