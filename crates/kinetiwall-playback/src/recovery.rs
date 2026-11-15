//! Crash recovery policy, retry budget tracking, and recovery outcomes.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::health::{FailureClassification, PlayerInstanceId};
use crate::types::{PlaybackError, PlaybackSessionId};

/// Configurable crash recovery policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryPolicy {
    /// Maximum number of automatic restarts permitted within the observation window.
    pub max_retries: u32,
    /// Duration of the rolling observation window (e.g. 60 seconds).
    pub window_duration: Duration,
    /// Initial backoff delay before the first retry (e.g. 100 ms).
    pub initial_backoff: Duration,
    /// Maximum backoff delay cap (e.g. 2000 ms).
    pub max_backoff: Duration,
}

impl Default for RecoveryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            window_duration: Duration::from_secs(60),
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_millis(2000),
        }
    }
}

/// Tracks recovery attempts within a rolling time window.
#[derive(Debug, Clone)]
pub struct RecoveryBudget {
    attempts: u32,
    window_start: Instant,
}

impl Default for RecoveryBudget {
    fn default() -> Self {
        Self::new()
    }
}

impl RecoveryBudget {
    #[must_use]
    pub fn new() -> Self {
        Self {
            attempts: 0,
            window_start: Instant::now(),
        }
    }

    /// Evaluates whether another recovery attempt is permitted under the policy.
    pub fn can_attempt(&mut self, policy: &RecoveryPolicy, now: Instant) -> bool {
        if now.duration_since(self.window_start) > policy.window_duration {
            // Window expired, reset budget
            self.attempts = 0;
            self.window_start = now;
        }
        self.attempts < policy.max_retries
    }

    /// Records a recovery attempt and computes the exponential backoff delay.
    pub fn record_attempt(&mut self, policy: &RecoveryPolicy, now: Instant) -> Duration {
        if now.duration_since(self.window_start) > policy.window_duration {
            self.attempts = 0;
            self.window_start = now;
        }
        self.attempts = self.attempts.saturating_add(1);

        // Exponential backoff: initial * 2^(attempts - 1), capped at max_backoff
        let exp = 1u32
            .checked_shl(self.attempts.saturating_sub(1))
            .unwrap_or(u32::MAX);
        let backoff = policy.initial_backoff.saturating_mul(exp);
        backoff.min(policy.max_backoff)
    }

    #[must_use]
    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    pub fn reset(&mut self) {
        self.attempts = 0;
        self.window_start = Instant::now();
    }
}

/// Result of a crash recovery evaluation or execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryOutcome {
    /// Successfully recovered: new instance launched and ready/playing.
    Recovered {
        new_instance_id: PlayerInstanceId,
        new_session_id: PlaybackSessionId,
    },
    /// Ineligible: playback is no longer desired (user stopped or unassigned).
    IneligibleNotDesired,
    /// Ineligible: supervisor is shutting down.
    IneligibleShuttingDown,
    /// Ineligible: failure is non-retryable (e.g. invalid asset or missing runtime).
    IneligibleNonRetryable { reason: FailureClassification },
    /// Ineligible: recovery budget exhausted (circuit breaker tripped).
    BudgetExhausted { attempts: u32 },
    /// Recovery was attempted, but launching the replacement failed.
    RecoveryFailed { error: PlaybackError },
}
