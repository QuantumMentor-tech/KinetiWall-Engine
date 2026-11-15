//! Standalone isolated video player process executable.
//!
//! Communicates with the KinetiWall controller via line-delimited JSON on stdin/stdout.

use std::io::{self, BufRead, Write};
use std::time::Duration;

use kinetiwall_playback::PlayerReport;
use kinetiwall_player_video::engine::create_video_engine;
use kinetiwall_player_video::protocol::{FromHost, HostOp, ToHost};
use kinetiwall_player_video::validate::validate_media_path;

fn send_msg(msg: &FromHost) {
    if let Ok(line) = serde_json::to_string(msg) {
        let stdout = io::stdout();
        let mut handle = stdout.lock();
        let _ = writeln!(handle, "{line}");
        let _ = handle.flush();
    }
}

fn main() {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut engine = create_video_engine();
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF, pipe closed by controller
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let cmd: ToHost = match serde_json::from_str(trimmed) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                match cmd {
                    ToHost::Prepare { session_id } => {
                        let res = engine.initialize();
                        send_msg(&FromHost::Ack {
                            session_id,
                            op: HostOp::Prepare,
                            ok: res.is_ok(),
                            error: res.err(),
                        });
                        if res.is_ok() {
                            send_msg(&FromHost::Report {
                                session_id,
                                report: PlayerReport::Ready,
                            });
                        }
                    }
                    ToHost::Start {
                        session_id,
                        media_path,
                    } => {
                        // Validate path safety first
                        match validate_media_path(&media_path) {
                            Ok(validated_path) => {
                                let res = engine.load_file(&validated_path);
                                send_msg(&FromHost::Ack {
                                    session_id,
                                    op: HostOp::Start,
                                    ok: res.is_ok(),
                                    error: res.err(),
                                });

                                if res.is_ok() {
                                    // Process initial events until playback confirmed or failed
                                    let mut confirmed = false;
                                    for _ in 0..10 {
                                        if let Some(event) =
                                            engine.wait_event(Duration::from_millis(50))
                                        {
                                            match event {
                                                kinetiwall_player_video::engine::EngineEvent::FileLoaded => {
                                                    // File loaded successfully
                                                }
                                                kinetiwall_player_video::engine::EngineEvent::PlaybackRestart => {
                                                    confirmed = true;
                                                    send_msg(&FromHost::Report {
                                                        session_id,
                                                        report: PlayerReport::Playing,
                                                    });
                                                    let hwdec = engine.hwdec_current();
                                                    send_msg(&FromHost::HwDec {
                                                        session_id,
                                                        mode: hwdec,
                                                    });
                                                    break;
                                                }
                                                kinetiwall_player_video::engine::EngineEvent::PlaybackFailed(kind) => {
                                                    send_msg(&FromHost::Report {
                                                        session_id,
                                                        report: PlayerReport::Failed { kind },
                                                    });
                                                    break;
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                    if !confirmed {
                                        // Still poll or emit playing if engine confirms playback
                                    }
                                }
                            }
                            Err(err) => {
                                send_msg(&FromHost::Ack {
                                    session_id,
                                    op: HostOp::Start,
                                    ok: false,
                                    error: Some(err),
                                });
                                send_msg(&FromHost::Report {
                                    session_id,
                                    report: PlayerReport::Failed { kind: err },
                                });
                            }
                        }
                    }
                    ToHost::Stop { session_id } => {
                        let res = engine.stop();
                        send_msg(&FromHost::Ack {
                            session_id,
                            op: HostOp::Stop,
                            ok: res.is_ok(),
                            error: res.err(),
                        });
                        send_msg(&FromHost::Report {
                            session_id,
                            report: PlayerReport::Stopped,
                        });
                    }
                    ToHost::SetVolume { volume, .. } => {
                        let _ = engine.set_volume(volume);
                    }
                    ToHost::SetMute { muted, .. } => {
                        let _ = engine.set_mute(muted);
                    }
                    ToHost::SetPause { paused, .. } => {
                        let _ = engine.set_pause(paused);
                    }
                    ToHost::Shutdown => {
                        engine.shutdown();
                        break;
                    }
                }
            }
            Err(_) => break,
        }
    }
}
