/**
 * Operating System Theme Detection & Media Query Synchronization (Step 62).
 * Provides event-driven, leak-free subscriptions to Windows/OS theme changes.
 */

import type { EffectiveTheme } from '../types/theme.ts';

export interface SystemThemeDetector {
  /** Returns the current OS theme observation. */
  getCurrentTheme(): EffectiveTheme;
  /** Whether the current runtime environment supports OS theme detection. */
  isSupported(): boolean;
  /** Registers a listener for OS appearance changes. Returns a cleanup disposal function. */
  subscribe(listener: (theme: EffectiveTheme) => void): () => void;
}

/**
 * Standard browser / WebView2 implementation using `window.matchMedia('(prefers-color-scheme: dark)')`.
 */
export class BrowserSystemThemeDetector implements SystemThemeDetector {
  private mediaQueryList: MediaQueryList | null = null;

  constructor() {
    if (typeof window !== 'undefined' && typeof window.matchMedia === 'function') {
      try {
        this.mediaQueryList = window.matchMedia('(prefers-color-scheme: dark)');
      } catch {
        this.mediaQueryList = null;
      }
    }
  }

  isSupported(): boolean {
    return this.mediaQueryList !== null;
  }

  getCurrentTheme(): EffectiveTheme {
    if (!this.mediaQueryList) {
      return 'dark'; // Safe default
    }
    return this.mediaQueryList.matches ? 'dark' : 'light';
  }

  subscribe(listener: (theme: EffectiveTheme) => void): () => void {
    if (!this.mediaQueryList) {
      return () => {};
    }

    const handler = (e: MediaQueryListEvent | MediaQueryList) => {
      const isDark = 'matches' in e ? e.matches : false;
      listener(isDark ? 'dark' : 'light');
    };

    const mql = this.mediaQueryList;
    if (typeof mql.addEventListener === 'function') {
      mql.addEventListener('change', handler);
      return () => {
        mql.removeEventListener('change', handler);
      };
    } else if (typeof (mql as unknown as { addListener?: (cb: typeof handler) => void }).addListener === 'function') {
      // Fallback for older WebView2 runtimes
      (mql as unknown as { addListener: (cb: typeof handler) => void }).addListener(handler);
      return () => {
        (mql as unknown as { removeListener: (cb: typeof handler) => void }).removeListener(handler);
      };
    }

    return () => {};
  }
}

/**
 * Mock system theme detector for headless unit tests and controlled test scenarios.
 */
export class MockSystemThemeDetector implements SystemThemeDetector {
  private currentTheme: EffectiveTheme;
  private readonly listeners: Set<(theme: EffectiveTheme) => void> = new Set();
  private supported: boolean;

  constructor(initialTheme: EffectiveTheme = 'dark', isSupported: boolean = true) {
    this.currentTheme = initialTheme;
    this.supported = isSupported;
  }

  isSupported(): boolean {
    return this.supported;
  }

  getCurrentTheme(): EffectiveTheme {
    return this.currentTheme;
  }

  setSupported(supported: boolean): void {
    this.supported = supported;
  }

  setSystemTheme(newTheme: EffectiveTheme): void {
    if (this.currentTheme === newTheme) {
      return;
    }
    this.currentTheme = newTheme;
    for (const listener of this.listeners) {
      listener(newTheme);
    }
  }

  subscribe(listener: (theme: EffectiveTheme) => void): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }
}

