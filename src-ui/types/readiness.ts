/**
 * Capability Readiness Architecture & System Health Contracts (Step 66).
 *
 * Truthful readiness modeling for KinetiWall:
 * - Distinguishes between essential capabilities (required for baseline operation)
 *   and optional capabilities (enhancements like video playback, web engine, audio reactivity).
 * - Distinguishes 6 distinct states: `ready`, `initializing`, `limited`, `action_required`, `unavailable`, `unknown`.
 * - Forbids collapsing unknown/unverified states into ready.
 * - Prevents optional failures from blocking unrelated operational features.
 */

export type ReadinessState =
  | 'ready'
  | 'initializing'
  | 'limited'
  | 'action_required'
  | 'unavailable'
  | 'unknown';

export type CapabilityRequirement = 'essential' | 'optional';

export type CapabilityCategory = 'core' | 'playback' | 'integration';

export type CapabilityId =
  | 'configuration'
  | 'wallpaper_catalog'
  | 'desktop_integration'
  | 'video_player'
  | 'web_engine'
  | 'audio_reactivity';

export interface CapabilityReadiness {
  id: CapabilityId;
  label: string;
  category: CapabilityCategory;
  requirement: CapabilityRequirement;
  state: ReadinessState;
  detail: string;
  remediation?: string;
  isRetryable: boolean;
}

export interface AggregateReadinessReport {
  isOperational: boolean;
  hasWarnings: boolean;
  hasBlockers: boolean;
  overallState: ReadinessState;
  capabilities: CapabilityReadiness[];
  evaluatedAtMillis: number;
}

/**
 * Computes an aggregate readiness assessment across all evaluated capabilities.
 * Enforces architectural invariants:
 * 1. An unavailable optional feature must never mark the entire engine non-operational.
 * 2. An essential capability in `action_required` or `unavailable` marks the engine as having blockers.
 * 3. Never collapse `unknown` into `ready`.
 */
export function computeAggregateReadiness(
  capabilities: CapabilityReadiness[],
  evaluatedAtMillis = Date.now(),
): AggregateReadinessReport {
  let hasBlockers = false;
  let hasWarnings = false;
  let isInitializing = false;

  for (const cap of capabilities) {
    if (cap.requirement === 'essential') {
      if (cap.state === 'action_required' || cap.state === 'unavailable') {
        hasBlockers = true;
      } else if (cap.state === 'initializing') {
        isInitializing = true;
      } else if (cap.state === 'limited' || cap.state === 'unknown') {
        hasWarnings = true;
      }
    } else {
      // Optional capability
      if (cap.state === 'limited' || cap.state === 'unavailable' || cap.state === 'unknown') {
        hasWarnings = true;
      }
    }
  }

  let overallState: ReadinessState = 'ready';
  if (hasBlockers) {
    overallState = 'action_required';
  } else if (isInitializing) {
    overallState = 'initializing';
  } else if (hasWarnings) {
    overallState = 'limited';
  }

  return {
    isOperational: !hasBlockers,
    hasWarnings,
    hasBlockers,
    overallState,
    capabilities,
    evaluatedAtMillis,
  };
}

/**
 * User-facing presentation badge tone for a readiness state.
 */
export function getReadinessTone(
  state: ReadinessState,
): 'success' | 'warning' | 'error' | 'cyan' | 'neutral' {
  switch (state) {
    case 'ready':
      return 'success';
    case 'initializing':
      return 'cyan';
    case 'limited':
      return 'warning';
    case 'action_required':
    case 'unavailable':
      return 'error';
    case 'unknown':
    default:
      return 'neutral';
  }
}

/**
 * Accessible display label for a readiness state.
 */
export function getReadinessLabel(state: ReadinessState): string {
  switch (state) {
    case 'ready':
      return 'Ready';
    case 'initializing':
      return 'Checking...';
    case 'limited':
      return 'Limited';
    case 'action_required':
      return 'Action Required';
    case 'unavailable':
      return 'Unavailable';
    case 'unknown':
    default:
      return 'Unknown';
  }
}

/**
 * Visual icon symbol for a readiness state.
 */
export function getReadinessSymbol(state: ReadinessState): string {
  switch (state) {
    case 'ready':
      return '✓';
    case 'initializing':
      return '⏳';
    case 'limited':
      return '⚠';
    case 'action_required':
      return '!';
    case 'unavailable':
      return '✕';
    case 'unknown':
    default:
      return '?';
  }
}

