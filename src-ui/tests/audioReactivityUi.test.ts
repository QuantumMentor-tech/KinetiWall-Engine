/**
 * Step 34: Audio-Reactive Wallpapers & WASAPI Loopback Integration UI Tests.
 *
 * Verifies:
 * 1. AudioStore opt-in defaults (disabled by default, bounded parameters).
 * 2. Status descriptions and truthful state reporting.
 * 3. WallpaperDetailsPanel Audio Reactivity section rendering for web wallpapers and omission for video wallpapers.
 * 4. User interaction: toggling audio reactivity, disclosure display, and honest status badge updates.
 */

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

import { WallpaperDetailsPanel } from '../components/domain/WallpaperDetailsPanel.ts';
import {
  clickNode,
  findNodeByTestId,
  renderToHtml,
} from '../components/primitives/uiNode.ts';
import { AudioStore } from '../stores/audioStore.ts';
import { describeAudioStatus } from '../types/audio.ts';
import type { CatalogEntrySummary } from '../types/catalog.ts';

function createMockEntry(overrides: Partial<CatalogEntrySummary> = {}): CatalogEntrySummary {
  return {
    id: 'a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d',
    title: 'Neon Cyber City',
    wallpaper_type: 'web',
    source_kind: 'local_loose_file',
    asset_path: 'C:\\Wallpapers\\Neon\\index.html',
    author: 'CyberArtist',
    description: 'An interactive neon city visualizer.',
    tags: ['cyberpunk', 'reactive'],
    availability: { status: 'available' },
    is_builtin: false,
    created_at: '2026-11-13T10:00:00Z',
    updated_at: '2026-11-13T10:00:00Z',
    schema_version: 1,
    ...overrides,
  };
}

describe('Step 34: Audio Reactivity UI & Store Contracts', () => {
  it('§5.1 & §5.2: AudioStore defaults to disabled, validates and clamps sensitivity and smoothing', () => {
    const store = new AudioStore();
    const state = store.getState();

    // 1. Must be disabled by default (opt-in requirement)
    assert.strictEqual(state.settings.enabled, false);
    assert.strictEqual(state.status, 'disabled');
    assert.strictEqual(state.latestFrame, null);

    // 2. Sensitivity clamping (0.1 to 5.0)
    store.setSensitivity(99.0);
    assert.strictEqual(store.getState().settings.sensitivity, 5.0);
    store.setSensitivity(-10.0);
    assert.strictEqual(store.getState().settings.sensitivity, 0.1);

    // 3. Smoothing clamping (0.0 to 0.95)
    store.setSmoothing(1.5);
    assert.strictEqual(store.getState().settings.smoothing, 0.95);
    store.setSmoothing(-0.5);
    assert.strictEqual(store.getState().settings.smoothing, 0.0);

    // 4. Enabling updates status to active
    store.setEnabled(true);
    assert.strictEqual(store.getState().settings.enabled, true);
    assert.strictEqual(store.getState().status, 'active');

    // 5. Receiving visualizer frame
    store.receiveFrame({
      sequence: 1,
      timestamp_ms: 1000,
      rms: 0.5,
      peak: 0.8,
      frequency_bands: [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8],
      is_silent: false,
    });
    assert.notStrictEqual(store.getState().latestFrame, null);
    assert.strictEqual(store.getState().latestFrame?.rms, 0.5);

    // 6. Disabling clears active frame and sets status back to disabled
    store.setEnabled(false);
    assert.strictEqual(store.getState().settings.enabled, false);
    assert.strictEqual(store.getState().status, 'disabled');
    assert.strictEqual(store.getState().latestFrame, null);
  });

  it('§5.3: describeAudioStatus provides truthful labels and tones for all states', () => {
    assert.deepStrictEqual(describeAudioStatus('disabled'), {
      label: 'Audio reactivity disabled',
      variant: 'neutral',
    });
    assert.deepStrictEqual(describeAudioStatus('initializing'), {
      label: 'Waiting for audio endpoint...',
      variant: 'warning',
    });
    assert.deepStrictEqual(describeAudioStatus('active'), {
      label: 'Audio capture active',
      variant: 'success',
    });
    assert.deepStrictEqual(describeAudioStatus('suspended'), {
      label: 'Audio capture suspended',
      variant: 'warning',
    });
    assert.deepStrictEqual(describeAudioStatus('unavailable', 'No audio device detected'), {
      label: 'No audio device detected',
      variant: 'warning',
    });
    assert.deepStrictEqual(describeAudioStatus('error', 'Loopback stream failed'), {
      label: 'Loopback stream failed',
      variant: 'danger',
    });
  });

  it('§5.1 & §8.3: WallpaperDetailsPanel displays audio section for web wallpapers and omits it for video', () => {
    const webEntry = createMockEntry({ wallpaper_type: 'web' });
    const videoEntry = createMockEntry({ wallpaper_type: 'video' });

    let toggledTo: boolean | null = null;
    const webPanel = WallpaperDetailsPanel({
      entry: webEntry,
      thumbnailFailed: false,
      isHiddenByActiveQuery: false,
      audioSettings: { enabled: false, sensitivity: 1.0, smoothing: 0.5, target_fps: 30 },
      audioStatus: 'disabled',
      onClose: () => { },
      onThumbnailError: () => { },
      onToggleAudioReactivity: (val) => {
        toggledTo = val;
      },
    });

    const webHtml = renderToHtml(webPanel);
    assert.match(webHtml, /data-testid="inspector-audio-section"/);
    assert.match(webHtml, /Audio reactivity disabled/);
    assert.match(webHtml, /Enable Audio Reactivity/);
    assert.match(webHtml, /never recorded or uploaded/);

    // Test toggle click
    const toggleBtn = findNodeByTestId(webPanel, 'inspector-audio-toggle-btn');
    assert.ok(toggleBtn);
    clickNode(toggleBtn);
    assert.strictEqual(toggledTo, true);

    // When entry is video, audio section is omitted to preserve ordinary video behavior
    const videoPanel = WallpaperDetailsPanel({
      entry: videoEntry,
      thumbnailFailed: false,
      isHiddenByActiveQuery: false,
      onClose: () => { },
      onThumbnailError: () => { },
    });
    const videoHtml = renderToHtml(videoPanel);
    assert.doesNotMatch(videoHtml, /data-testid="inspector-audio-section"/);
  });

  it('§5.3: WallpaperDetailsPanel reflects active audio capture state and toggle to disable', () => {
    const webEntry = createMockEntry({ wallpaper_type: 'web' });

    let toggledTo: boolean | null = null;
    const panel = WallpaperDetailsPanel({
      entry: webEntry,
      thumbnailFailed: false,
      isHiddenByActiveQuery: false,
      audioSettings: { enabled: true, sensitivity: 1.0, smoothing: 0.5, target_fps: 30 },
      audioStatus: 'active',
      onClose: () => { },
      onThumbnailError: () => { },
      onToggleAudioReactivity: (val) => {
        toggledTo = val;
      },
    });

    const html = renderToHtml(panel);
    assert.match(html, /Audio capture active/);
    assert.match(html, /Disable Audio Reactivity/);

    const toggleBtn = findNodeByTestId(panel, 'inspector-audio-toggle-btn');
    assert.ok(toggleBtn);
    clickNode(toggleBtn);
    assert.strictEqual(toggledTo, false);
  });
});
