/**
//! Strongly-typed Audio Reactivity contracts for the UI layer (Step 34).
 */

export type AudioCaptureStatus =
  | 'disabled'
  | 'initializing'
  | 'active'
  | 'suspended'
  | 'unavailable'
  | 'error';

export interface AudioReactivitySettings {
  enabled: boolean;
  sensitivity: number;
  smoothing: number;
  target_fps: number;
}

export interface VisualizerFrame {
  sequence: number;
  timestamp_ms: number;
  rms: number;
  peak: number;
  frequency_bands: number[];
  is_silent: boolean;
}

export const DEFAULT_AUDIO_SETTINGS: Readonly<AudioReactivitySettings> = {
  enabled: false,
  sensitivity: 1.0,
  smoothing: 0.5,
  target_fps: 30,
} as const;

export function describeAudioStatus(
  status: AudioCaptureStatus,
  errorDetail?: string | null,
): { label: string; variant: 'neutral' | 'success' | 'warning' | 'danger' } {
  switch (status) {
    case 'disabled':
      return { label: 'Audio reactivity disabled', variant: 'neutral' };
    case 'initializing':
      return { label: 'Waiting for audio endpoint...', variant: 'warning' };
    case 'active':
      return { label: 'Audio capture active', variant: 'success' };
    case 'suspended':
      return { label: 'Audio capture suspended', variant: 'warning' };
    case 'unavailable':
      return {
        label: errorDetail || 'Audio endpoint unavailable',
        variant: 'warning',
      };
    case 'error':
      return {
        label: errorDetail || 'Audio capture error',
        variant: 'danger',
      };
  }
}

