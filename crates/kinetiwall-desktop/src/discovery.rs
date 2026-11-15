//! Desktop host window discovery engine and shell hierarchy inspector.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::provider::DesktopWindowProvider;
use crate::types::{
    CandidateRole, DesktopCandidateWindow, DesktopShellTopology, DiscoveryError,
    NativeWindowHandle, TopologyClassification,
};

/// Configuration options controlling the behavior of the desktop discovery engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopDiscoveryOptions {
    /// Whether to allow sending an active probe message (e.g. `0x052C`) to Explorer
    /// if the desktop is in a passive, unsplit state.
    ///
    /// **MANDATORY DEFAULT: FALSE** (Read-only, non-invasive discovery).
    pub allow_active_probe: bool,

    /// Maximum timeout in milliseconds for an active probe if enabled.
    pub probe_timeout_ms: u32,

    /// Message ID used for the active probe if permitted. Default is `0x052C`.
    pub probe_message_id: u32,

    /// Whether to revalidate all observed window handles prior to returning the snapshot.
    pub revalidate_handles: bool,
}

impl Default for DesktopDiscoveryOptions {
    fn default() -> Self {
        Self {
            allow_active_probe: false,
            probe_timeout_ms: 1000,
            probe_message_id: 0x052C,
            revalidate_handles: true,
        }
    }
}

/// Service that discovers and analyzes Windows desktop shell window hierarchies.
pub struct DesktopHostDiscovery {
    provider: Arc<dyn DesktopWindowProvider>,
}

impl DesktopHostDiscovery {
    #[must_use]
    pub fn new(provider: Arc<dyn DesktopWindowProvider>) -> Self {
        Self { provider }
    }

    /// Primary discovery entry point: inspects the desktop window hierarchy and classifies
    /// the observed shell topology according to deterministic validation rules.
    pub fn discover_topology(
        &self,
        options: &DesktopDiscoveryOptions,
    ) -> Result<TopologyClassification, DiscoveryError> {
        let now_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // 1. Locate Progman root window
        let progman_opt = self.provider.find_window(Some("Progman"), None);

        // 2. Locate SHELLDLL_DefView icon view container
        let mut defview_opt = None;
        let mut icon_carrier_opt = None;

        if let Some(progman) = progman_opt {
            if let Some(dv) =
                self.provider
                    .find_window_ex(progman, None, Some("SHELLDLL_DefView"), None)
            {
                defview_opt = Some(dv);
                icon_carrier_opt = Some(progman);
            }
        }

        // If SHELLDLL_DefView is not under Progman, check top-level WorkerW windows
        let top_windows = self.provider.enum_windows();

        if defview_opt.is_none() {
            for &hwnd in &top_windows {
                if let Ok(class) = self.provider.get_class_name(hwnd) {
                    if class.eq_ignore_ascii_case("WorkerW") {
                        if let Some(dv) =
                            self.provider
                                .find_window_ex(hwnd, None, Some("SHELLDLL_DefView"), None)
                        {
                            defview_opt = Some(dv);
                            icon_carrier_opt = Some(hwnd);
                            break;
                        }
                    }
                }
            }
        }

        // If neither Progman nor any WorkerW exists at all on the desktop:
        if progman_opt.is_none() && icon_carrier_opt.is_none() {
            let any_workerw = top_windows.iter().any(|&h| {
                self.provider
                    .get_class_name(h)
                    .is_ok_and(|c| c.eq_ignore_ascii_case("WorkerW"))
            });
            if !any_workerw {
                return Ok(TopologyClassification {
                    topology: DesktopShellTopology::DesktopRootUnavailable,
                    recommended_candidate: None,
                    all_candidates: Vec::new(),
                    icon_carrier: None,
                    icon_view: None,
                    observed_at_millis: now_millis,
                    is_safe_for_attachment_review: false,
                    reason: "Neither Progman nor any WorkerW desktop root window was found on the desktop.".to_string(),
                });
            }
        }

        // Check if SHELLDLL_DefView was missing
        let Some(defview) = defview_opt else {
            return Ok(TopologyClassification {
                topology: DesktopShellTopology::UnknownTopology,
                recommended_candidate: None,
                all_candidates: Vec::new(),
                icon_carrier: None,
                icon_view: None,
                observed_at_millis: now_millis,
                is_safe_for_attachment_review: false,
                reason: "SHELLDLL_DefView desktop icon view container could not be located."
                    .to_string(),
            });
        };

        let icon_carrier_hwnd = icon_carrier_opt.unwrap_or(NativeWindowHandle::NULL);

        // 3. Collect top-level WorkerW windows and classify candidates
        let mut worker_candidates = Vec::new();
        let mut all_observed = Vec::new();

        // Register Progman if found
        if let Some(progman) = progman_opt {
            let (tid, pid) = self.provider.get_window_thread_process_id(progman);
            let bounds = self.provider.get_window_rect(progman).ok();
            let is_vis = self.provider.is_window_visible(progman);
            let has_dv = icon_carrier_hwnd == progman;
            all_observed.push(DesktopCandidateWindow {
                handle: progman,
                class_name: "Progman".to_string(),
                window_title: self.provider.get_window_text(progman).unwrap_or_default(),
                process_id: pid,
                thread_id: tid,
                role: if has_dv {
                    CandidateRole::IconHostCarrier
                } else {
                    CandidateRole::DesktopRoot
                },
                parent: None,
                bounds,
                is_visible: is_vis,
                has_shelldll_defview_child: has_dv,
                validated_at_millis: now_millis,
            });
        }

        // Register icon view
        let (dv_tid, dv_pid) = self.provider.get_window_thread_process_id(defview);
        let dv_bounds = self.provider.get_window_rect(defview).ok();
        let dv_vis = self.provider.is_window_visible(defview);
        let icon_view_cand = DesktopCandidateWindow {
            handle: defview,
            class_name: "SHELLDLL_DefView".to_string(),
            window_title: self.provider.get_window_text(defview).unwrap_or_default(),
            process_id: dv_pid,
            thread_id: dv_tid,
            role: CandidateRole::IconView,
            parent: Some(icon_carrier_hwnd),
            bounds: dv_bounds,
            is_visible: dv_vis,
            has_shelldll_defview_child: false,
            validated_at_millis: now_millis,
        };

        // Scan all top-level windows for WorkerW instances
        for &hwnd in &top_windows {
            if let Ok(class) = self.provider.get_class_name(hwnd) {
                if class.eq_ignore_ascii_case("WorkerW") {
                    let has_dv = self
                        .provider
                        .find_window_ex(hwnd, None, Some("SHELLDLL_DefView"), None)
                        .is_some();
                    let (tid, pid) = self.provider.get_window_thread_process_id(hwnd);
                    let bounds = self.provider.get_window_rect(hwnd).ok();
                    let is_vis = self.provider.is_window_visible(hwnd);
                    let title = self.provider.get_window_text(hwnd).unwrap_or_default();

                    let role = if has_dv {
                        CandidateRole::IconHostCarrier
                    } else {
                        CandidateRole::WallpaperHostCandidate
                    };

                    let candidate = DesktopCandidateWindow {
                        handle: hwnd,
                        class_name: class,
                        window_title: title,
                        process_id: pid,
                        thread_id: tid,
                        role,
                        parent: None,
                        bounds,
                        is_visible: is_vis,
                        has_shelldll_defview_child: has_dv,
                        validated_at_millis: now_millis,
                    };

                    if !has_dv {
                        worker_candidates.push(candidate.clone());
                    }
                    all_observed.push(candidate);
                }
            }
        }

        // 4. Handle Case 1: No secondary WorkerW windows exist (Classic Unsplit state)
        if worker_candidates.is_empty() {
            let icon_carrier_snapshot =
                progman_opt.and_then(|p| all_observed.iter().find(|c| c.handle == p).cloned());
            if !options.allow_active_probe {
                // Passive discovery only: report ClassicUnsplit truthfully without altering shell
                return Ok(TopologyClassification {
                    topology: DesktopShellTopology::ClassicUnsplit,
                    recommended_candidate: None,
                    all_candidates: all_observed,
                    icon_carrier: icon_carrier_snapshot,
                    icon_view: Some(icon_view_cand),
                    observed_at_millis: now_millis,
                    is_safe_for_attachment_review: false,
                    reason: "Classic unsplit desktop shell observed: SHELLDLL_DefView is inside Progman with no background WorkerW. Passive discovery does not dispatch active probe messages without explicit opt-in.".to_string(),
                });
            }

            // Optional Active Probe allowed: dispatch bounded 0x052C to Progman
            if let Some(progman) = progman_opt {
                let probe_res = self.provider.send_message_timeout(
                    progman,
                    options.probe_message_id,
                    0x0000000D,
                    0,
                    options.probe_timeout_ms,
                );

                if let Err(e) = probe_res {
                    return Ok(TopologyClassification {
                        topology: DesktopShellTopology::ClassicUnsplit,
                        recommended_candidate: None,
                        all_candidates: all_observed,
                        icon_carrier: icon_carrier_snapshot,
                        icon_view: Some(icon_view_cand),
                        observed_at_millis: now_millis,
                        is_safe_for_attachment_review: false,
                        reason: format!(
                            "Active probe 0x{:04X} to Progman failed: {}",
                            options.probe_message_id, e
                        ),
                    });
                }

                // Re-scan top-level windows after active probe
                let post_probe_top = self.provider.enum_windows();
                worker_candidates.clear();
                for &hwnd in &post_probe_top {
                    if let Ok(class) = self.provider.get_class_name(hwnd) {
                        if class.eq_ignore_ascii_case("WorkerW") {
                            let has_dv = self
                                .provider
                                .find_window_ex(hwnd, None, Some("SHELLDLL_DefView"), None)
                                .is_some();
                            if !has_dv {
                                let (tid, pid) = self.provider.get_window_thread_process_id(hwnd);
                                let bounds = self.provider.get_window_rect(hwnd).ok();
                                let is_vis = self.provider.is_window_visible(hwnd);
                                let title = self.provider.get_window_text(hwnd).unwrap_or_default();
                                worker_candidates.push(DesktopCandidateWindow {
                                    handle: hwnd,
                                    class_name: class,
                                    window_title: title,
                                    process_id: pid,
                                    thread_id: tid,
                                    role: CandidateRole::WallpaperHostCandidate,
                                    parent: None,
                                    bounds,
                                    is_visible: is_vis,
                                    has_shelldll_defview_child: false,
                                    validated_at_millis: now_millis,
                                });
                            }
                        }
                    }
                }
            }
        }

        // 5. Evaluate remaining candidate WorkerW windows
        let icon_carrier_rec = all_observed
            .iter()
            .find(|c| c.handle == icon_carrier_hwnd)
            .cloned();

        // Filter and revalidate handles if requested
        if options.revalidate_handles {
            worker_candidates.retain(|c| self.revalidate_candidate(c));
        }

        match worker_candidates.len() {
            0 => Ok(TopologyClassification {
                topology: DesktopShellTopology::ClassicUnsplit,
                recommended_candidate: None,
                all_candidates: all_observed,
                icon_carrier: icon_carrier_rec,
                icon_view: Some(icon_view_cand),
                observed_at_millis: now_millis,
                is_safe_for_attachment_review: false,
                reason: "No secondary WorkerW host windows found behind desktop icons.".to_string(),
            }),
            1 => {
                let candidate = worker_candidates[0].clone();
                Ok(TopologyClassification {
                    topology: DesktopShellTopology::StandardWorkerWSplit,
                    recommended_candidate: Some(candidate),
                    all_candidates: all_observed,
                    icon_carrier: icon_carrier_rec,
                    icon_view: Some(icon_view_cand),
                    observed_at_millis: now_millis,
                    is_safe_for_attachment_review: true,
                    reason: "Standard split WorkerW desktop topology detected: confirmed single WorkerW host behind desktop icon view.".to_string(),
                })
            }
            _ => {
                // Multiple WorkerW instances found. Look for the specific WorkerW immediately behind the icon carrier.
                // In Win32 Z-order (top to bottom returned by EnumWindows), the wallpaper host WorkerW
                // is typically positioned immediately below the window containing SHELLDLL_DefView.
                let mut host_candidate = None;
                for i in 0..top_windows.len().saturating_sub(1) {
                    if top_windows[i] == icon_carrier_hwnd {
                        let next_hwnd = top_windows[i + 1];
                        if let Some(matched) =
                            worker_candidates.iter().find(|c| c.handle == next_hwnd)
                        {
                            host_candidate = Some(matched.clone());
                            break;
                        }
                    }
                }

                if let Some(matched) = host_candidate {
                    Ok(TopologyClassification {
                        topology: DesktopShellTopology::ModernMultiWorkerW,
                        recommended_candidate: Some(matched),
                        all_candidates: all_observed,
                        icon_carrier: icon_carrier_rec,
                        icon_view: Some(icon_view_cand),
                        observed_at_millis: now_millis,
                        is_safe_for_attachment_review: true,
                        reason: "Modern multi-WorkerW desktop topology detected: successfully identified unique WorkerW sitting immediately behind the icon carrier.".to_string(),
                    })
                } else {
                    Ok(TopologyClassification {
                        topology: DesktopShellTopology::Ambiguous,
                        recommended_candidate: None,
                        all_candidates: all_observed,
                        icon_carrier: icon_carrier_rec,
                        icon_view: Some(icon_view_cand),
                        observed_at_millis: now_millis,
                        is_safe_for_attachment_review: false,
                        reason: format!(
                            "Ambiguous desktop topology: found {} candidate WorkerW windows without a definitive Z-order match immediately behind the icon carrier.",
                            worker_candidates.len()
                        ),
                    })
                }
            }
        }
    }

    /// Verifies whether an observed candidate window handle currently exists,
    /// matches its recorded window class, and is owned by the same process.
    pub fn revalidate_candidate(&self, candidate: &DesktopCandidateWindow) -> bool {
        if candidate.handle.is_null() {
            return false;
        }

        if !self.provider.is_window(candidate.handle) {
            return false;
        }

        // Verify class name has not changed (detect handle reuse)
        if let Ok(class) = self.provider.get_class_name(candidate.handle) {
            if !class.eq_ignore_ascii_case(&candidate.class_name) {
                return false;
            }
        } else {
            return false;
        }

        // Verify process ownership has not changed
        let (_, pid) = self.provider.get_window_thread_process_id(candidate.handle);
        if candidate.process_id != 0 && pid != candidate.process_id {
            return false;
        }

        true
    }
}
