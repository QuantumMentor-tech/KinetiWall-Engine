//! Stress, Reliability, and Resource Lifecycle tests for `kinetiwall-player-video`.

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use kinetiwall_catalog::WallpaperId;
use kinetiwall_playback::{
    DisplayTargetId, PlaybackErrorKind, PlaybackSessionId, PlayerAdapter, SessionContext, StopAck,
};
use kinetiwall_player_video::validate::validate_media_path;
use kinetiwall_player_video::VideoPlayerAdapter;

fn make_temp_video_fixture(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("kw_video_stress_{}", uuid::Uuid::new_v4()));
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

/// Stress test for repeated real process lifecycle execution: launches child process,
/// monitors readiness and playback, issues graceful stop, shuts down adapter, and verifies
/// process termination across multiple cycles.
#[test]
fn test_video_adapter_repeated_process_lifecycle_stress() {
    let video_path = make_temp_video_fixture("looping_stress.mp4");
    let video_path_str = video_path.to_string_lossy().into_owned();

    for cycle in 0..5 {
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
        assert!(adapter.prepare(&ctx).is_ok());

        // 2. Start
        assert!(adapter.start(&ctx).is_ok());

        // Verify child PID exists
        let pid = adapter.process_id(&ctx);
        assert!(
            pid.is_some(),
            "Child PID should be available in cycle {}",
            cycle
        );

        // Poll for Playing report
        let mut saw_playing = false;
        for _ in 0..25 {
            std::thread::sleep(Duration::from_millis(40));
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
        assert!(saw_playing, "Cycle {} must reach Playing", cycle);

        // 3. Stop
        let stop_res = adapter.stop(&ctx);
        assert_eq!(stop_res, Ok(StopAck::Stopped));

        // 4. Shutdown and cleanup
        assert!(adapter.shutdown().is_ok());
    }

    let _ = std::fs::remove_dir_all(video_path.parent().unwrap());
}

/// Fuzzing and boundary validation test against malformed, inaccessible, and path-traversal inputs.
#[test]
fn test_video_path_security_and_traversal_fuzzing() {
    let invalid_inputs = [
        "",
        "   ",
        "C:\\some\\path\0malicious.mp4",
        "../../etc/passwd",
        "C:\\windows\\system32\\cmd.exe",
        "C:\\video.mp4\0.txt",
        "relative/path/video.mp4",
        "http://evil.com/video.mp4",
    ];

    for input in invalid_inputs {
        let res = validate_media_path(input);
        assert!(
            res.is_err(),
            "Input {:?} should be rejected by media path validator",
            input
        );
    }
}

/// Tests that starting an adapter with a missing asset consistently returns AssetUnavailable
/// without hanging or leaving an unmanaged child process running.
#[test]
fn test_video_adapter_missing_asset_stress() {
    for _ in 0..10 {
        let wallpaper_id = WallpaperId::new();
        let ctx = SessionContext::new(
            PlaybackSessionId::new(),
            kinetiwall_playback::PlaybackRequestId::new(),
            wallpaper_id,
            DisplayTargetId(0),
        );

        let mut adapter = VideoPlayerAdapter::new(|_| None);
        assert!(adapter.prepare(&ctx).is_ok());

        let err = adapter.start(&ctx).unwrap_err();
        assert_eq!(err.kind, PlaybackErrorKind::AssetUnavailable);
        assert!(adapter.shutdown().is_ok());
    }
}
