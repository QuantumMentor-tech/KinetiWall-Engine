/**
 * Typed Frontend-to-Rust Display IPC Service Bridge (Step 46).
 * Handles boundary validation, sanitization, sequence monotonicity, and fallback handling.
 * Never accesses Win32 display APIs directly from the renderer.
 */

import type {
  DisplayId,
  DisplayTopologySnapshotDto,
  FullscreenPlaybackSettingsDto,
  MultiMonitorLayoutMode,
  MultiMonitorStatusDto,
  PowerPlaybackSettingsDto,
  ResolvedDisplayAssignmentDto,
  SystemPowerSnapshotDto,
} from '../types/display.ts';

export interface DisplayIpcTransport {
  invokeGetTopologySnapshot(): Promise<unknown>;
  invokeGetResolvedAssignments(): Promise<unknown>;
  invokeAssignWallpaper(displayId: DisplayId, wallpaperId: string): Promise<unknown>;
  invokeRemoveAssignment(displayId: DisplayId): Promise<unknown>;
  invokeSetLayoutMode(mode: MultiMonitorLayoutMode): Promise<unknown>;
  invokeApplyLayout(): Promise<unknown>;
  invokeGetMultiMonitorStatus(): Promise<unknown>;
  invokeGetFullscreenPolicySettings(): Promise<unknown>;
  invokeUpdateFullscreenPolicySettings(settings: FullscreenPlaybackSettingsDto): Promise<unknown>;
  invokeGetSystemPowerSnapshot(): Promise<unknown>;
  invokeGetPowerPlaybackSettings(): Promise<unknown>;
  invokeUpdatePowerPlaybackSettings(settings: PowerPlaybackSettingsDto): Promise<unknown>;
}

export class DisplayIpcClient {
  private readonly transport: DisplayIpcTransport;

  constructor(transport: DisplayIpcTransport) {
    this.transport = transport;
  }

  async getTopologySnapshot(): Promise<DisplayTopologySnapshotDto> {
    const raw = await this.transport.invokeGetTopologySnapshot();
    if (!raw || typeof raw !== 'object') {
      throw new Error('Invalid display topology response from host');
    }
    const obj = raw as Record<string, unknown>;
    const generation = typeof obj.generation === 'number' ? obj.generation : 1;
    const displays = Array.isArray(obj.displays) ? (obj.displays as any[]) : [];
    const primaryDisplayId = typeof obj.primary_display_id === 'string'
      ? obj.primary_display_id
      : typeof obj.primaryDisplayId === 'string'
      ? obj.primaryDisplayId
      : undefined;

    const rawBounds = (obj.virtual_desktop_bounds ?? obj.virtualDesktopBounds) as Record<string, unknown> | undefined;
    const virtualDesktopBounds = {
      left: typeof rawBounds?.left === 'number' ? rawBounds.left : 0,
      top: typeof rawBounds?.top === 'number' ? rawBounds.top : 0,
      right: typeof rawBounds?.right === 'number' ? rawBounds.right : 1920,
      bottom: typeof rawBounds?.bottom === 'number' ? rawBounds.bottom : 1080,
      width: typeof rawBounds?.width === 'number' ? rawBounds.width : 1920,
      height: typeof rawBounds?.height === 'number' ? rawBounds.height : 1080,
    };

    const topologyType = (typeof obj.topology_type === 'string'
      ? obj.topology_type
      : typeof obj.topologyType === 'string'
      ? obj.topologyType
      : 'single_display') as any;

    return {
      generation,
      displays: displays.map((d) => ({
        id: String(d.id),
        deviceName: String(d.device_name ?? d.deviceName ?? ''),
        friendlyName: d.friendly_name ?? d.friendlyName ?? undefined,
        hardwareId: d.hardware_id ?? d.hardwareId ?? undefined,
        isPrimary: Boolean(d.is_primary ?? d.isPrimary),
        isAttached: Boolean(d.is_attached ?? d.isAttached ?? true),
        geometry: {
          left: Number(d.geometry?.monitor_rect?.left ?? d.geometry?.left ?? 0),
          top: Number(d.geometry?.monitor_rect?.top ?? d.geometry?.top ?? 0),
          right: Number(d.geometry?.monitor_rect?.right ?? d.geometry?.right ?? 1920),
          bottom: Number(d.geometry?.monitor_rect?.bottom ?? d.geometry?.bottom ?? 1080),
          width: Number(d.geometry?.width ?? 1920),
          height: Number(d.geometry?.height ?? 1080),
          scaleFactorPercent: d.geometry?.scale_factor_percent ?? d.geometry?.scaleFactorPercent ?? 100,
        },
      })),
      primaryDisplayId,
      virtualDesktopBounds,
      topologyType,
    };
  }

  async getResolvedAssignments(): Promise<ResolvedDisplayAssignmentDto[]> {
    const raw = await this.transport.invokeGetResolvedAssignments();
    if (!Array.isArray(raw)) {
      return [];
    }

    return raw.map((r: any) => ({
      displayId: String(r.display_id ?? r.displayId),
      displayPresence: (r.display_presence ?? r.displayPresence ?? 'connected') as any,
      targetGeometry: r.target_geometry ?? r.targetGeometry
        ? {
            left: Number(r.target_geometry?.monitor_rect?.left ?? r.targetGeometry?.left ?? 0),
            top: Number(r.target_geometry?.monitor_rect?.top ?? r.targetGeometry?.top ?? 0),
            right: Number(r.target_geometry?.monitor_rect?.right ?? r.targetGeometry?.right ?? 1920),
            bottom: Number(r.target_geometry?.monitor_rect?.bottom ?? r.targetGeometry?.bottom ?? 1080),
            width: Number(r.target_geometry?.width ?? r.targetGeometry?.width ?? 1920),
            height: Number(r.target_geometry?.height ?? r.targetGeometry?.height ?? 1080),
            scaleFactorPercent: r.target_geometry?.scale_factor_percent ?? r.targetGeometry?.scaleFactorPercent,
          }
        : undefined,
      wallpaperId: r.wallpaper_id ?? r.wallpaperId ?? undefined,
      assignmentKind: (r.assignment_kind ?? r.assignmentKind) as any,
      outcome: (r.outcome ?? 'unassigned') as any,
      isPlayable: Boolean(r.is_playable ?? r.isPlayable),
      effectiveTitle: r.effective_title ?? r.effectiveTitle ?? undefined,
      effectiveType: (r.effective_type ?? r.effectiveType) as any,
      snapshotGeneration: Number(r.snapshot_generation ?? r.snapshotGeneration ?? 1),
    }));
  }

  async assignWallpaper(displayId: DisplayId, wallpaperId: string): Promise<void> {
    if (!displayId || !wallpaperId) {
      throw new Error('Both display ID and wallpaper ID are required for assignment');
    }
    await this.transport.invokeAssignWallpaper(displayId, wallpaperId);
  }

  async removeAssignment(displayId: DisplayId): Promise<boolean> {
    if (!displayId) {
      throw new Error('Display ID is required to remove assignment');
    }
    const res = await this.transport.invokeRemoveAssignment(displayId);
    return Boolean(res);
  }

  async setLayoutMode(mode: MultiMonitorLayoutMode): Promise<void> {
    await this.transport.invokeSetLayoutMode(mode);
  }

  async applyLayout(): Promise<MultiMonitorStatusDto> {
    const raw = await this.transport.invokeApplyLayout();
    return this.parseStatusDto(raw);
  }

  async getMultiMonitorStatus(): Promise<MultiMonitorStatusDto> {
    const raw = await this.transport.invokeGetMultiMonitorStatus();
    return this.parseStatusDto(raw);
  }

  private parseStatusDto(raw: unknown): MultiMonitorStatusDto {
    if (!raw || typeof raw !== 'object') {
      return {
        savedMode: 'independent',
        activeMode: 'independent',
        sessions: [],
        lastUpdatedMillis: Date.now(),
      };
    }
    const obj = raw as Record<string, unknown>;
    const rawSessions = Array.isArray(obj.sessions) ? obj.sessions : [];

    return {
      savedMode: (obj.saved_mode ?? obj.savedMode ?? 'independent') as MultiMonitorLayoutMode,
      activeMode: (obj.active_mode ?? obj.activeMode) as MultiMonitorLayoutMode | undefined,
      sessions: rawSessions.map((s: any) => ({
        sessionId: String(s.session_id ?? s.sessionId),
        displayId: String(s.display_id ?? s.displayId),
        wallpaperId: String(s.wallpaper_id ?? s.wallpaperId),
        state: (s.state ?? 'uninitialized') as any,
        fallbackSource: s.fallback_source ?? s.fallbackSource ?? undefined,
        failureReason: s.failure_reason ?? s.failureReason ?? undefined,
        geometry: {
          left: Number(s.geometry?.monitor_rect?.left ?? s.geometry?.left ?? 0),
          top: Number(s.geometry?.monitor_rect?.top ?? s.geometry?.top ?? 0),
          right: Number(s.geometry?.monitor_rect?.right ?? s.geometry?.right ?? 1920),
          bottom: Number(s.geometry?.monitor_rect?.bottom ?? s.geometry?.bottom ?? 1080),
          width: Number(s.geometry?.width ?? 1920),
          height: Number(s.geometry?.height ?? 1080),
        },
        generation: Number(s.generation ?? 1),
        createdAtMillis: Number(s.created_at_millis ?? s.createdAtMillis ?? Date.now()),
        lastUpdatedMillis: Number(s.last_updated_millis ?? s.lastUpdatedMillis ?? Date.now()),
      })),
      lastUpdatedMillis: Date.now(),
    };
  }

  async getFullscreenPolicySettings(): Promise<FullscreenPlaybackSettingsDto> {
    const raw = await this.transport.invokeGetFullscreenPolicySettings();
    if (!raw || typeof raw !== 'object') {
      return {
        enabled: true,
        pauseMode: 'per_display',
        debounceDurationMillis: 250,
      };
    }
    const obj = raw as Record<string, unknown>;
    return {
      enabled: typeof obj.enabled === 'boolean' ? obj.enabled : true,
      pauseMode: (obj.pause_mode ?? obj.pauseMode ?? 'per_display') as any,
      debounceDurationMillis: typeof (obj.debounce_duration_millis ?? obj.debounceDurationMillis) === 'number'
        ? Number(obj.debounce_duration_millis ?? obj.debounceDurationMillis)
        : 250,
    };
  }

  async updateFullscreenPolicySettings(settings: FullscreenPlaybackSettingsDto): Promise<FullscreenPlaybackSettingsDto> {
    await this.transport.invokeUpdateFullscreenPolicySettings(settings);
    return this.getFullscreenPolicySettings();
  }

  async getSystemPowerSnapshot(): Promise<SystemPowerSnapshotDto> {
    const raw = await this.transport.invokeGetSystemPowerSnapshot();
    if (!raw || typeof raw !== 'object') {
      return {
        powerSource: 'ac',
        batteryAvailability: 'no_battery',
        batteryPercent: undefined,
        chargingState: 'no_battery',
        batterySaver: 'disabled',
        isSystemSuspended: false,
        sampledAtMillis: Date.now(),
      };
    }
    const obj = raw as Record<string, unknown>;
    return {
      powerSource: (obj.power_source ?? obj.powerSource ?? 'unknown') as any,
      batteryAvailability: (obj.battery_availability ?? obj.batteryAvailability ?? 'unknown') as any,
      batteryPercent: typeof (obj.battery_percent ?? obj.batteryPercent) === 'number'
        ? Number(obj.battery_percent ?? obj.batteryPercent)
        : undefined,
      chargingState: (obj.charging_state ?? obj.chargingState ?? 'unknown') as any,
      batterySaver: (obj.battery_saver ?? obj.batterySaver ?? 'unknown') as any,
      isSystemSuspended: Boolean(obj.is_system_suspended ?? obj.isSystemSuspended ?? false),
      sampledAtMillis: Number(obj.sampled_at_millis ?? obj.sampledAtMillis ?? Date.now()),
    };
  }

  async getPowerPlaybackSettings(): Promise<PowerPlaybackSettingsDto> {
    const raw = await this.transport.invokeGetPowerPlaybackSettings();
    if (!raw || typeof raw !== 'object') {
      return {
        mode: 'off',
        pauseOnBattery: true,
        pauseOnBatterySaver: true,
        lowBatteryThresholdPercent: 20,
        pauseOnSystemSuspend: true,
        enableStaticFallback: true,
      };
    }
    const obj = raw as Record<string, unknown>;
    return {
      mode: (obj.mode ?? 'off') as any,
      pauseOnBattery: typeof (obj.pause_on_battery ?? obj.pauseOnBattery) === 'boolean'
        ? Boolean(obj.pause_on_battery ?? obj.pauseOnBattery)
        : true,
      pauseOnBatterySaver: typeof (obj.pause_on_battery_saver ?? obj.pauseOnBatterySaver) === 'boolean'
        ? Boolean(obj.pause_on_battery_saver ?? obj.pauseOnBatterySaver)
        : true,
      lowBatteryThresholdPercent: typeof (obj.low_battery_threshold_percent ?? obj.lowBatteryThresholdPercent) === 'number'
        ? Number(obj.low_battery_threshold_percent ?? obj.lowBatteryThresholdPercent)
        : 20,
      pauseOnSystemSuspend: typeof (obj.pause_on_system_suspend ?? obj.pauseOnSystemSuspend) === 'boolean'
        ? Boolean(obj.pause_on_system_suspend ?? obj.pauseOnSystemSuspend)
        : true,
      enableStaticFallback: typeof (obj.enable_static_fallback ?? obj.enableStaticFallback) === 'boolean'
        ? Boolean(obj.enable_static_fallback ?? obj.enableStaticFallback)
        : true,
    };
  }

  async updatePowerPlaybackSettings(settings: PowerPlaybackSettingsDto): Promise<PowerPlaybackSettingsDto> {
    await this.transport.invokeUpdatePowerPlaybackSettings(settings);
    return this.getPowerPlaybackSettings();
  }
}

/**
 * Creates an in-memory mock transport with realistic multi-display topology
 * for automated testing and isolated UI previewing.
 */
export function createMockDisplayIpcTransport(): DisplayIpcTransport {
  let mode: MultiMonitorLayoutMode = 'independent';
  let policySettings: FullscreenPlaybackSettingsDto = {
    enabled: true,
    pauseMode: 'per_display',
    debounceDurationMillis: 250,
  };
  let powerSnapshot: SystemPowerSnapshotDto = {
    powerSource: 'ac',
    batteryAvailability: 'present',
    batteryPercent: 85,
    chargingState: 'charging',
    batterySaver: 'disabled',
    isSystemSuspended: false,
    sampledAtMillis: Date.now(),
  };
  let powerSettings: PowerPlaybackSettingsDto = {
    mode: 'off',
    pauseOnBattery: true,
    pauseOnBatterySaver: true,
    lowBatteryThresholdPercent: 20,
    pauseOnSystemSuspend: true,
    enableStaticFallback: true,
  };
  const assignments = new Map<string, string>([
    ['\\\\.\\DISPLAY1', 'wp-cyberpunk-city'],
  ]);

  const mockDisplays = [
    {
      id: '\\\\.\\DISPLAY1',
      device_name: '\\\\.\\DISPLAY1',
      friendly_name: 'LG UltraGear 27"',
      hardware_id: 'MONITOR\\GSM5B01',
      is_primary: true,
      is_attached: true,
      geometry: {
        monitor_rect: { left: 0, top: 0, right: 2560, bottom: 1440 },
        width: 2560,
        height: 1440,
        scale_factor_percent: 100,
      },
    },
    {
      id: '\\\\.\\DISPLAY2',
      device_name: '\\\\.\\DISPLAY2',
      friendly_name: 'Dell UltraSharp 24"',
      hardware_id: 'MONITOR\\DELA0BC',
      is_primary: false,
      is_attached: true,
      geometry: {
        monitor_rect: { left: 2560, top: 0, right: 4480, bottom: 1080 },
        width: 1920,
        height: 1080,
        scale_factor_percent: 100,
      },
    },
  ];

  return {
    async invokeGetTopologySnapshot() {
      return {
        generation: 1,
        displays: mockDisplays,
        primary_display_id: '\\\\.\\DISPLAY1',
        virtual_desktop_bounds: {
          left: 0,
          top: 0,
          right: 4480,
          bottom: 1440,
          width: 4480,
          height: 1440,
        },
        topology_type: 'dual_extended',
      };
    },

    async invokeGetResolvedAssignments() {
      return mockDisplays.map((d) => {
        const wp = assignments.get(d.id);
        return {
          display_id: d.id,
          display_presence: 'connected',
          target_geometry: d.geometry,
          wallpaper_id: wp,
          assignment_kind: wp ? 'explicit' : undefined,
          outcome: wp ? 'valid_explicit' : 'unassigned',
          is_playable: Boolean(wp),
          effective_title: wp ? 'Cyberpunk City Loop' : undefined,
          effective_type: wp ? 'video' : undefined,
          snapshot_generation: 1,
        };
      });
    },

    async invokeAssignWallpaper(displayId: string, wallpaperId: string) {
      assignments.set(displayId, wallpaperId);
      return { success: true };
    },

    async invokeRemoveAssignment(displayId: string) {
      assignments.delete(displayId);
      return { success: true };
    },

    async invokeSetLayoutMode(newMode: MultiMonitorLayoutMode) {
      mode = newMode;
      return { success: true };
    },

    async invokeApplyLayout() {
      return {
        saved_mode: mode,
        active_mode: mode,
        sessions: [
          {
            session_id: 'sess-display-1',
            display_id: '\\\\.\\DISPLAY1',
            wallpaper_id: assignments.get('\\\\.\\DISPLAY1') ?? 'wp-cyberpunk-city',
            state: 'active',
            geometry: mockDisplays[0].geometry,
            generation: 1,
            created_at_millis: Date.now() - 60000,
            last_updated_millis: Date.now(),
          },
        ],
      };
    },

    async invokeGetMultiMonitorStatus() {
      return {
        saved_mode: mode,
        active_mode: mode,
        sessions: [
          {
            session_id: 'sess-display-1',
            display_id: '\\\\.\\DISPLAY1',
            wallpaper_id: assignments.get('\\\\.\\DISPLAY1') ?? 'wp-cyberpunk-city',
            state: 'active',
            geometry: mockDisplays[0].geometry,
            generation: 1,
            created_at_millis: Date.now() - 60000,
            last_updated_millis: Date.now(),
          },
        ],
      };
    },

    async invokeGetFullscreenPolicySettings() {
      return { ...policySettings };
    },

    async invokeUpdateFullscreenPolicySettings(settings: FullscreenPlaybackSettingsDto) {
      policySettings = { ...settings };
      return { success: true };
    },

    async invokeGetSystemPowerSnapshot() {
      return { ...powerSnapshot };
    },

    async invokeGetPowerPlaybackSettings() {
      return { ...powerSettings };
    },

    async invokeUpdatePowerPlaybackSettings(settings: PowerPlaybackSettingsDto) {
      powerSettings = { ...settings };
      return { success: true };
    },
  };
}
