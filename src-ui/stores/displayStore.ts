/**
 * Multi-Monitor Display Store (Step 46).
 * Manages reactive display topology state, assignment changes, layout mode selection,
 * and live runtime session tracking with sequence-guarded updates and safe rollback on error.
 */

import type { DisplayIpcClient } from '../services/displayIpcClient.ts';
import type {
  AdaptiveQualitySettingsDto,
  AggregateBudgetStateDto,
  AggregateWorkloadSnapshotDto,
  ApplicationResourceBudgetDto,
  DisplayId,
  DisplayPowerPlaybackSettingsDto,
  DisplayPowerSnapshotDto,
  DisplaySessionStatusDto,
  DisplayTopologySnapshotDto,
  EffectivePlaybackDecisionDto,
  FullscreenPlaybackSettingsDto,
  InactivityPlaybackSettingsDto,
  InactivitySnapshotDto,
  MultiMonitorLayoutMode,
  MultiMonitorStatusDto,
  PowerPlaybackSettingsDto,
  PowerPolicyMode,
  ResolvedDisplayAssignmentDto,
  SessionPlaybackSettingsDto,
  SystemPowerSnapshotDto,
  WindowsSessionSnapshotDto,
} from '../types/display.ts';

export interface DisplayStoreState {
  snapshot: DisplayTopologySnapshotDto | null;
  resolvedAssignments: ResolvedDisplayAssignmentDto[];
  sessions: DisplaySessionStatusDto[];
  savedMode: MultiMonitorLayoutMode;
  activeMode: MultiMonitorLayoutMode | null;
  selectedDisplayId: DisplayId | null;
  fullscreenPolicy: FullscreenPlaybackSettingsDto;
  powerSnapshot: SystemPowerSnapshotDto;
  powerSettings: PowerPlaybackSettingsDto;
  adaptiveSettings: AdaptiveQualitySettingsDto;
  resourceBudget: ApplicationResourceBudgetDto;
  budgetState: AggregateBudgetStateDto;
  workloadSnapshot: AggregateWorkloadSnapshotDto | null;
  sessionSnapshot: WindowsSessionSnapshotDto;
  sessionSettings: SessionPlaybackSettingsDto;
  displayPowerSnapshot: DisplayPowerSnapshotDto;
  displayPowerSettings: DisplayPowerPlaybackSettingsDto;
  inactivitySnapshot: InactivitySnapshotDto;
  inactivitySettings: InactivityPlaybackSettingsDto;
  policyDecisions: Record<DisplayId, EffectivePlaybackDecisionDto>;
  isLoading: boolean;
  isSaving: boolean;
  isApplying: boolean;
  error: string | null;
  lastUpdatedMillis: number;
  isPickerOpen: boolean;
  pickerTargetDisplayId: DisplayId | null;
}

export type DisplayStoreListener = (state: DisplayStoreState) => void;

export class DisplayStore {
  private readonly client: DisplayIpcClient;
  private state: DisplayStoreState;
  private listeners: Set<DisplayStoreListener> = new Set();
  private nextRequestId = 0;

  constructor(client: DisplayIpcClient, initialState?: Partial<DisplayStoreState>) {
    this.client = client;
    this.state = {
      snapshot: null,
      resolvedAssignments: [],
      sessions: [],
      savedMode: 'independent',
      activeMode: null,
      selectedDisplayId: null,
      fullscreenPolicy: {
        enabled: true,
        pauseMode: 'per_display',
        debounceDurationMillis: 250,
      },
      powerSnapshot: {
        powerSource: 'ac',
        batteryAvailability: 'no_battery',
        batteryPercent: undefined,
        chargingState: 'no_battery',
        batterySaver: 'disabled',
        isSystemSuspended: false,
        sampledAtMillis: Date.now(),
      },
      powerSettings: {
        mode: 'off',
        pauseOnBattery: true,
        pauseOnBatterySaver: true,
        lowBatteryThresholdPercent: 20,
        pauseOnSystemSuspend: true,
        enableStaticFallback: true,
      },
      adaptiveSettings: {
        enabled: false,
        targetFpsFloor: 30,
        sustainedPressureThresholdMs: 3000,
        recoveryDwellThresholdMs: 5000,
        cpuPressureThresholdPercent: 75.0,
        cpuRecoveryThresholdPercent: 40.0,
        adjustmentCooldownMs: 4000,
        allowPowerPolicyCoordination: true,
      },
      resourceBudget: {
        enabled: false,
        targetAggregateCpuPercent: 75.0,
        pressureThresholdCpuPercent: 80.0,
        recoveryThresholdCpuPercent: 45.0,
        sustainedPressureThresholdMs: 3000,
        recoveryDwellThresholdMs: 5000,
        adjustmentCooldownMs: 4000,
        maxSimultaneousAdjustments: 1,
        allowPowerCoordination: true,
      },
      budgetState: 'disabled',
      workloadSnapshot: null,
      sessionSnapshot: {
        lockState: 'unknown',
        connectionState: 'unknown',
        isRegistered: false,
        lastEventTimestampMillis: 0,
      },
      sessionSettings: {
        enabled: false,
        pauseOnLock: true,
        optimizeRemoteSession: false,
        enableStaticFallback: false,
      },
      displayPowerSnapshot: {
        state: 'on',
        scope: 'session_display_status',
        sampledAtMillis: Date.now(),
        isStale: false,
      },
      displayPowerSettings: {
        enabled: false,
        pauseOnDisplayOff: true,
        resumeOnDisplayOn: true,
        dimmedAction: 'do_nothing',
        enableStaticFallback: true,
      },
      inactivitySnapshot: {
        state: 'active',
        elapsedInactivityMillis: 0,
        idleThresholdMillis: 600000,
        sampledAtMillis: Date.now(),
        isFresh: true,
      },
      inactivitySettings: {
        enabled: false,
        idleThresholdMillis: 600000,
        resumeOnActivity: true,
        enableStaticFallback: true,
      },
      policyDecisions: {},
      isLoading: false,
      isSaving: false,
      isApplying: false,
      error: null,
      lastUpdatedMillis: Date.now(),
      isPickerOpen: false,
      pickerTargetDisplayId: null,
      ...initialState,
    };
  }

  public getState(): DisplayStoreState {
    return this.state;
  }

  public subscribe(fn: DisplayStoreListener): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private notify(): void {
    for (const listener of this.listeners) {
      listener(this.state);
    }
  }

  public selectDisplay(displayId: DisplayId | null): void {
    if (this.state.selectedDisplayId === displayId) return;
    this.state = {
      ...this.state,
      selectedDisplayId: displayId,
    };
    this.notify();
  }

  public openWallpaperPicker(displayId: DisplayId): void {
    this.state = {
      ...this.state,
      isPickerOpen: true,
      pickerTargetDisplayId: displayId,
    };
    this.notify();
  }

  public closeWallpaperPicker(): void {
    if (!this.state.isPickerOpen && this.state.pickerTargetDisplayId === null) return;
    this.state = {
      ...this.state,
      isPickerOpen: false,
      pickerTargetDisplayId: null,
    };
    this.notify();
  }

  public async refreshTopology(): Promise<void> {
    const requestId = ++this.nextRequestId;
    this.state = {
      ...this.state,
      isLoading: true,
      error: null,
    };
    this.notify();

    try {
      const [
        snapshot,
        resolvedAssignments,
        multiStatus,
        fullscreenPolicy,
        powerSnapshot,
        powerSettings,
        adaptiveSettings,
        resourceBudget,
        budgetState,
        workloadSnapshot,
        sessionSnapshot,
        sessionSettings,
        displayPowerSnapshot,
        displayPowerSettings,
        inactivitySnapshot,
        inactivitySettings,
      ] = await Promise.all([
        this.client.getTopologySnapshot(),
        this.client.getResolvedAssignments(),
        this.client.getMultiMonitorStatus(),
        this.client.getFullscreenPolicySettings(),
        this.client.getSystemPowerSnapshot(),
        this.client.getPowerPlaybackSettings(),
        this.client.getAdaptiveQualitySettings().catch(() => this.state.adaptiveSettings),
        this.client.getResourceBudgetSettings().catch(() => this.state.resourceBudget),
        this.client.getAggregateBudgetState().catch(() => this.state.budgetState),
        this.client.getAggregateWorkloadSnapshot().catch(() => this.state.workloadSnapshot),
        this.client.getSessionSnapshot().catch(() => this.state.sessionSnapshot),
        this.client.getSessionPlaybackSettings().catch(() => this.state.sessionSettings),
        this.client.getDisplayPowerSnapshot().catch(() => this.state.displayPowerSnapshot),
        this.client.getDisplayPowerPlaybackSettings().catch(() => this.state.displayPowerSettings),
        this.client.getInactivitySnapshot().catch(() => this.state.inactivitySnapshot),
        this.client.getInactivityPlaybackSettings().catch(() => this.state.inactivitySettings),
      ]);

      if (requestId !== this.nextRequestId) {
        return; // Ignore stale request
      }

      // Default selection to primary display or first available display if none selected
      let selectedDisplayId = this.state.selectedDisplayId;
      if (!selectedDisplayId || !snapshot.displays.some((d) => d.id === selectedDisplayId)) {
        selectedDisplayId = snapshot.primaryDisplayId ?? snapshot.displays[0]?.id ?? null;
      }

      this.state = {
        ...this.state,
        snapshot,
        resolvedAssignments,
        sessions: multiStatus.sessions,
        savedMode: multiStatus.savedMode,
        activeMode: multiStatus.activeMode ?? null,
        selectedDisplayId,
        fullscreenPolicy,
        powerSnapshot,
        powerSettings,
        adaptiveSettings,
        resourceBudget,
        budgetState,
        workloadSnapshot,
        sessionSnapshot,
        sessionSettings,
        displayPowerSnapshot,
        displayPowerSettings,
        inactivitySnapshot,
        inactivitySettings,
        isLoading: false,
        error: null,
        lastUpdatedMillis: Date.now(),
      };
      this.notify();
    } catch (err) {
      if (requestId !== this.nextRequestId) return;
      const message = err instanceof Error ? err.message : String(err);
      this.state = {
        ...this.state,
        isLoading: false,
        error: `Failed to load display configuration: ${message}`,
      };
      this.notify();
    }
  }

  public async assignWallpaper(displayId: DisplayId, wallpaperId: string): Promise<boolean> {
    this.state = {
      ...this.state,
      isSaving: true,
      error: null,
    };
    this.notify();

    try {
      await this.client.assignWallpaper(displayId, wallpaperId);
      const resolvedAssignments = await this.client.getResolvedAssignments();
      this.state = {
        ...this.state,
        resolvedAssignments,
        isSaving: false,
        isPickerOpen: false,
        pickerTargetDisplayId: null,
        error: null,
        lastUpdatedMillis: Date.now(),
      };
      this.notify();
      return true;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      this.state = {
        ...this.state,
        isSaving: false,
        error: `Failed to save display assignment: ${message}`,
      };
      this.notify();
      return false;
    }
  }

  public async removeAssignment(displayId: DisplayId): Promise<boolean> {
    this.state = {
      ...this.state,
      isSaving: true,
      error: null,
    };
    this.notify();

    try {
      await this.client.removeAssignment(displayId);
      const resolvedAssignments = await this.client.getResolvedAssignments();
      this.state = {
        ...this.state,
        resolvedAssignments,
        isSaving: false,
        error: null,
        lastUpdatedMillis: Date.now(),
      };
      this.notify();
      return true;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      this.state = {
        ...this.state,
        isSaving: false,
        error: `Failed to remove display assignment: ${message}`,
      };
      this.notify();
      return false;
    }
  }

  public async setLayoutMode(mode: MultiMonitorLayoutMode): Promise<boolean> {
    if (this.state.savedMode === mode) return true;

    const previousMode = this.state.savedMode;
    this.state = {
      ...this.state,
      savedMode: mode,
      isSaving: true,
      error: null,
    };
    this.notify();

    try {
      await this.client.setLayoutMode(mode);
      const multiStatus = await this.client.getMultiMonitorStatus();
      this.state = {
        ...this.state,
        savedMode: multiStatus.savedMode,
        activeMode: multiStatus.activeMode ?? null,
        isSaving: false,
        error: null,
        lastUpdatedMillis: Date.now(),
      };
      this.notify();
      return true;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      // Safe rollback to previous mode
      this.state = {
        ...this.state,
        savedMode: previousMode,
        isSaving: false,
        error: `Failed to set layout mode: ${message}`,
      };
      this.notify();
      return false;
    }
  }

  public async applyLayout(): Promise<boolean> {
    this.state = {
      ...this.state,
      isApplying: true,
      error: null,
    };
    this.notify();

    try {
      await this.client.applyLayout();
      const multiStatus = await this.client.getMultiMonitorStatus();
      this.state = {
        ...this.state,
        sessions: multiStatus.sessions,
        savedMode: multiStatus.savedMode,
        activeMode: multiStatus.activeMode ?? null,
        isApplying: false,
        error: null,
        lastUpdatedMillis: Date.now(),
      };
      this.notify();
      return true;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      this.state = {
        ...this.state,
        isApplying: false,
        error: `Failed to apply layout: ${message}`,
      };
      this.notify();
      return false;
    }
  }

  public async updateFullscreenPolicy(
    patch: Partial<FullscreenPlaybackSettingsDto>,
  ): Promise<boolean> {
    const updated: FullscreenPlaybackSettingsDto = {
      ...this.state.fullscreenPolicy,
      ...patch,
    };
    this.state = {
      ...this.state,
      isSaving: true,
      error: null,
    };
    this.notify();

    try {
      const result = await this.client.updateFullscreenPolicySettings(updated);
      this.state = {
        ...this.state,
        fullscreenPolicy: result,
        isSaving: false,
        error: null,
        lastUpdatedMillis: Date.now(),
      };
      this.notify();
      return true;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      this.state = {
        ...this.state,
        isSaving: false,
        error: `Failed to update smart playback policy: ${message}`,
      };
      this.notify();
      return false;
    }
  }

  public async toggleSmartPauseFullscreen(): Promise<boolean> {
    return this.updateFullscreenPolicy({
      enabled: !this.state.fullscreenPolicy.enabled,
    });
  }

  public async updatePowerSettings(
    patch: Partial<PowerPlaybackSettingsDto>,
  ): Promise<boolean> {
    const updated: PowerPlaybackSettingsDto = {
      ...this.state.powerSettings,
      ...patch,
    };
    this.state = {
      ...this.state,
      isSaving: true,
      error: null,
    };
    this.notify();

    try {
      const result = await this.client.updatePowerPlaybackSettings(updated);
      this.state = {
        ...this.state,
        powerSettings: result,
        isSaving: false,
        error: null,
        lastUpdatedMillis: Date.now(),
      };
      this.notify();
      return true;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      this.state = {
        ...this.state,
        isSaving: false,
        error: `Failed to update power playback policy: ${message}`,
      };
      this.notify();
      return false;
    }
  }

  public async setPowerPolicyMode(mode: PowerPolicyMode): Promise<boolean> {
    return this.updatePowerSettings({ mode });
  }

  public async toggleStaticFallback(): Promise<boolean> {
    return this.updatePowerSettings({
      enableStaticFallback: !this.state.powerSettings.enableStaticFallback,
    });
  }

  public async updateAdaptiveQualitySettings(
    patch: Partial<AdaptiveQualitySettingsDto>,
  ): Promise<boolean> {
    const updated: AdaptiveQualitySettingsDto = {
      ...this.state.adaptiveSettings,
      ...patch,
    };
    this.state = {
      ...this.state,
      isSaving: true,
      error: null,
    };
    this.notify();

    try {
      const result = await this.client.updateAdaptiveQualitySettings(updated);
      this.state = {
        ...this.state,
        adaptiveSettings: result,
        isSaving: false,
        error: null,
        lastUpdatedMillis: Date.now(),
      };
      this.notify();
      return true;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      this.state = {
        ...this.state,
        isSaving: false,
        error: `Failed to update adaptive quality settings: ${message}`,
      };
      this.notify();
      return false;
    }
  }

  public async toggleAdaptiveQuality(): Promise<boolean> {
    return this.updateAdaptiveQualitySettings({
      enabled: !this.state.adaptiveSettings.enabled,
    });
  }

  public async updateResourceBudgetSettings(
    patch: Partial<ApplicationResourceBudgetDto>,
  ): Promise<boolean> {
    const updated: ApplicationResourceBudgetDto = {
      ...this.state.resourceBudget,
      ...patch,
    };
    this.state = {
      ...this.state,
      isSaving: true,
      error: null,
    };
    this.notify();

    try {
      const result = await this.client.updateResourceBudgetSettings(updated);
      const budgetState = await this.client.getAggregateBudgetState().catch(() => this.state.budgetState);
      const workloadSnapshot = await this.client.getAggregateWorkloadSnapshot().catch(() => this.state.workloadSnapshot);
      this.state = {
        ...this.state,
        resourceBudget: result,
        budgetState,
        workloadSnapshot,
        isSaving: false,
        error: null,
        lastUpdatedMillis: Date.now(),
      };
      this.notify();
      return true;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      this.state = {
        ...this.state,
        isSaving: false,
        error: `Failed to update resource budget settings: ${message}`,
      };
      this.notify();
      return false;
    }
  }

  public async toggleResourceBudget(): Promise<boolean> {
    return this.updateResourceBudgetSettings({
      enabled: !this.state.resourceBudget.enabled,
    });
  }

  public async updateSessionPlaybackSettings(
    patch: Partial<SessionPlaybackSettingsDto>,
  ): Promise<boolean> {
    const updated: SessionPlaybackSettingsDto = {
      ...this.state.sessionSettings,
      ...patch,
    };
    this.state = {
      ...this.state,
      isSaving: true,
      error: null,
    };
    this.notify();

    try {
      const result = await this.client.updateSessionPlaybackSettings(updated);
      const sessionSnapshot = await this.client.getSessionSnapshot().catch(() => this.state.sessionSnapshot);
      this.state = {
        ...this.state,
        sessionSettings: result,
        sessionSnapshot,
        isSaving: false,
        error: null,
        lastUpdatedMillis: Date.now(),
      };
      this.notify();
      return true;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      this.state = {
        ...this.state,
        isSaving: false,
        error: `Failed to update session playback settings: ${message}`,
      };
      this.notify();
      return false;
    }
  }

  public async toggleLockPause(): Promise<boolean> {
    const nextEnabled = !this.state.sessionSettings.enabled;
    return this.updateSessionPlaybackSettings({
      enabled: nextEnabled,
      pauseOnLock: true,
    });
  }

  public async updateDisplayPowerPlaybackSettings(
    patch: Partial<DisplayPowerPlaybackSettingsDto>,
  ): Promise<boolean> {
    const updated: DisplayPowerPlaybackSettingsDto = {
      ...this.state.displayPowerSettings,
      ...patch,
    };
    this.state = {
      ...this.state,
      isSaving: true,
      error: null,
    };
    this.notify();

    try {
      const result = await this.client.updateDisplayPowerPlaybackSettings(updated);
      const displayPowerSnapshot = await this.client
        .getDisplayPowerSnapshot()
        .catch(() => this.state.displayPowerSnapshot);
      this.state = {
        ...this.state,
        displayPowerSettings: result,
        displayPowerSnapshot,
        isSaving: false,
        error: null,
        lastUpdatedMillis: Date.now(),
      };
      this.notify();
      return true;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      this.state = {
        ...this.state,
        isSaving: false,
        error: `Failed to update display power playback settings: ${message}`,
      };
      this.notify();
      return false;
    }
  }

  public async toggleDisplayPowerPause(): Promise<boolean> {
    const nextEnabled = !this.state.displayPowerSettings.enabled;
    return this.updateDisplayPowerPlaybackSettings({
      enabled: nextEnabled,
      pauseOnDisplayOff: true,
      resumeOnDisplayOn: true,
    });
  }

  public async updateInactivityPlaybackSettings(
    patch: Partial<InactivityPlaybackSettingsDto>,
  ): Promise<boolean> {
    const updated: InactivityPlaybackSettingsDto = {
      ...this.state.inactivitySettings,
      ...patch,
    };
    this.state = {
      ...this.state,
      isSaving: true,
      error: null,
    };
    this.notify();

    try {
      const result = await this.client.updateInactivityPlaybackSettings(updated);
      const inactivitySnapshot = await this.client
        .getInactivitySnapshot()
        .catch(() => this.state.inactivitySnapshot);
      this.state = {
        ...this.state,
        inactivitySettings: result,
        inactivitySnapshot,
        isSaving: false,
        error: null,
        lastUpdatedMillis: Date.now(),
      };
      this.notify();
      return true;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      this.state = {
        ...this.state,
        isSaving: false,
        error: `Failed to update user inactivity playback settings: ${message}`,
      };
      this.notify();
      return false;
    }
  }

  public async toggleInactivityPause(): Promise<boolean> {
    const nextEnabled = !this.state.inactivitySettings.enabled;
    return this.updateInactivityPlaybackSettings({
      enabled: nextEnabled,
      resumeOnActivity: true,
    });
  }

  /**
   * Updates or registers an authoritative policy arbitration decision for a display.
   */
  public updatePolicyDecision(decision: EffectivePlaybackDecisionDto): void {
    this.state = {
      ...this.state,
      policyDecisions: {
        ...this.state.policyDecisions,
        [decision.displayId]: decision,
      },
    };
    this.notify();
  }
}

