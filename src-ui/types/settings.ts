/**
/**
 * Authoritative Settings & Preferences DTOs (Step 61).
 * Strictly mirrors `KinetiWallConfig` Schema V1 (KW-DAT-004) from `crates/kinetiwall-desktop/src/settings.rs`.
 */

export type ThemePreferenceDto = 'system' | 'dark' | 'light';

export type ReducedMotionPreferenceDto = 'system' | 'always_reduce' | 'no_preference';

export type NotificationPreferenceDto = 'all' | 'errors_only' | 'disabled';

export type FullscreenActionDto = 'pause' | 'mute' | 'keep_playing';

export type WindowStateActionDto = 'pause' | 'mute' | 'keep_playing';

export type BatteryActionDto = 'pause' | 'throttle_30fps' | 'keep_playing';

export type RemoteDesktopActionDto = 'pause' | 'keep_playing';

export type StretchModeDto = 'fill' | 'fit' | 'stretch' | 'center';

export type LogLevelDto = 'error' | 'warn' | 'info' | 'debug' | 'trace';

export interface GeneralConfigDto {
  autostart: boolean;
  start_minimized: boolean;
  minimize_to_tray: boolean;
  theme: ThemePreferenceDto;
  reduced_motion: ReducedMotionPreferenceDto;
  notifications: NotificationPreferenceDto;
  language: string;
}

export interface PerformanceConfigDto {
  on_fullscreen_app: FullscreenActionDto;
  on_maximized_window: WindowStateActionDto;
  on_battery_power: BatteryActionDto;
  on_remote_desktop: RemoteDesktopActionDto;
  target_framerate_ac: number;
  target_framerate_battery: number;
  enable_adaptive_quality: boolean;
  enable_resource_budget: boolean;
  enable_display_power_pause: boolean;
  enable_inactivity_pause: boolean;
  inactivity_threshold_millis: number;
  pause_on_workstation_lock: boolean;
}

export interface RenderingConfigDto {
  hardware_acceleration: boolean;
  vsync: boolean;
  stretch_mode_default: StretchModeDto;
}

export interface AudioConfigDto {
  master_volume: number;
  master_mute: boolean;
  mute_when_other_app_plays_audio: boolean;
  output_device_id: string | null;
}

export interface DiagnosticsConfigDto {
  log_level: LogLevelDto;
  max_log_retention_days: number;
  enable_file_logging: boolean;
  enable_diagnostics_overlay: boolean;
}

export interface KinetiWallConfigDto {
  schema_version: number;
  general: GeneralConfigDto;
  performance: PerformanceConfigDto;
  rendering: RenderingConfigDto;
  audio: AudioConfigDto;
  monitors: Record<string, unknown>;
  wallpaper_properties: Record<string, unknown>;
  diagnostics: DiagnosticsConfigDto;
  [key: string]: unknown;
}

export type SettingsTab = 'general' | 'playback' | 'rendering' | 'audio' | 'diagnostics';

/**
 * Creates canonical factory default settings adhering to KW-DAT-004 Schema V1.
 */
export function createDefaultConfigDto(): KinetiWallConfigDto {
  return {
    schema_version: 1,
    general: {
      autostart: false,
      start_minimized: true,
      minimize_to_tray: true,
      theme: 'system',
      reduced_motion: 'system',
      notifications: 'all',
      language: 'en-US',
    },
    performance: {
      on_fullscreen_app: 'pause',
      on_maximized_window: 'keep_playing',
      on_battery_power: 'throttle_30fps',
      on_remote_desktop: 'pause',
      target_framerate_ac: 60,
      target_framerate_battery: 30,
      enable_adaptive_quality: true,
      enable_resource_budget: true,
      enable_display_power_pause: true,
      enable_inactivity_pause: true,
      inactivity_threshold_millis: 300000,
      pause_on_workstation_lock: true,
    },
    rendering: {
      hardware_acceleration: true,
      vsync: true,
      stretch_mode_default: 'fill',
    },
    audio: {
      master_volume: 0.5,
      master_mute: false,
      mute_when_other_app_plays_audio: true,
      output_device_id: null,
    },
    monitors: {},
    wallpaper_properties: {},
    diagnostics: {
      log_level: 'info',
      max_log_retention_days: 7,
      enable_file_logging: true,
      enable_diagnostics_overlay: false,
    },
  };
}

/**
 * Validates a configuration object against authoritative bounds.
 * Returns a map of field path to error message.
 */
export function validateSettings(config: KinetiWallConfigDto): Record<string, string> {
  const errors: Record<string, string> = {};

  if (!config.general.language || config.general.language.trim().length === 0) {
    errors['general.language'] = 'Language cannot be empty';
  }

  const validReducedMotion = ['system', 'always_reduce', 'no_preference'];
  if (config.general.reduced_motion && !validReducedMotion.includes(config.general.reduced_motion)) {
    errors['general.reduced_motion'] = `Reduced motion must be one of: ${validReducedMotion.join(', ')}`;
  }

  const validNotifications = ['all', 'errors_only', 'disabled'];
  if (config.general.notifications && !validNotifications.includes(config.general.notifications)) {
    errors['general.notifications'] = `Notification mode must be one of: ${validNotifications.join(', ')}`;
  }

  if (
    typeof config.performance.target_framerate_ac !== 'number' ||
    config.performance.target_framerate_ac < 15 ||
    config.performance.target_framerate_ac > 240
  ) {
    errors['performance.target_framerate_ac'] = 'AC target framerate must be between 15 and 240 FPS';
  }

  if (
    typeof config.performance.target_framerate_battery !== 'number' ||
    config.performance.target_framerate_battery < 15 ||
    config.performance.target_framerate_battery > 60
  ) {
    errors['performance.target_framerate_battery'] = 'Battery target framerate must be between 15 and 60 FPS';
  }

  if (
    typeof config.performance.inactivity_threshold_millis !== 'number' ||
    config.performance.inactivity_threshold_millis < 5000 ||
    config.performance.inactivity_threshold_millis > 86400000
  ) {
    errors['performance.inactivity_threshold_millis'] =
      'Inactivity threshold must be between 5 seconds and 24 hours';
  }

  if (
    typeof config.audio.master_volume !== 'number' ||
    Number.isNaN(config.audio.master_volume) ||
    config.audio.master_volume < 0.0 ||
    config.audio.master_volume > 1.0
  ) {
    errors['audio.master_volume'] = 'Master volume must be between 0% and 100%';
  }

  if (
    typeof config.diagnostics.max_log_retention_days !== 'number' ||
    config.diagnostics.max_log_retention_days < 1 ||
    config.diagnostics.max_log_retention_days > 30
  ) {
    errors['diagnostics.max_log_retention_days'] = 'Log retention must be between 1 and 30 days';
  }

  const validLogLevels = ['error', 'warn', 'info', 'debug', 'trace'];
  if (!validLogLevels.includes(config.diagnostics.log_level)) {
    errors['diagnostics.log_level'] = `Log level must be one of: ${validLogLevels.join(', ')}`;
  }

  return errors;
}

/**
 * Deep equality comparison between two configuration objects.
 */
export function areConfigsEqual(a: KinetiWallConfigDto | null, b: KinetiWallConfigDto | null): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return JSON.stringify(a) === JSON.stringify(b);
}

/**
 * Returns a list of dotted field names that differ between draft and persisted configuration.
 */
export function getDirtyFields(
  draft: KinetiWallConfigDto | null,
  persisted: KinetiWallConfigDto | null,
): string[] {
  if (!draft || !persisted) return [];
  const dirty: string[] = [];

  // General
  if (draft.general.autostart !== persisted.general.autostart) dirty.push('general.autostart');
  if (draft.general.start_minimized !== persisted.general.start_minimized) dirty.push('general.start_minimized');
  if (draft.general.minimize_to_tray !== persisted.general.minimize_to_tray) dirty.push('general.minimize_to_tray');
  if (draft.general.theme !== persisted.general.theme) dirty.push('general.theme');
  if (draft.general.reduced_motion !== persisted.general.reduced_motion) dirty.push('general.reduced_motion');
  if (draft.general.notifications !== persisted.general.notifications) dirty.push('general.notifications');
  if (draft.general.language !== persisted.general.language) dirty.push('general.language');

  // Performance
  if (draft.performance.on_fullscreen_app !== persisted.performance.on_fullscreen_app)
    dirty.push('performance.on_fullscreen_app');
  if (draft.performance.on_maximized_window !== persisted.performance.on_maximized_window)
    dirty.push('performance.on_maximized_window');
  if (draft.performance.on_battery_power !== persisted.performance.on_battery_power)
    dirty.push('performance.on_battery_power');
  if (draft.performance.on_remote_desktop !== persisted.performance.on_remote_desktop)
    dirty.push('performance.on_remote_desktop');
  if (draft.performance.target_framerate_ac !== persisted.performance.target_framerate_ac)
    dirty.push('performance.target_framerate_ac');
  if (draft.performance.target_framerate_battery !== persisted.performance.target_framerate_battery)
    dirty.push('performance.target_framerate_battery');
  if (draft.performance.enable_adaptive_quality !== persisted.performance.enable_adaptive_quality)
    dirty.push('performance.enable_adaptive_quality');
  if (draft.performance.enable_resource_budget !== persisted.performance.enable_resource_budget)
    dirty.push('performance.enable_resource_budget');
  if (draft.performance.enable_display_power_pause !== persisted.performance.enable_display_power_pause)
    dirty.push('performance.enable_display_power_pause');
  if (draft.performance.enable_inactivity_pause !== persisted.performance.enable_inactivity_pause)
    dirty.push('performance.enable_inactivity_pause');
  if (draft.performance.inactivity_threshold_millis !== persisted.performance.inactivity_threshold_millis)
    dirty.push('performance.inactivity_threshold_millis');
  if (draft.performance.pause_on_workstation_lock !== persisted.performance.pause_on_workstation_lock)
    dirty.push('performance.pause_on_workstation_lock');

  // Rendering
  if (draft.rendering.hardware_acceleration !== persisted.rendering.hardware_acceleration)
    dirty.push('rendering.hardware_acceleration');
  if (draft.rendering.vsync !== persisted.rendering.vsync) dirty.push('rendering.vsync');
  if (draft.rendering.stretch_mode_default !== persisted.rendering.stretch_mode_default)
    dirty.push('rendering.stretch_mode_default');

  // Audio
  if (Math.abs(draft.audio.master_volume - persisted.audio.master_volume) > 0.001)
    dirty.push('audio.master_volume');
  if (draft.audio.master_mute !== persisted.audio.master_mute) dirty.push('audio.master_mute');
  if (draft.audio.mute_when_other_app_plays_audio !== persisted.audio.mute_when_other_app_plays_audio)
    dirty.push('audio.mute_when_other_app_plays_audio');

  // Diagnostics
  if (draft.diagnostics.log_level !== persisted.diagnostics.log_level) dirty.push('diagnostics.log_level');
  if (draft.diagnostics.max_log_retention_days !== persisted.diagnostics.max_log_retention_days)
    dirty.push('diagnostics.max_log_retention_days');
  if (draft.diagnostics.enable_file_logging !== persisted.diagnostics.enable_file_logging)
    dirty.push('diagnostics.enable_file_logging');
  if (draft.diagnostics.enable_diagnostics_overlay !== persisted.diagnostics.enable_diagnostics_overlay)
    dirty.push('diagnostics.enable_diagnostics_overlay');

  return dirty;
}

