/**
 * Multi-Monitor Display, Assignment & Layout Domain Types (Step 46).
 * Aligns frontend contracts with `kinetiwall-desktop` Steps 42, 43, 44, and 45.
 */

export type DisplayId = string;

export interface DisplayGeometryDto {
  left: number;
  top: number;
  right: number;
  bottom: number;
  width: number;
  height: number;
  scaleFactorPercent?: number;
}

export interface DisplayDescriptorDto {
  id: DisplayId;
  deviceName: string;
  friendlyName?: string;
  hardwareId?: string;
  isPrimary: boolean;
  isAttached: boolean;
  geometry: DisplayGeometryDto;
}

export interface DisplayTopologySnapshotDto {
  generation: number;
  displays: DisplayDescriptorDto[];
  primaryDisplayId?: DisplayId;
  virtualDesktopBounds: {
    left: number;
    top: number;
    right: number;
    bottom: number;
    width: number;
    height: number;
  };
  topologyType: 'single_display' | 'dual_extended' | 'multi_extended' | 'cloned_or_mirrored' | 'unknown';
}

export type MultiMonitorLayoutMode = 'independent' | 'span' | 'clone';

export type PlaybackSyncLevel = 'none' | 'best_effort' | 'shared_timeline';

export type DisplayPresenceStatus = 'connected' | 'disconnected' | 'ambiguous' | 'session_scoped_only';

export type AssignmentResolutionOutcome =
  | 'valid_explicit'
  | 'valid_global_fallback'
  | 'unassigned'
  | 'wallpaper_missing'
  | 'wallpaper_unavailable'
  | 'display_disconnected'
  | 'display_ambiguous'
  | 'incompatible_content_type'
  | 'conflicting';

export interface ResolvedDisplayAssignmentDto {
  displayId: DisplayId;
  displayPresence: DisplayPresenceStatus;
  targetGeometry?: DisplayGeometryDto;
  wallpaperId?: string;
  assignmentKind?: 'explicit' | 'global_fallback';
  outcome: AssignmentResolutionOutcome;
  isPlayable: boolean;
  effectiveTitle?: string;
  effectiveType?: 'video' | 'web';
  snapshotGeneration: number;
}

export type DisplaySessionState =
  | 'uninitialized'
  | 'starting'
  | 'active'
  | 'detached'
  | 'suspended'
  | 'static_fallback'
  | 'failed'
  | 'stopping'
  | 'stopped';

export interface DisplaySessionStatusDto {
  sessionId: string;
  displayId: DisplayId;
  wallpaperId: string;
  state: DisplaySessionState;
  fallbackSource?: string;
  failureReason?: string;
  effectiveQualityLevel?: AdaptiveQualityLevelDto;
  effectiveFps?: number;
  geometry: DisplayGeometryDto;
  generation: number;
  createdAtMillis: number;
  lastUpdatedMillis: number;
}

export interface MultiMonitorStatusDto {
  savedMode: MultiMonitorLayoutMode;
  activeMode?: MultiMonitorLayoutMode;
  sessions: DisplaySessionStatusDto[];
  lastUpdatedMillis: number;
}

export interface DisplayLayoutPlanValidation {
  outcome: 'valid' | 'single_display_only' | 'conflicting_clone_assignments' | 'invalid_span_geometry' | 'topology_incompatible' | 'missing_wallpaper_asset';
  reason?: string;
  isActivatable: boolean;
}

export type FullscreenPauseMode = 'per_display' | 'all_displays';

export interface FullscreenPlaybackSettingsDto {
  enabled: boolean;
  pauseMode: FullscreenPauseMode;
  debounceDurationMillis: number;
}

export type PowerSource = 'ac' | 'battery' | 'unknown';
export type BatteryAvailability = 'present' | 'no_battery' | 'unknown';
export type BatteryChargingState = 'charging' | 'discharging' | 'not_charging' | 'no_battery' | 'unknown';
export type BatterySaverState = 'disabled' | 'enabled' | 'unknown';

export interface SystemPowerSnapshotDto {
  powerSource: PowerSource;
  batteryAvailability: BatteryAvailability;
  batteryPercent?: number;
  chargingState: BatteryChargingState;
  batterySaver: BatterySaverState;
  isSystemSuspended: boolean;
  sampledAtMillis: number;
}

export type PowerPolicyMode = 'off' | 'balanced' | 'save_battery';

export interface PowerPlaybackSettingsDto {
  mode: PowerPolicyMode;
  pauseOnBattery: boolean;
  pauseOnBatterySaver: boolean;
  lowBatteryThresholdPercent?: number;
  pauseOnSystemSuspend: boolean;
  enableStaticFallback: boolean;
}

export type AdaptiveQualityLevelDto = 'normal' | 'reduced' | 'minimum_eligible';

export interface AdaptiveQualitySettingsDto {
  enabled: boolean;
  targetFpsFloor: number;
  sustainedPressureThresholdMs: number;
  recoveryDwellThresholdMs: number;
  cpuPressureThresholdPercent: number;
  cpuRecoveryThresholdPercent: number;
  adjustmentCooldownMs: number;
  allowPowerPolicyCoordination: boolean;
}

export type AggregateBudgetStateDto =
  | 'disabled'
  | 'within_budget'
  | 'approaching_budget'
  | 'sustained_pressure'
  | 'coverage_incomplete'
  | 'optimization_active'
  | 'target_infeasible'
  | 'recovering';

export interface ApplicationResourceBudgetDto {
  enabled: boolean;
  targetAggregateCpuPercent: number;
  pressureThresholdCpuPercent: number;
  recoveryThresholdCpuPercent: number;
  sustainedPressureThresholdMs: number;
  recoveryDwellThresholdMs: number;
  adjustmentCooldownMs: number;
  maxSimultaneousAdjustments: number;
  allowPowerCoordination: boolean;
}

export interface AggregateWorkloadSnapshotDto {
  aggregateCpuPercent: number;
  totalWorkingSetBytes: number;
  totalSessionsCount: number;
  measuredSessionsCount: number;
  unknownSessionsCount: number;
  isCoverageComplete: boolean;
  timestampMillis: number;
}

export type WorkstationLockStateDto = 'locked' | 'unlocked' | 'unknown';

export type SessionConnectionStateDto =
  | 'console_connected'
  | 'console_disconnected'
  | 'remote_connected'
  | 'remote_disconnected'
  | 'unknown';

export interface WindowsSessionSnapshotDto {
  lockState: WorkstationLockStateDto;
  connectionState: SessionConnectionStateDto;
  sessionId?: number;
  isRegistered: boolean;
  lastEventTimestampMillis: number;
}

export interface SessionPlaybackSettingsDto {
  enabled: boolean;
  pauseOnLock: boolean;
  optimizeRemoteSession: boolean;
  enableStaticFallback: boolean;
}

export type DisplayPowerStateDto = 'off' | 'on' | 'dimmed' | 'unknown';

export type DisplayPowerScopeDto = 'session_display_status' | 'console_display_state';

export type DisplayDimmedActionDto = 'do_nothing' | 'pause';

export interface DisplayPowerSnapshotDto {
  state: DisplayPowerStateDto;
  scope: DisplayPowerScopeDto;
  sampledAtMillis: number;
  isStale: boolean;
}

export interface DisplayPowerPlaybackSettingsDto {
  enabled: boolean;
  pauseOnDisplayOff: boolean;
  resumeOnDisplayOn: boolean;
  dimmedAction: DisplayDimmedActionDto;
  enableStaticFallback: boolean;
}

export type InactivityStateDto = 'active' | 'idle' | 'transitioning' | 'unknown' | 'stale';

export interface InactivitySnapshotDto {
  state: InactivityStateDto;
  elapsedInactivityMillis: number;
  idleThresholdMillis: number;
  sampledAtMillis: number;
  isFresh: boolean;
}

export interface InactivityPlaybackSettingsDto {
  enabled: boolean;
  idleThresholdMillis: number;
  resumeOnActivity: boolean;
  enableStaticFallback: boolean;
}






export type DesiredPlaybackStateDto = 'active' | 'suspended' | 'static_fallback' | 'terminated';

export type PolicyExplanationCodeDto =
  | 'manual_pause_active'
  | 'session_terminated'
  | 'backend_failure'
  | 'fullscreen_active'
  | 'workstation_locked'
  | 'display_power_off'
  | 'user_inactivity_idle'
  | 'power_saving_active'
  | 'system_suspended'
  | 'remote_session_optimized'
  | 'resource_budget_pressure'
  | 'adaptive_quality_pacing'
  | 'static_fallback_presented'
  | 'unconstrained_active'
  | 'unknown_state_preserved';

export interface EffectivePlaybackDecisionDto {
  displayId: DisplayId;
  sessionId?: string;
  desiredState: DesiredPlaybackStateDto;
  actualState: DisplaySessionState;
  effectiveFpsLimit?: number;
  explanationCodes: PolicyExplanationCodeDto[];
  primaryExplanation: string;
  primaryReasonCode: string;
  isStateDivergent: boolean;
  requiresTransition: boolean;
  evaluatedAtMillis: number;
}
