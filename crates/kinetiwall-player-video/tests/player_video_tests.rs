//! Contract & Integration tests for `VideoPlayerAdapter` and `kinetiwall-player-video`.

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use kinetiwall_catalog::WallpaperId;
use kinetiwall_playback::{
    CancelAck, DisplayTargetId, PlaybackErrorKind, PlaybackSessionId, PlayerAdapter,
    PlayerAvailability, SessionContext, StopAck,
};
use kinetiwall_player_video::validate::validate_media_path;
use kinetiwall_player_video::VideoPlayerAdapter;

fn make_temp_video_file(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("kw_test_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let file_path = dir.join(name);
    let mut f = File::create(&file_path).unwrap();
    // Valid MP4 header (ftyp box)
    let header = [
        0x00, 0x00, 0x00, 0x18, b'f', b't', b'y', b'p', b'm', b'p', b'4', b'2', 0x00, 0x00, 0x00,
        0x00,
    ];
    f.write_all(&header).unwrap();
    file_path
}

#[test]
fn test_media_path_validation_boundaries() {
    // Null byte rejection
    assert_eq!(
        validate_media_path("C:\\videos\\test\0.mp4"),
        Err(PlaybackErrorKind::InvalidRequest)
    );

    // Non-video extension rejection
    assert_eq!(
        validate_media_path("C:\\windows\\system32\\cmd.exe"),
        Err(PlaybackErrorKind::InvalidRequest)
    );

    // Missing file
    assert_eq!(
        validate_media_path("C:\\nonexistent_dir_12345\\missing.mp4"),
        Err(PlaybackErrorKind::AssetUnavailable)
    );

    // Valid file
    let path = make_temp_video_file("sample.mp4");
    assert!(validate_media_path(path.to_str().unwrap()).is_ok());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn test_video_player_adapter_capabilities_and_availability() {
    let adapter = VideoPlayerAdapter::new(|_| None);
    let caps = adapter.capabilities();
    assert_eq!(
        caps.wallpaper_type,
        kinetiwall_catalog::WallpaperType::Video
    );
    assert!(caps.supports_cancellation);
    assert!(caps.reports_observed_state);
    assert_eq!(adapter.availability(), PlayerAvailability::Available);
}

#[test]
fn test_video_player_adapter_full_lifecycle_with_isolated_process() {
    let video_path = make_temp_video_file("nature.mp4");
    let video_path_str = video_path.to_string_lossy().into_owned();

    let wallpaper_id = WallpaperId::new();
    let session_id = PlaybackSessionId::new();
    let ctx = SessionContext::new(
        session_id,
        kinetiwall_playback::PlaybackRequestId::new(),
        wallpaper_id,
        DisplayTargetId(0),
    );

    let p = video_path_str.clone();
    let mut adapter = VideoPlayerAdapter::new(move |wid| {
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
            if let kinetiwall_player_video::FromHost::Report { report, .. } = ev {
                if report == kinetiwall_playback::PlayerReport::Playing {
                    saw_playing = true;
                }
            }
        }
        if saw_playing {
            break;
        }
    }
    assert!(saw_playing, "Expected Playing report from isolated host");

    // 3. Stop
    let stop_res = adapter.stop(&ctx);
    assert_eq!(stop_res, Ok(StopAck::Stopped));

    // 4. Shutdown
    assert!(adapter.shutdown().is_ok());

    let _ = std::fs::remove_dir_all(video_path.parent().unwrap());
}

#[test]
fn test_video_player_adapter_missing_asset_fails_cleanly() {
    let wallpaper_id = WallpaperId::new();
    let ctx = SessionContext::new(
        PlaybackSessionId::new(),
        kinetiwall_playback::PlaybackRequestId::new(),
        wallpaper_id,
        DisplayTargetId(0),
    );

    let mut adapter = VideoPlayerAdapter::new(|_| None);
    assert!(adapter.prepare(&ctx).is_ok());

    // Start with missing asset from lookup
    let err = adapter.start(&ctx).unwrap_err();
    assert_eq!(err.kind, PlaybackErrorKind::AssetUnavailable);
    assert!(adapter.shutdown().is_ok());
}

#[test]
fn test_video_player_adapter_cancel_flow() {
    let wallpaper_id = WallpaperId::new();
    let ctx = SessionContext::new(
        PlaybackSessionId::new(),
        kinetiwall_playback::PlaybackRequestId::new(),
        wallpaper_id,
        DisplayTargetId(0),
    );

    let mut adapter = VideoPlayerAdapter::new(|_| None);
    assert!(adapter.prepare(&ctx).is_ok());
    assert_eq!(adapter.cancel(&ctx), Ok(CancelAck::Accepted));
    assert!(adapter.shutdown().is_ok());
}
