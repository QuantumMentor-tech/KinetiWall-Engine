/**
 * Audio Reactivity Store (`Step 34`).
 * Manages user opt-in preferences, live capture status, and visualization frames.
 */

import {
  DEFAULT_AUDIO_SETTINGS,
  type AudioCaptureStatus,
  type AudioReactivitySettings,
  type VisualizerFrame,
} from '../types/audio.ts';

export interface AudioStoreState {
  settings: AudioReactivitySettings;
  status: AudioCaptureStatus;
  errorDetail: string | null;
  latestFrame: VisualizerFrame | null;
}

export type AudioSubscriber = (state: AudioStoreState) => void;

export class AudioStore {
  private state: AudioStoreState;
  private subscribers: Set<AudioSubscriber> = new Set();

  constructor(initialSettings?: Partial<AudioReactivitySettings>) {
    this.state = {
      settings: { ...DEFAULT_AUDIO_SETTINGS, ...initialSettings },
      status: initialSettings?.enabled ? 'initializing' : 'disabled',
      errorDetail: null,
      latestFrame: null,
    };
  }

  public getState(): AudioStoreState {
    return this.state;
  }

  public subscribe(fn: AudioSubscriber): () => void {
    this.subscribers.add(fn);
    return () => this.subscribers.delete(fn);
  }

  private notify(): void {
    for (const sub of this.subscribers) {
      sub(this.state);
    }
  }

  public setEnabled(enabled: boolean): void {
    if (this.state.settings.enabled === enabled) return;
    this.state = {
      ...this.state,
      settings: { ...this.state.settings, enabled },
      status: enabled ? 'active' : 'disabled',
      errorDetail: null,
      latestFrame: enabled ? this.state.latestFrame : null,
    };
    this.notify();
  }

  public setSensitivity(sensitivity: number): void {
    const clamped = Math.max(0.1, Math.min(5.0, Number.isFinite(sensitivity) ? sensitivity : 1.0));
    this.state = {
      ...this.state,
      settings: { ...this.state.settings, sensitivity: clamped },
    };
    this.notify();
  }

  public setSmoothing(smoothing: number): void {
    const clamped = Math.max(0.0, Math.min(0.95, Number.isFinite(smoothing) ? smoothing : 0.5));
    this.state = {
      ...this.state,
      settings: { ...this.state.settings, smoothing: clamped },
    };
    this.notify();
  }

  public setStatus(status: AudioCaptureStatus, errorDetail?: string | null): void {
    this.state = {
      ...this.state,
      status,
      errorDetail: errorDetail ?? null,
    };
    this.notify();
  }

  public receiveFrame(frame: VisualizerFrame): void {
    if (!this.state.settings.enabled) return;
    this.state = {
      ...this.state,
      latestFrame: frame,
    };
    this.notify();
  }
}

