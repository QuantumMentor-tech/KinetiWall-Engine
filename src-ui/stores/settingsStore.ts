/**
 * Centralized Settings Store (Step 61).
 * Manages reactive state for persistent application preferences, draft editing,
 * save-state UX, and atomic persistence coordination.
 */

import { SettingsIpcClient } from '../services/settingsIpcClient.ts';
import {
  createDefaultConfigDto,
  getDirtyFields,
  validateSettings,
  type KinetiWallConfigDto,
  type SettingsTab,
} from '../types/settings.ts';

export interface SettingsStoreState {
  persistedConfig: KinetiWallConfigDto | null;
  draftConfig: KinetiWallConfigDto | null;
  isLoading: boolean;
  isSaving: boolean;
  isDirty: boolean;
  dirtyFields: string[];
  validationErrors: Record<string, string>;
  saveStatus: 'idle' | 'saving' | 'saved' | 'error';
  saveError: string | null;
  lastSavedTimestampMillis: number | null;
  activeTab: SettingsTab;
}

export type SettingsStoreListener = (state: SettingsStoreState) => void;

export class SettingsStore {
  private readonly client: SettingsIpcClient;
  private state: SettingsStoreState;
  private readonly listeners: Set<SettingsStoreListener> = new Set();

  constructor(client: SettingsIpcClient, initialState?: Partial<SettingsStoreState>) {
    this.client = client;
    this.state = {
      persistedConfig: null,
      draftConfig: null,
      isLoading: false,
      isSaving: false,
      isDirty: false,
      dirtyFields: [],
      validationErrors: {},
      saveStatus: 'idle',
      saveError: null,
      lastSavedTimestampMillis: null,
      activeTab: 'general',
      ...initialState,
    };
  }

  getState(): Readonly<SettingsStoreState> {
    return this.state;
  }

  subscribe(listener: SettingsStoreListener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  private notify(): void {
    for (const listener of this.listeners) {
      listener(this.state);
    }
  }

  async loadConfig(): Promise<void> {
    this.state = {
      ...this.state,
      isLoading: true,
      saveError: null,
    };
    this.notify();

    try {
      const config = await this.client.getConfig();
      this.state = {
        ...this.state,
        persistedConfig: config,
        draftConfig: JSON.parse(JSON.stringify(config)),
        isLoading: false,
        isDirty: false,
        dirtyFields: [],
        validationErrors: {},
        saveStatus: 'idle',
        saveError: null,
      };
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      this.state = {
        ...this.state,
        isLoading: false,
        saveStatus: 'error',
        saveError: msg,
      };
    }
    this.notify();
  }

  updateDraftField<S extends keyof KinetiWallConfigDto, F extends keyof KinetiWallConfigDto[S]>(
    section: S,
    field: F,
    value: KinetiWallConfigDto[S][F],
  ): void {
    if (!this.state.draftConfig) {
      return;
    }

    const updatedDraft: KinetiWallConfigDto = JSON.parse(JSON.stringify(this.state.draftConfig));
    const sectionObj = updatedDraft[section] as Record<string, unknown>;
    sectionObj[field as string] = value;

    const dirtyFields = getDirtyFields(updatedDraft, this.state.persistedConfig);
    const validationErrors = validateSettings(updatedDraft);

    this.state = {
      ...this.state,
      draftConfig: updatedDraft,
      isDirty: dirtyFields.length > 0,
      dirtyFields,
      validationErrors,
      saveStatus: dirtyFields.length > 0 ? 'idle' : this.state.saveStatus,
      saveError: null,
    };
    this.notify();
  }

  discardChanges(): void {
    if (!this.state.persistedConfig) {
      return;
    }

    this.state = {
      ...this.state,
      draftConfig: JSON.parse(JSON.stringify(this.state.persistedConfig)),
      isDirty: false,
      dirtyFields: [],
      validationErrors: {},
      saveStatus: 'idle',
      saveError: null,
    };
    this.notify();
  }

  async saveChanges(): Promise<boolean> {
    if (!this.state.draftConfig) {
      return false;
    }

    // Run authoritative client validation before triggering persistence
    const validationErrors = validateSettings(this.state.draftConfig);
    if (Object.keys(validationErrors).length > 0) {
      this.state = {
        ...this.state,
        validationErrors,
        saveStatus: 'error',
        saveError: 'Please correct the validation errors before saving preferences.',
      };
      this.notify();
      return false;
    }

    this.state = {
      ...this.state,
      isSaving: true,
      saveStatus: 'saving',
      saveError: null,
    };
    this.notify();

    try {
      const confirmed = await this.client.saveConfig(this.state.draftConfig);
      this.state = {
        ...this.state,
        persistedConfig: confirmed,
        draftConfig: JSON.parse(JSON.stringify(confirmed)),
        isSaving: false,
        isDirty: false,
        dirtyFields: [],
        validationErrors: {},
        saveStatus: 'saved',
        saveError: null,
        lastSavedTimestampMillis: Date.now(),
      };
      this.notify();
      return true;
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      this.state = {
        ...this.state,
        isSaving: false,
        saveStatus: 'error',
        saveError: msg,
      };
      this.notify();
      return false;
    }
  }

  resetSectionToDefaults(section: keyof KinetiWallConfigDto): void {
    if (!this.state.draftConfig) {
      return;
    }

    const defaults = createDefaultConfigDto();
    const updatedDraft: KinetiWallConfigDto = JSON.parse(JSON.stringify(this.state.draftConfig));
    updatedDraft[section] = defaults[section];

    const dirtyFields = getDirtyFields(updatedDraft, this.state.persistedConfig);
    const validationErrors = validateSettings(updatedDraft);

    this.state = {
      ...this.state,
      draftConfig: updatedDraft,
      isDirty: dirtyFields.length > 0,
      dirtyFields,
      validationErrors,
      saveStatus: 'idle',
      saveError: null,
    };
    this.notify();
  }

  async resetAllToDefaults(): Promise<boolean> {
    this.state = {
      ...this.state,
      isSaving: true,
      saveStatus: 'saving',
      saveError: null,
    };
    this.notify();

    try {
      const confirmed = await this.client.restoreDefaults();
      this.state = {
        ...this.state,
        persistedConfig: confirmed,
        draftConfig: JSON.parse(JSON.stringify(confirmed)),
        isSaving: false,
        isDirty: false,
        dirtyFields: [],
        validationErrors: {},
        saveStatus: 'saved',
        saveError: null,
        lastSavedTimestampMillis: Date.now(),
      };
      this.notify();
      return true;
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      this.state = {
        ...this.state,
        isSaving: false,
        saveStatus: 'error',
        saveError: msg,
      };
      this.notify();
      return false;
    }
  }

  setActiveTab(tab: SettingsTab): void {
    if (this.state.activeTab === tab) {
      return;
    }
    this.state = {
      ...this.state,
      activeTab: tab,
    };
    this.notify();
  }
}

