//! Compatibility policy layer and wallpaper host suitability evaluation.

use serde::{Deserialize, Serialize};

use crate::types::{
    CandidateRole, DesktopCandidateWindow, DesktopShellTopology, NativeWindowHandle,
    TopologyClassification,
};

/// Specific reason why an observed desktop topology is rejected for wallpaper attachment.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum CompatibilityRejectionReason {
    #[error("Desktop topology `{0:?}` is not supported for background wallpaper embedding: {1}")]
    TopologyNotSupported(DesktopShellTopology, String),

    #[error("No recommended wallpaper host candidate was found in the observed topology: {0}")]
    NoRecommendedCandidate(String),

    #[error("Desktop topology is ambiguous ({0} candidate windows found without unique host resolution)")]
    AmbiguousTopology(usize),

    #[error("Desktop root window is unavailable or shell is restarting")]
    DesktopRootUnavailable,

    #[error("Candidate window {0} failed native validation checks")]
    CandidateNotValidated(NativeWindowHandle),

    #[error(
        "Candidate window has unexpected role `{0:?}`; only `WallpaperHostCandidate` is permitted"
    )]
    WrongCandidateRole(CandidateRole),
}

/// Compatibility rules and evaluation policy for candidate wallpaper host windows.
pub struct CompatibilityPolicy;

impl CompatibilityPolicy {
    /// Evaluates whether an authoritative `TopologyClassification` meets all safety
    /// and architectural criteria required for future wallpaper surface attachment.
    ///
    /// # Safety Invariant
    /// Guarantees that only conclusively validated `WallpaperHostCandidate` windows
    /// in recognized split topologies are ever approved. Unsplit, ambiguous, or
    /// unverified topologies are rejected immediately.
    pub fn evaluate_attachment_suitability(
        classification: &TopologyClassification,
    ) -> Result<DesktopCandidateWindow, CompatibilityRejectionReason> {
        match classification.topology {
            DesktopShellTopology::StandardWorkerWSplit | DesktopShellTopology::ModernMultiWorkerW => {
                let Some(candidate) = &classification.recommended_candidate else {
                    return Err(CompatibilityRejectionReason::NoRecommendedCandidate(
                        classification.reason.clone(),
                    ));
                };

                if candidate.role != CandidateRole::WallpaperHostCandidate {
                    return Err(CompatibilityRejectionReason::WrongCandidateRole(candidate.role));
                }

                if candidate.handle.is_null() {
                    return Err(CompatibilityRejectionReason::CandidateNotValidated(candidate.handle));
                }

                if candidate.has_shelldll_defview_child {
                    return Err(CompatibilityRejectionReason::WrongCandidateRole(CandidateRole::IconHostCarrier));
                }

                Ok(candidate.clone())
            }
            DesktopShellTopology::ClassicUnsplit => Err(CompatibilityRejectionReason::TopologyNotSupported(
                classification.topology,
                "Classic unsplit Progman desktop does not provide an isolated background WorkerW surface. An active split probe is required before attachment can proceed.".to_string(),
            )),
            DesktopShellTopology::Ambiguous => {
                let count = classification.all_candidates.iter().filter(|c| c.role == CandidateRole::WallpaperHostCandidate).count();
                Err(CompatibilityRejectionReason::AmbiguousTopology(count))
            }
            DesktopShellTopology::DesktopRootUnavailable => {
                Err(CompatibilityRejectionReason::DesktopRootUnavailable)
            }
            DesktopShellTopology::ShellUnavailable | DesktopShellTopology::UnknownTopology => {
                Err(CompatibilityRejectionReason::TopologyNotSupported(
                    classification.topology,
                    classification.reason.clone(),
                ))
            }
        }
    }
}
