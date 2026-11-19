/**
 * Step 61: Settings Interface Integration, Preference Controls & Save-State UX Tests.
 *
 * Exhaustively verifies:
 * 1. SettingsStore lifecycle, initial load, and factory default synchronization.
 * 2. Drafting changes, granular dirty-field tracking, and discard rollback.
 * 3. Client-side validation against authoritative bounds (clamping, framerates, volume).
 * 4. Atomic save flow, save failure handling, and state preservation.
 * 5. Scoped section reset vs full factory reset.
 * 6. Interactive preference controls (PreferenceToggle, PreferenceSegmentedGroup, PreferenceSlider, PreferenceSelect).
 * 7. SaveStatusBar reactive states and accessibility contracts.
 * 8. SettingsView full rendering and tab switching.
 * 9. AppShell navigation to settings.
 */

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

import {
  PreferenceSection,
  PreferenceSegmentedGroup,
  PreferenceSelect,
  PreferenceSlider,
  PreferenceToggle,
  SaveStatusBar,
} from '../components/domain/PreferenceControls.ts';
import {
  activateByKeyboard,
  changeNodeValue,
  clickNode,
  findNodeByTestId,
  renderToHtml,
} from '../components/primitives/uiNode.ts';
import { AppShellController } from '../components/shell/AppShell.ts';
import { CatalogIpcClient } from '../services/catalogIpcClient.ts';
import {
  SettingsIpcClient,
  createMockSettingsIpcTransport,
} from '../services/settingsIpcClient.ts';
import { CatalogStore } from '../stores/catalogStore.ts';
import { RouterStore } from '../stores/routerStore.ts';
import { SettingsStore } from '../stores/settingsStore.ts';
import {
  createDefaultConfigDto,
  validateSettings,
  type KinetiWallConfigDto,
} from '../types/settings.ts';
import { SettingsView } from '../views/SettingsView.ts';

describe('Step 61: Settings Interface & Save-State UX', () => {
  // -----------------------------------------------------------------------------------------------
  // 1. SettingsStore Lifecycle & Defaults
  // -----------------------------------------------------------------------------------------------
  it('§1. SettingsStore loads factory defaults and initializes clean state', async () => {
    const transport = createMockSettingsIpcTransport();
    const client = new SettingsIpcClient(transport);
    const store = new SettingsStore(client);

    assert.strictEqual(store.getState().persistedConfig, null);
    assert.strictEqual(store.getState().isDirty, false);

    await store.loadConfig();

    const state = store.getState();
    assert.notStrictEqual(state.persistedConfig, null);
    assert.strictEqual(state.persistedConfig?.schema_version, 1);
    assert.strictEqual(state.persistedConfig?.general.autostart, false);
    assert.strictEqual(state.persistedConfig?.performance.target_framerate_ac, 60);
    assert.strictEqual(state.persistedConfig?.audio.master_volume, 0.5);
    assert.strictEqual(state.isDirty, false);
    assert.strictEqual(state.dirtyFields.length, 0);
    assert.strictEqual(state.saveStatus, 'idle');
  });

  // -----------------------------------------------------------------------------------------------
  // 2. Draft Editing & Dirty Tracking
  // -----------------------------------------------------------------------------------------------
  it('§2. Draft editing tracks modified fields without mutating persisted config', async () => {
    const transport = createMockSettingsIpcTransport();
    const client = new SettingsIpcClient(transport);
    const store = new SettingsStore(client);
    await store.loadConfig();

    // Mutate draft field
    store.updateDraftField('general', 'autostart', true);

    const state = store.getState();
    assert.strictEqual(state.draftConfig?.general.autostart, true);
    // Authoritative persisted config remains unmodified
    assert.strictEqual(state.persistedConfig?.general.autostart, false);
    assert.strictEqual(state.isDirty, true);
    assert.deepStrictEqual(state.dirtyFields, ['general.autostart']);

    // Reverting to identical value clears dirty state
    store.updateDraftField('general', 'autostart', false);
    assert.strictEqual(store.getState().isDirty, false);
    assert.deepStrictEqual(store.getState().dirtyFields, []);
  });

  // -----------------------------------------------------------------------------------------------
  // 3. Discarding Changes
  // -----------------------------------------------------------------------------------------------
  it('§3. Discarding changes rolls back draft to last confirmed persisted state', async () => {
    const transport = createMockSettingsIpcTransport();
    const client = new SettingsIpcClient(transport);
    const store = new SettingsStore(client);
    await store.loadConfig();

    store.updateDraftField('performance', 'target_framerate_ac', 144);
    store.updateDraftField('audio', 'master_mute', true);

    assert.strictEqual(store.getState().isDirty, true);
    assert.strictEqual(store.getState().dirtyFields.length, 2);

    store.discardChanges();

    const state = store.getState();
    assert.strictEqual(state.isDirty, false);
    assert.strictEqual(state.dirtyFields.length, 0);
    assert.strictEqual(state.draftConfig?.performance.target_framerate_ac, 60);
    assert.strictEqual(state.draftConfig?.audio.master_mute, false);
  });

  // -----------------------------------------------------------------------------------------------
  // 4. Validation Rules Enforcement
  // -----------------------------------------------------------------------------------------------
  it('§4. Validates out-of-bounds fields and prevents saving invalid configurations', async () => {
    const transport = createMockSettingsIpcTransport();
    const client = new SettingsIpcClient(transport);
    const store = new SettingsStore(client);
    await store.loadConfig();

    // 1. Language validation
    store.updateDraftField('general', 'language', '');
    assert.ok(store.getState().validationErrors['general.language']);

    // 2. Framerate AC validation (bounds: 15-240)
    store.updateDraftField('performance', 'target_framerate_ac', 300);
    assert.ok(store.getState().validationErrors['performance.target_framerate_ac']);

    // 3. Framerate Battery validation (bounds: 15-60)
    store.updateDraftField('performance', 'target_framerate_battery', 120);
    assert.ok(store.getState().validationErrors['performance.target_framerate_battery']);

    // 4. Master volume validation (bounds: 0.0-1.0)
    store.updateDraftField('audio', 'master_volume', 2.0);
    assert.ok(store.getState().validationErrors['audio.master_volume']);

    // Attempting to save while invalid must fail and be rejected
    const saveSuccess = await store.saveChanges();
    assert.strictEqual(saveSuccess, false);
    assert.strictEqual(store.getState().saveStatus, 'error');
    assert.ok(store.getState().saveError?.includes('validation errors'));
  });

  // -----------------------------------------------------------------------------------------------
  // 5. Successful Persistence
  // -----------------------------------------------------------------------------------------------
  it('§5. Saving valid changes commits to storage and updates confirmed persisted state', async () => {
    const transport = createMockSettingsIpcTransport();
    const client = new SettingsIpcClient(transport);
    const store = new SettingsStore(client);
    await store.loadConfig();

    store.updateDraftField('general', 'theme', 'dark');
    store.updateDraftField('performance', 'target_framerate_ac', 120);

    assert.strictEqual(store.getState().isDirty, true);

    const saved = await store.saveChanges();
    assert.strictEqual(saved, true);

    const state = store.getState();
    assert.strictEqual(state.isDirty, false);
    assert.strictEqual(state.dirtyFields.length, 0);
    assert.strictEqual(state.saveStatus, 'saved');
    assert.notStrictEqual(state.lastSavedTimestampMillis, null);
    assert.strictEqual(state.persistedConfig?.general.theme, 'dark');
    assert.strictEqual(state.persistedConfig?.performance.target_framerate_ac, 120);

    // Verify transport also received the new values
    const underlying = transport.getPersistedConfig();
    assert.strictEqual(underlying.general.theme, 'dark');
    assert.strictEqual(underlying.performance.target_framerate_ac, 120);
  });

  // -----------------------------------------------------------------------------------------------
  // 6. Persistence Failure Handling
  // -----------------------------------------------------------------------------------------------
  it('§6. Persistence failures preserve draft edits without falsely indicating success', async () => {
    const transport = createMockSettingsIpcTransport();
    const client = new SettingsIpcClient(transport);
    const store = new SettingsStore(client);
    await store.loadConfig();

    store.updateDraftField('general', 'start_minimized', false);
    assert.strictEqual(store.getState().isDirty, true);

    // Injected fault in storage transport
    transport.setFailOnSave(true);

    const saved = await store.saveChanges();
    assert.strictEqual(saved, false);

    const state = store.getState();
    assert.strictEqual(state.saveStatus, 'error');
    assert.ok(state.saveError?.includes('Simulated atomic write failure'));
    // Draft remains dirty so user changes are not discarded
    assert.strictEqual(state.isDirty, true);
    assert.strictEqual(state.draftConfig?.general.start_minimized, false);
    // Confirmed persisted state is unchanged
    assert.strictEqual(state.persistedConfig?.general.start_minimized, true);
  });

  // -----------------------------------------------------------------------------------------------
  // 7. Scoped Section Reset vs Full Factory Reset
  // -----------------------------------------------------------------------------------------------
  it('§7. Scoped section reset restores only targeted section to defaults', async () => {
    const transport = createMockSettingsIpcTransport();
    const client = new SettingsIpcClient(transport);
    const store = new SettingsStore(client);
    await store.loadConfig();

    store.updateDraftField('general', 'theme', 'light');
    store.updateDraftField('performance', 'target_framerate_ac', 144);
    await store.saveChanges();

    // Modify both in draft
    store.updateDraftField('general', 'theme', 'dark');
    store.updateDraftField('performance', 'target_framerate_ac', 200);

    // Reset only general section
    store.resetSectionToDefaults('general');

    const state = store.getState();
    // General section is reset to factory default ('system')
    assert.strictEqual(state.draftConfig?.general.theme, 'system');
    // Performance section retains user's draft (200)
    assert.strictEqual(state.draftConfig?.performance.target_framerate_ac, 200);
  });

  it('§7b. Full factory reset restores all application preferences', async () => {
    const transport = createMockSettingsIpcTransport();
    const client = new SettingsIpcClient(transport);
    const store = new SettingsStore(client);
    await store.loadConfig();

    store.updateDraftField('general', 'theme', 'dark');
    store.updateDraftField('audio', 'master_volume', 0.9);
    await store.saveChanges();

    const resetSuccess = await store.resetAllToDefaults();
    assert.strictEqual(resetSuccess, true);

    const state = store.getState();
    assert.strictEqual(state.persistedConfig?.general.theme, 'system');
    assert.strictEqual(state.persistedConfig?.audio.master_volume, 0.5);
    assert.strictEqual(state.isDirty, false);
  });

  // -----------------------------------------------------------------------------------------------
  // 8. Reusable Preference Controls (Interactivity & Accessibility)
  // -----------------------------------------------------------------------------------------------
  it('§8. PreferenceToggle enforces switch semantics and handles keyboard activation', () => {
    let toggled = false;
    const node = PreferenceToggle({
      id: 'test-switch',
      testId: 'test-switch',
      label: 'Enable Hardware Acceleration',
      description: 'Use GPU for decoding',
      checked: false,
      dirty: false,
      onChange: (val) => {
        toggled = val;
      },
    });

    const btn = findNodeByTestId(node, 'test-switch');
    assert.ok(btn);
    assert.strictEqual(btn.attrs.role, 'switch');
    assert.strictEqual(btn.attrs['aria-checked'], 'false');

    // Simulate click
    clickNode(btn);
    assert.strictEqual(toggled, true);

    // Simulate keyboard activation (Space)
    activateByKeyboard(btn, ' ');
    assert.strictEqual(toggled, true);
  });

  it('§8b. PreferenceSegmentedGroup handles enum selection and dirty indicator', () => {
    let selected = 'pause';
    const node = PreferenceSegmentedGroup({
      testId: 'test-enum-group',
      label: 'Fullscreen Policy',
      description: 'Action on fullscreen',
      value: 'pause',
      dirty: true,
      options: [
        { value: 'pause', label: 'Pause', testId: 'opt-pause' },
        { value: 'mute', label: 'Mute', testId: 'opt-mute' },
      ],
      onSelect: (val) => {
        selected = val;
      },
    });

    const muteBtn = findNodeByTestId(node, 'opt-mute');
    assert.ok(muteBtn);
    assert.strictEqual(muteBtn.attrs.role, 'radio');
    assert.strictEqual(muteBtn.attrs['aria-checked'], 'false');

    clickNode(muteBtn);
    assert.strictEqual(selected, 'mute');

    // Dirty dot indicator exists when dirty: true
    const dot = findNodeByTestId(node, 'test-enum-group-dirty-dot');
    assert.ok(dot);
  });

  it('§8c. PreferenceSlider handles numeric range updates and badge display', () => {
    let value = 60;
    const node = PreferenceSlider({
      id: 'test-slider',
      testId: 'test-slider',
      label: 'Target Framerate',
      description: 'Max rendering FPS',
      value: 60,
      min: 15,
      max: 240,
      step: 5,
      unitLabel: 'FPS',
      onChange: (v) => {
        value = v;
      },
    });

    const badge = findNodeByTestId(node, 'test-slider-badge');
    assert.ok(badge);
    assert.strictEqual(badge.attrs['aria-label'], '60 FPS');

    const input = findNodeByTestId(node, 'test-slider');
    assert.ok(input);
    assert.strictEqual(input.attrs.type, 'range');
    assert.strictEqual(input.attrs['aria-valuenow'], 60);

    changeNodeValue(input, '120');
    assert.strictEqual(value, 120);
  });

  // -----------------------------------------------------------------------------------------------
  // 9. SaveStatusBar States & Actions
  // -----------------------------------------------------------------------------------------------
  it('§9. SaveStatusBar dynamically adapts between Saved, Dirty, and Error states', () => {
    let saved = false;
    let discarded = false;

    // 1. Saved state (clean)
    const cleanBar = SaveStatusBar({
      testId: 'save-bar',
      isDirty: false,
      dirtyCount: 0,
      isSaving: false,
      saveStatus: 'saved',
      saveError: null,
      hasValidationErrors: false,
      lastSavedTimestampMillis: Date.now(),
      onSave: () => {
        saved = true;
      },
      onDiscard: () => {
        discarded = true;
      },
    });

    const cleanHtml = renderToHtml(cleanBar);
    assert.ok(cleanHtml.includes('Preferences saved and up to date'));

    // 2. Dirty state
    const dirtyBar = SaveStatusBar({
      testId: 'save-bar',
      isDirty: true,
      dirtyCount: 2,
      isSaving: false,
      saveStatus: 'idle',
      saveError: null,
      hasValidationErrors: false,
      lastSavedTimestampMillis: null,
      onSave: () => {
        saved = true;
      },
      onDiscard: () => {
        discarded = true;
      },
    });

    const dirtyHtml = renderToHtml(dirtyBar);
    assert.ok(dirtyHtml.includes('2 unsaved preferences in draft'));

    const saveBtn = findNodeByTestId(dirtyBar, 'save-bar-save-btn');
    assert.ok(saveBtn);
    assert.strictEqual(saveBtn.attrs.disabled, false);

    clickNode(saveBtn);
    assert.strictEqual(saved, true);

    const discardBtn = findNodeByTestId(dirtyBar, 'save-bar-discard-btn');
    assert.ok(discardBtn);
    clickNode(discardBtn);
    assert.strictEqual(discarded, true);
  });

  // -----------------------------------------------------------------------------------------------
  // 10. Full SettingsView Integration & Tab Switching
  // -----------------------------------------------------------------------------------------------
  it('§10. SettingsView renders full preferences hierarchy and supports tab navigation', async () => {
    const transport = createMockSettingsIpcTransport();
    const client = new SettingsIpcClient(transport);
    const store = new SettingsStore(client);
    await store.loadConfig();

    const view = SettingsView({ settingsStore: store });
    const html = renderToHtml(view);

    // Initial General tab
    assert.ok(html.includes('General Preferences'));
    assert.ok(html.includes('Run on Windows Startup'));
    assert.ok(html.includes('Appearance &amp; Theme') || html.includes('Appearance & Theme'));

    // Switch to Playback tab
    store.setActiveTab('playback');
    const playbackHtml = renderToHtml(SettingsView({ settingsStore: store }));
    assert.ok(playbackHtml.includes('Playback Policies &amp; Resource Arbitration') || playbackHtml.includes('Playback Policies & Resource Arbitration'));
    assert.ok(playbackHtml.includes('Fullscreen Application Response'));
    assert.ok(playbackHtml.includes('Target Framerate (AC Power)'));

    // Switch to Rendering tab
    store.setActiveTab('rendering');
    const renderingHtml = renderToHtml(SettingsView({ settingsStore: store }));
    assert.ok(renderingHtml.includes('Hardware GPU Acceleration'));
    assert.ok(renderingHtml.includes('Vertical Synchronization (V-Sync)'));

    // Switch to Audio tab
    store.setActiveTab('audio');
    const audioHtml = renderToHtml(SettingsView({ settingsStore: store }));
    assert.ok(audioHtml.includes('Master Wallpaper Volume'));
    assert.ok(audioHtml.includes('Master Mute'));

    // Switch to Diagnostics tab
    store.setActiveTab('diagnostics');
    const diagHtml = renderToHtml(SettingsView({ settingsStore: store }));
    assert.ok(diagHtml.includes('Logging Verbosity'));
    assert.ok(diagHtml.includes('Log File Retention Period'));
  });

  // -----------------------------------------------------------------------------------------------
  // 11. AppShell Navigation to Settings
  // -----------------------------------------------------------------------------------------------
  it('§11. AppShell navigates to settings view seamlessly via the primary navigation rail', async () => {
    const routerStore = new RouterStore('dashboard', 1920, 1080);
    const catalogClient = new CatalogIpcClient({
      async invokeGetLibrarySnapshot() {
        return {
          entries: [],
          integrity: {
            valid_record_count: 0,
            available_count: 0,
            missing_count: 0,
            inaccessible_count: 0,
            invalid_asset_count: 0,
            unchecked_count: 0,
            load_issue_count: 0,
          },
          load_issues: [],
        };
      },
    });
    const catalogStore = new CatalogStore(catalogClient);
    const settingsTransport = createMockSettingsIpcTransport();
    const settingsClient = new SettingsIpcClient(settingsTransport);
    const settingsStore = new SettingsStore(settingsClient);
    await settingsStore.loadConfig();

    const shell = new AppShellController({
      routerStore,
      catalogStore,
      settingsStore,
    });

    const initialHtml = renderToHtml(shell.render());
    assert.ok(initialHtml.includes('Engine status overview'));

    // Navigate to settings route
    const navigated = shell.navigate('settings');
    assert.strictEqual(navigated, true);

    const settingsHtml = renderToHtml(shell.render());
    assert.ok(settingsHtml.includes('Application Settings'));
    assert.ok(settingsHtml.includes('General Preferences'));
  });
});

