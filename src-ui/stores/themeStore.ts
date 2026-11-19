/**
 * Authoritative Theme State Management Store (Step 62).
 * Manages user preference intent, OS synchronization, and runtime DOM styling.
 */

import {
  BrowserSystemThemeDetector,
  type SystemThemeDetector,
} from '../services/systemThemeDetector.ts';
import {
  resolveEffectiveTheme,
  type EffectiveTheme,
  type StoredThemePreference,
  type ThemeState,
} from '../types/theme.ts';
import type { KinetiWallConfigDto } from '../types/settings.ts';

export type ThemeStoreListener = (state: ThemeState) => void;

export class ThemeStore {
  private state: ThemeState;
  private readonly detector: SystemThemeDetector;
  private readonly listeners: Set<ThemeStoreListener> = new Set();
  private readonly unsubscribeDetector: () => void;

  constructor(
    detector?: SystemThemeDetector,
    initialPreference: StoredThemePreference = 'system',
  ) {
    this.detector = detector ?? new BrowserSystemThemeDetector();
    const systemTheme = this.detector.getCurrentTheme();
    const isSystemSupported = this.detector.isSupported();
    const effectiveTheme = resolveEffectiveTheme(initialPreference, systemTheme);

    this.state = {
      preference: initialPreference,
      effectiveTheme,
      systemTheme,
      isSystemSupported,
    };

    // Subscribe to OS theme changes
    this.unsubscribeDetector = this.detector.subscribe((newSystemTheme) => {
      this.handleSystemThemeChange(newSystemTheme);
    });
  }

  getState(): Readonly<ThemeState> {
    return this.state;
  }

  subscribe(listener: ThemeStoreListener): () => void {
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

  setPreference(preference: StoredThemePreference): void {
    if (this.state.preference === preference) {
      return;
    }

    const effectiveTheme = resolveEffectiveTheme(preference, this.state.systemTheme);
    this.state = {
      ...this.state,
      preference,
      effectiveTheme,
    };

    this.applyToDom();
    this.notify();
  }

  syncWithSettings(config: KinetiWallConfigDto): void {
    if (config.general && config.general.theme) {
      this.setPreference(config.general.theme);
    }
  }

  private handleSystemThemeChange(newSystemTheme: EffectiveTheme): void {
    const isCurrentSystemTheme = this.state.systemTheme === newSystemTheme;
    const newEffectiveTheme = resolveEffectiveTheme(this.state.preference, newSystemTheme);
    const isCurrentEffectiveTheme = this.state.effectiveTheme === newEffectiveTheme;

    if (isCurrentSystemTheme && isCurrentEffectiveTheme) {
      return;
    }

    this.state = {
      ...this.state,
      systemTheme: newSystemTheme,
      effectiveTheme: newEffectiveTheme,
    };

    if (!isCurrentEffectiveTheme) {
      this.applyToDom();
    }
    this.notify();
  }

  applyToDom(targetElement?: HTMLElement | null): void {
    if (typeof document === 'undefined') {
      return;
    }

    const target = targetElement ?? document.documentElement;
    if (target) {
      target.setAttribute('data-theme', this.state.effectiveTheme);
      if ('style' in target && typeof target.style.setProperty === 'function') {
        target.style.setProperty('color-scheme', this.state.effectiveTheme);
      }
    }
  }

  dispose(): void {
    this.unsubscribeDetector();
    this.listeners.clear();
  }
}

