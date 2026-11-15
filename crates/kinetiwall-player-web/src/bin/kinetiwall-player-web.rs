//! Standalone isolated web wallpaper player process executable.
//!
//! Communicates with the KinetiWall controller via line-delimited JSON on stdin/stdout.

use std::io::{self, BufRead, Write};
use std::time::Duration;

use kinetiwall_playback::PlayerReport;
use kinetiwall_player_web::engine::create_web_engine;
use kinetiwall_player_web::protocol::{FromHost, HostOp, ToHost};
use kinetiwall_player_web::validate::validate_web_wallpaper_path;

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
    let mut engine = create_web_engine();
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
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
                        entrypoint_path,
                    } => match validate_web_wallpaper_path(&entrypoint_path) {
                        Ok(validated) => {
                            let res = engine.load_wallpaper(&validated);
                            send_msg(&FromHost::Ack {
                                session_id,
                                op: HostOp::Start,
                                ok: res.is_ok(),
                                error: res.err(),
                            });

                            if res.is_ok() {
                                // Drain initial events
                                for _ in 0..10 {
                                    if let Some(event) =
                                        engine.wait_event(Duration::from_millis(50))
                                    {
                                        match event {
                                            kinetiwall_player_web::engine::WebEngineEvent::RenderingStarted => {
                                                send_msg(&FromHost::Report {
                                                    session_id,
                                                    report: PlayerReport::Playing,
                                                });
                                                break;
                                            }
                                            kinetiwall_player_web::engine::WebEngineEvent::PlaybackFailed(kind) => {
                                                send_msg(&FromHost::Report {
                                                    session_id,
                                                    report: PlayerReport::Failed { kind },
                                                });
                                                break;
                                            }
                                            kinetiwall_player_web::engine::WebEngineEvent::SecurityViolation(detail) => {
                                                send_msg(&FromHost::SecurityNotice {
                                                    session_id,
                                                    detail,
                                                });
                                            }
                                            _ => {}
                                        }
                                    }
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
                    },
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
                    ToHost::VisualizerUpdate {
                        session_id: _,
                        frame,
                    } => {
                        let _ = engine.post_visualizer_frame(&frame);
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
