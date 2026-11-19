/**
 * Authoritative Accessibility State Management Store (Step 63).
 * Manages reduced-motion preference intent, OS synchronization, and runtime DOM attributes.
 */

import {
  BrowserSystemMotionDetector,
  type SystemMotionDetector,
} from '../services/systemMotionDetector.ts';
import {
  resolveEffectiveReducedMotion,
  type AccessibilityState,
} from '../types/accessibility.ts';
import type { KinetiWallConfigDto, ReducedMotionPreferenceDto } from '../types/settings.ts';

export type AccessibilityStoreListener = (state: AccessibilityState) => void;

export class AccessibilityStore {
  private state: AccessibilityState;
  private readonly detector: SystemMotionDetector;
  private readonly listeners: Set<AccessibilityStoreListener> = new Set();
  private readonly unsubscribeDetector: () => void;

  constructor(
    detector?: SystemMotionDetector,
    initialPreference: ReducedMotionPreferenceDto = 'system',
  ) {
    this.detector = detector ?? new BrowserSystemMotionDetector();
    const isSystemReducedMotion = this.detector.isReducedMotion();
    const isSystemSupported = this.detector.isSupported();
    const effectiveReducedMotion = resolveEffectiveReducedMotion(
      initialPreference,
      isSystemReducedMotion,
    );

    this.state = {
      reducedMotionPreference: initialPreference,
      isSystemReducedMotion,
      effectiveReducedMotion,
      isSystemSupported,
    };

    // Subscribe to OS reduced-motion changes
    this.unsubscribeDetector = this.detector.subscribe((newSystemReducedMotion) => {
      this.handleSystemMotionChange(newSystemReducedMotion);
    });
  }

  getState(): Readonly<AccessibilityState> {
    return this.state;
  }

  subscribe(listener: AccessibilityStoreListener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  private notify(): void {
    for (const listener of this.listeners) {
      try {
        listener(this.state);
      } catch (err) {
        console.error('[AccessibilityStore] Error notifying listener:', err);
      }
    }
  }

  setPreference(preference: ReducedMotionPreferenceDto): void {
    if (this.state.reducedMotionPreference === preference) {
      return;
    }

    const effectiveReducedMotion = resolveEffectiveReducedMotion(
      preference,
      this.state.isSystemReducedMotion,
    );
    this.state = {
      ...this.state,
      reducedMotionPreference: preference,
      effectiveReducedMotion,
    };

    this.applyToDom();
    this.notify();
  }

  syncWithSettings(config: KinetiWallConfigDto): void {
    if (config.general && config.general.reduced_motion) {
      this.setPreference(config.general.reduced_motion);
    }
  }

  private handleSystemMotionChange(newSystemReducedMotion: boolean): void {
    const isCurrentSystemMotion = this.state.isSystemReducedMotion === newSystemReducedMotion;
    const newEffectiveReducedMotion = resolveEffectiveReducedMotion(
      this.state.reducedMotionPreference,
      newSystemReducedMotion,
    );
    const isCurrentEffectiveReducedMotion =
      this.state.effectiveReducedMotion === newEffectiveReducedMotion;

    if (isCurrentSystemMotion && isCurrentEffectiveReducedMotion) {
      return;
    }

    this.state = {
      ...this.state,
      isSystemReducedMotion: newSystemReducedMotion,
      effectiveReducedMotion: newEffectiveReducedMotion,
    };

    if (!isCurrentEffectiveReducedMotion) {
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
      target.setAttribute(
        'data-reduced-motion',
        this.state.effectiveReducedMotion ? 'true' : 'false',
      );
    }
  }

  dispose(): void {
    this.unsubscribeDetector();
    this.listeners.clear();
  }
}

