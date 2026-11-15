//! Stress, Reliability, and Resource Lifecycle tests for `kinetiwall-player-web`.

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use kinetiwall_audio::VisualizerFrame;
use kinetiwall_catalog::WallpaperId;
use kinetiwall_playback::{
    DisplayTargetId, PlaybackSessionId, PlayerAdapter, SessionContext, StopAck,
};
use kinetiwall_player_web::validate::{is_uri_permitted, validate_web_wallpaper_path};
use kinetiwall_player_web::WebPlayerAdapter;

fn make_temp_web_fixture(name: &str, html: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("kw_web_stress_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let file_path = dir.join(name);
    let mut f = File::create(&file_path).unwrap();
    f.write_all(html.as_bytes()).unwrap();
    file_path
}

/// Stress test for repeated real WebView2 child process lifecycle execution:
/// launches child process, loads HTML fixture, polls for Playing report,
/// issues graceful stop, shuts down adapter, and verifies clean process teardown.
#[test]
fn test_web_adapter_repeated_process_lifecycle_stress() {
    let fixture_path = make_temp_web_fixture(
        "stress_anim.html",
        r#"<!DOCTYPE html>
<html>
<head><style>body { margin: 0; background: #000; overflow: hidden; }</style></head>
<body>
<canvas id="c" width="800" height="600"></canvas>
<script>
  const c = document.getElementById('c');
  const ctx = c.getContext('2d');
  let t = 0;
  function render() {
    ctx.fillStyle = `hsl(${t % 360}, 50%, 50%)`;
    ctx.fillRect(0, 0, 800, 600);
    t++;
    requestAnimationFrame(render);
  }
  render();
</script>
</body>
</html>"#,
    );
    let fixture_path_str = fixture_path.to_string_lossy().into_owned();

    for cycle in 0..5 {
        let wallpaper_id = WallpaperId::new();
        let session_id = PlaybackSessionId::new();
        let ctx = SessionContext::new(
            session_id,
            kinetiwall_playback::PlaybackRequestId::new(),
            wallpaper_id,
            DisplayTargetId(0),
        );

        let p = fixture_path_str.clone();
        let mut adapter = WebPlayerAdapter::new(move |wid| {
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
            "Child PID must be present in cycle {}",
            cycle
        );

        // Poll for Playing report
        let mut saw_playing = false;
        for _ in 0..25 {
            std::thread::sleep(Duration::from_millis(40));
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
        assert!(saw_playing, "Cycle {} must reach Playing", cycle);

        // 3. Stop
        let stop_res = adapter.stop(&ctx);
        assert_eq!(stop_res, Ok(StopAck::Stopped));

        // 4. Shutdown
        assert!(adapter.shutdown().is_ok());
    }

    let _ = std::fs::remove_dir_all(fixture_path.parent().unwrap());
}

/// Stress test for high-frequency audio visualization frame streaming over anonymous IPC pipe:
/// streams 200 consecutive visualizer frames into the running child worker, verifying that
/// the pipe does not block, overflow, or destabilize the child process.
#[test]
fn test_web_adapter_high_frequency_audio_streaming_stress() {
    let fixture_path = make_temp_web_fixture(
        "audio_reactive.html",
        r#"<!DOCTYPE html><html><body><h1>Audio Stress</h1></body></html>"#,
    );
    let fixture_path_str = fixture_path.to_string_lossy().into_owned();

    let wallpaper_id = WallpaperId::new();
    let session_id = PlaybackSessionId::new();
    let ctx = SessionContext::new(
        session_id,
        kinetiwall_playback::PlaybackRequestId::new(),
        wallpaper_id,
        DisplayTargetId(0),
    );

    let p = fixture_path_str.clone();
    let mut adapter = WebPlayerAdapter::new(move |wid| {
        if wid == wallpaper_id {
            Some(p.clone())
        } else {
            None
        }
    });

    assert!(adapter.prepare(&ctx).is_ok());
    assert!(adapter.start(&ctx).is_ok());

    // Stream 200 consecutive frames at high rate
    for i in 1..=200 {
        let frame = VisualizerFrame {
            sequence: i,
            timestamp_ms: i * 16,
            rms: (i as f32 % 10.0) / 10.0,
            peak: 0.95,
            frequency_bands: [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8],
            is_silent: i % 20 == 0,
        };
        let send_res = adapter.send_visualizer_frame(session_id, frame);
        assert!(
            send_res.is_ok(),
            "Streaming frame {} should succeed without IPC error",
            i
        );
    }

    assert_eq!(adapter.stop(&ctx), Ok(StopAck::Stopped));
    assert!(adapter.shutdown().is_ok());

    let _ = std::fs::remove_dir_all(fixture_path.parent().unwrap());
}

/// Boundary fuzzing test verifying that disallowed URI protocols, malformed paths,
/// and traversal attacks are rejected by the web confinement layer.
#[test]
fn test_web_security_uri_and_traversal_fuzzing() {
    let forbidden_uris = [
        "javascript:alert(document.cookie)",
        "file:///C:/Windows/System32/cmd.exe",
        "data:text/html,<script>alert(1)</script>",
        "vbscript:msgbox(1)",
        "blob:https://kinetiwall.local/guid",
        "http://attacker.com/evil.html",
        "https://attacker.com/evil.html",
        "chrome://settings",
        "ms-settings:privacy",
    ];

    for uri in forbidden_uris {
        assert!(
            !is_uri_permitted(uri),
            "URI {:?} must NOT be permitted by web sandbox confinement",
            uri
        );
    }

    let invalid_paths = [
        "",
        "   ",
        "C:\\some\\path\0index.html",
        "../../etc/passwd",
        "C:\\index.html\0.exe",
        "relative/path/index.html",
    ];

    for p in invalid_paths {
        assert!(
            validate_web_wallpaper_path(p).is_err(),
            "Path {:?} must be rejected by web path validator",
            p
        );
    }
}
