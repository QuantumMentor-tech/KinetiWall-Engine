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


