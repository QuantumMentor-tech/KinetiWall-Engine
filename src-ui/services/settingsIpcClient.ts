/**
 * Authoritative Frontend Settings IPC Client Bridge (Step 61).
 * Interfaces with `kinetiwall-desktop` centralized configuration repository.
 */

import {
  createDefaultConfigDto,
  validateSettings,
  type KinetiWallConfigDto,
} from '../types/settings.ts';

export interface SettingsIpcTransport {
  invokeGetConfig(): Promise<unknown>;
  invokeSaveConfig(config: KinetiWallConfigDto): Promise<unknown>;
  invokeRestoreDefaults(): Promise<unknown>;
}

export class SettingsIpcClient {
  private readonly transport: SettingsIpcTransport;

  constructor(transport: SettingsIpcTransport) {
    this.transport = transport;
  }

  async getConfig(): Promise<KinetiWallConfigDto> {
    try {
      const raw = await this.transport.invokeGetConfig();
      if (!raw || typeof raw !== 'object') {
        throw new Error('Malformed configuration payload received from engine');
      }
      return raw as KinetiWallConfigDto;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      throw new Error(`Failed to load engine configuration: ${message}`);
    }
  }

  async saveConfig(config: KinetiWallConfigDto): Promise<KinetiWallConfigDto> {
    const validationErrors = validateSettings(config);
    const errorKeys = Object.keys(validationErrors);
    if (errorKeys.length > 0) {
      const firstError = validationErrors[errorKeys[0]!];
      throw new Error(`Configuration validation failed: ${firstError}`);
    }

    try {
      const raw = await this.transport.invokeSaveConfig(config);
      if (!raw || typeof raw !== 'object') {
        throw new Error('Engine did not return confirmed configuration after save');
      }
      return raw as KinetiWallConfigDto;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      throw new Error(`Failed to persist engine configuration: ${message}`);
    }
  }

  async restoreDefaults(): Promise<KinetiWallConfigDto> {
    try {
      const raw = await this.transport.invokeRestoreDefaults();
      if (!raw || typeof raw !== 'object') {
        throw new Error('Engine did not return factory configuration after restore');
      }
      return raw as KinetiWallConfigDto;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      throw new Error(`Failed to restore factory defaults: ${message}`);
    }
  }
}

/**
 * Creates an in-memory mock IPC transport for deterministic unit and UI testing.
 */
export function createMockSettingsIpcTransport(
  initial?: Partial<KinetiWallConfigDto>,
): SettingsIpcTransport & {
  setFailOnSave: (fail: boolean) => void;
  setFailOnLoad: (fail: boolean) => void;
  getPersistedConfig: () => KinetiWallConfigDto;
} {
  let storedConfig: KinetiWallConfigDto = {
    ...createDefaultConfigDto(),
    ...initial,
  };
  let shouldFailOnSave = false;
  let shouldFailOnLoad = false;

  return {
    async invokeGetConfig(): Promise<unknown> {
      if (shouldFailOnLoad) {
        throw new Error('Simulated I/O failure loading config.json');
      }
      // Return a deep copy to prevent mutation leakage
      return JSON.parse(JSON.stringify(storedConfig));
    },

    async invokeSaveConfig(config: KinetiWallConfigDto): Promise<unknown> {
      if (shouldFailOnSave) {
        throw new Error('Simulated atomic write failure to config.json.tmp');
      }
      storedConfig = JSON.parse(JSON.stringify(config));
      return storedConfig;
    },

    async invokeRestoreDefaults(): Promise<unknown> {
      if (shouldFailOnSave) {
        throw new Error('Simulated persistence failure during factory restore');
      }
      storedConfig = createDefaultConfigDto();
      return JSON.parse(JSON.stringify(storedConfig));
    },

    setFailOnSave(fail: boolean) {
      shouldFailOnSave = fail;
    },

    setFailOnLoad(fail: boolean) {
      shouldFailOnLoad = fail;
    },

    getPersistedConfig() {
      return JSON.parse(JSON.stringify(storedConfig));
    },
  };
}

