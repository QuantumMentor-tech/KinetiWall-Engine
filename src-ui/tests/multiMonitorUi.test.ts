/**
 * Step 46: Multi-Monitor Configuration UI & Assignment Management Tests.
 *
 * Verifies:
 * 1. DisplayIpcClient boundary validation, coordinates normalization, and transport mapping.
 * 2. DisplayStore reactive subscriptions, layout mode switching, sequence monotonicity, and rollback.
 * 3. LayoutModeSelector presentation modes, single-display safeguards, and honest sync disclaimer.
 * 4. DisplayArrangementView negative coordinate normalization, primary indicator, and selection.
 * 5. DisplayCard hardware specs, truthful session badge, and assignment action controls.
 * 6. WallpaperPickerModal accessibility dialog, catalog availability filtering, and selection.
 * 7. DisplaysView complete integration with AppShell and DashboardView navigation.
 */

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

import { AppShellController } from '../components/shell/AppShell.ts';
import { DisplayArrangementView } from '../components/domain/DisplayArrangementView.ts';
import { DisplayCard } from '../components/domain/DisplayCard.ts';
import { LayoutModeSelector } from '../components/domain/LayoutModeSelector.ts';
import { WallpaperPickerModal } from '../components/domain/WallpaperPickerModal.ts';
import {
  clickNode,
  findNodeByTestId,
  renderToHtml,
} from '../components/primitives/uiNode.ts';
import { NAVIGATION_ROUTES, getRouteDefinition } from '../routes.ts';
import { CatalogIpcClient } from '../services/catalogIpcClient.ts';
import {
  DisplayIpcClient,
  createMockDisplayIpcTransport,
  type DisplayIpcTransport,
} from '../services/displayIpcClient.ts';
import { CatalogStore } from '../stores/catalogStore.ts';
import { DisplayStore } from '../stores/displayStore.ts';
import { RouterStore } from '../stores/routerStore.ts';
import type { CatalogEntrySummary } from '../types/catalog.ts';
import type {
  DisplayDescriptorDto,
  DisplayTopologySnapshotDto,
  ResolvedDisplayAssignmentDto,
} from '../types/display.ts';
import { DashboardView } from '../views/DashboardView.ts';
import { DisplaysView } from '../views/DisplaysView.ts';

function createMockTopology(): DisplayTopologySnapshotDto {
  return {
    generation: 1,
    displays: [
      {
        id: '\\\\.\\DISPLAY1',
        deviceName: '\\\\.\\DISPLAY1',
        friendlyName: 'Primary Gaming Monitor 144Hz',
        hardwareId: 'MONITOR\\GSM5B01',
        isPrimary: true,
        isAttached: true,
        geometry: {
          left: 0,
          top: 0,
          right: 2560,
          bottom: 1440,
          width: 2560,
          height: 1440,
          scaleFactorPercent: 100,
        },
      },
      {
        id: '\\\\.\\DISPLAY2',
        deviceName: '\\\\.\\DISPLAY2',
        friendlyName: 'Secondary Side Monitor',
        hardwareId: 'MONITOR\\DELA0BC',
        isPrimary: false,
        isAttached: true,
        geometry: {
          left: -1920, // Negative coordinate to test virtual desktop normalization
          top: 0,
          right: 0,
          bottom: 1080,
          width: 1920,
          height: 1080,
          scaleFactorPercent: 125,
        },
      },
    ],
    primaryDisplayId: '\\\\.\\DISPLAY1',
    virtualDesktopBounds: {
      left: -1920,
      top: 0,
      right: 2560,
      bottom: 1440,
      width: 4480,
      height: 1440,
    },
    topologyType: 'dual_extended',
  };
}

function createMockCatalogWallpapers(): CatalogEntrySummary[] {
  return [
    {
      id: 'wp-synthwave-neon',
      title: 'Synthwave Neon Drive',
      author: 'KinetiWall Arts',
      wallpaper_type: 'video',
      source_kind: 'managed_package',
      tags: ['synthwave', 'retrowave', 'loop'],
      availability: { status: 'available' },
      is_builtin: true,
      has_thumbnail_ref: true,
      created_at: '2026-11-15T00:00:00Z',
      updated_at: '2026-11-15T00:00:00Z',
    },
    {
      id: 'wp-broken-asset',
      title: 'Broken Missing Asset',
      author: 'Unknown',
      wallpaper_type: 'video',
      source_kind: 'external_direct_file',
      tags: ['missing'],
      availability: { status: 'missing' },
      is_builtin: false,
      has_thumbnail_ref: false,
      created_at: '2026-11-15T00:00:00Z',
      updated_at: '2026-11-15T00:00:00Z',
    },
  ];
}

describe('Step 46: Multi-Monitor Configuration UI & Assignment Management', () => {
  it('1. Navigation Registry: Route `displays` is marked available and can be resolved', () => {
    const route = getRouteDefinition('displays');
    assert.strictEqual(route.id, 'displays');
    assert.strictEqual(route.isAvailable, true);
    assert.strictEqual(route.unavailableReason, undefined);
    assert.strictEqual(route.iconName, 'monitor');
  });

  it('2. DisplayIpcClient: Parses topology snapshot, normalizes bounds, and resolves assignments', async () => {
    const transport = createMockDisplayIpcTransport();
    const client = new DisplayIpcClient(transport);

    const snapshot = await client.getTopologySnapshot();
    assert.strictEqual(snapshot.displays.length, 2);
    assert.strictEqual(snapshot.primaryDisplayId, '\\\\.\\DISPLAY1');
    assert.strictEqual(snapshot.displays[0].isPrimary, true);
    assert.strictEqual(snapshot.virtualDesktopBounds.width, 4480);

    const assignments = await client.getResolvedAssignments();
    assert.strictEqual(assignments.length, 2);
    assert.strictEqual(assignments[0].wallpaperId, 'wp-cyberpunk-city');

    const multiStatus = await client.getMultiMonitorStatus();
    assert.strictEqual(multiStatus.savedMode, 'independent');
    assert.strictEqual(multiStatus.sessions.length, 1);
    assert.strictEqual(multiStatus.sessions[0].state, 'active');
  });

  it('3. DisplayStore: Loads topology, handles display selection and reactive notifications', async () => {
    const transport = createMockDisplayIpcTransport();
    const client = new DisplayIpcClient(transport);
    const store = new DisplayStore(client);

    let notifyCount = 0;
    store.subscribe(() => {
      notifyCount++;
    });

    await store.refreshTopology();
    assert.ok(notifyCount >= 2); // loading -> ready

    const state = store.getState();
    assert.strictEqual(state.isLoading, false);
    assert.strictEqual(state.error, null);
    assert.strictEqual(state.snapshot?.displays.length, 2);
    assert.strictEqual(state.selectedDisplayId, '\\\\.\\DISPLAY1'); // Defaults to primary

    // Select second display
    store.selectDisplay('\\\\.\\DISPLAY2');
    assert.strictEqual(store.getState().selectedDisplayId, '\\\\.\\DISPLAY2');
  });

  it('4. DisplayStore: Assigns and removes wallpapers with sequence integrity and safe rollback', async () => {
    const transport = createMockDisplayIpcTransport();
    const client = new DisplayIpcClient(transport);
    const store = new DisplayStore(client);
    await store.refreshTopology();

    // Assign wallpaper to display 2
    const assignSuccess = await store.assignWallpaper('\\\\.\\DISPLAY2', 'wp-synthwave-neon');
    assert.strictEqual(assignSuccess, true);

    const stateAfterAssign = store.getState();
    const assigned2 = stateAfterAssign.resolvedAssignments.find((a) => a.displayId === '\\\\.\\DISPLAY2');
    assert.strictEqual(assigned2?.wallpaperId, 'wp-synthwave-neon');

    // Remove assignment from display 2
    const removeSuccess = await store.removeAssignment('\\\\.\\DISPLAY2');
    assert.strictEqual(removeSuccess, true);

    const stateAfterRemove = store.getState();
    const removed2 = stateAfterRemove.resolvedAssignments.find((a) => a.displayId === '\\\\.\\DISPLAY2');
    assert.strictEqual(removed2?.wallpaperId, undefined);

    // Rollback test: failing IPC
    const failingTransport: DisplayIpcTransport = {
      ...transport,
      async invokeSetLayoutMode() {
        throw new Error('Host rejected layout transition: conflicting geometry');
      },
    };
    const failingStore = new DisplayStore(new DisplayIpcClient(failingTransport), { savedMode: 'independent' });
    const layoutSuccess = await failingStore.setLayoutMode('span');
    assert.strictEqual(layoutSuccess, false);
    assert.strictEqual(failingStore.getState().savedMode, 'independent'); // Safely rolled back!
    assert.ok(failingStore.getState().error?.includes('conflicting geometry'));
  });

  it('5. LayoutModeSelector: Displays 3 modes, enforces single-display safeguards, and shows honest sync note', () => {
    let chosenMode = '';

    // Multi-display case: all enabled
    const multiNode = LayoutModeSelector({
      currentMode: 'independent',
      displayCount: 2,
      onSelectMode: (mode) => {
        chosenMode = mode;
      },
    });

    const multiHtml = renderToHtml(multiNode);
    assert.ok(multiHtml.includes('Independent'));
    assert.ok(multiHtml.includes('Span Virtual Desktop'));
    assert.ok(multiHtml.includes('Clone / Duplicate'));
    assert.ok(multiHtml.includes('Best-effort software synchronization (hardware genlock not claimed)'));

    const spanOption = findNodeByTestId(multiNode, 'layout-mode-option-span');
    assert.ok(spanOption);
    assert.strictEqual(spanOption.attrs['data-disabled'], 'false');

    clickNode(spanOption);
    assert.strictEqual(chosenMode, 'span');

    // Single-display case: span and clone disabled with safeguards
    const singleNode = LayoutModeSelector({
      currentMode: 'independent',
      displayCount: 1,
      onSelectMode: (mode) => {
        chosenMode = mode;
      },
    });

    const singleSpan = findNodeByTestId(singleNode, 'layout-mode-option-span');
    assert.ok(singleSpan);
    assert.strictEqual(singleSpan.attrs['data-disabled'], 'true');
    assert.ok(renderToHtml(singleNode).includes('Requires 2 or more connected displays'));
  });

  it('6. DisplayArrangementView: Handles negative coordinates safely and displays primary and active indicators', () => {
    const topology = createMockTopology();
    let selectedId = '';

    const viewNode = DisplayArrangementView({
      snapshot: topology,
      selectedDisplayId: '\\\\.\\DISPLAY1',
      resolvedAssignments: [
        {
          displayId: '\\\\.\\DISPLAY1',
          displayPresence: 'connected',
          outcome: 'valid_explicit',
          isPlayable: true,
          effectiveTitle: 'Cyberpunk 2077 Loop',
          snapshotGeneration: 1,
        },
      ],
      sessions: [
        {
          sessionId: 's-1',
          displayId: '\\\\.\\DISPLAY1',
          wallpaperId: 'wp-1',
          state: 'active',
          geometry: topology.displays[0].geometry,
          generation: 1,
          createdAtMillis: Date.now(),
          lastUpdatedMillis: Date.now(),
        },
      ],
      onSelectDisplay: (id) => {
        selectedId = id;
      },
    });

    const html = renderToHtml(viewNode);
    assert.ok(html.includes('Primary Gaming Monitor 144Hz'));
    assert.ok(html.includes('Secondary Side Monitor'));
    assert.ok(html.includes('★')); // Primary display star
    assert.ok(html.includes('●')); // Live wallpaper active dot

    const box2 = findNodeByTestId(viewNode, 'display-box-\\\\.\\DISPLAY2');
    assert.ok(box2);
    // Verify the style left coordinate is non-negative despite virtual desktop bounds starting at -1920
    const styleAttr = String(box2.attrs.style);
    const leftMatch = styleAttr.match(/left:\s*([\d.]+)px/);
    assert.ok(leftMatch, 'Must contain non-negative left offset');
    assert.ok(parseFloat(leftMatch[1]) >= 0, 'Left position must be normalized to >= 0');

    clickNode(box2);
    assert.strictEqual(selectedId, '\\\\.\\DISPLAY2');
  });

  it('7. DisplayCard: Renders hardware specs, truthful session badges, and triggers picker/clear actions', () => {
    const topology = createMockTopology();
    const display = topology.displays[0];
    let changed = false;
    let cleared = false;

    const cardNode = DisplayCard({
      display,
      assignment: {
        displayId: display.id,
        displayPresence: 'connected',
        wallpaperId: 'wp-cyberpunk',
        assignmentKind: 'explicit',
        outcome: 'valid_explicit',
        isPlayable: true,
        effectiveTitle: 'Neon Cyberpunk',
        snapshotGeneration: 1,
      },
      session: {
        sessionId: 'sess-1',
        displayId: display.id,
        wallpaperId: 'wp-cyberpunk',
        state: 'active',
        geometry: display.geometry,
        generation: 1,
        createdAtMillis: Date.now(),
        lastUpdatedMillis: Date.now(),
      },
      isSelected: true,
      layoutMode: 'independent',
      onSelect: () => {},
      onChangeWallpaper: () => {
        changed = true;
      },
      onClearAssignment: () => {
        cleared = true;
      },
    });

    const html = renderToHtml(cardNode);
    assert.ok(html.includes('Primary Gaming Monitor 144Hz'));
    assert.ok(html.includes('2560 × 1440'));
    assert.ok(html.includes('Live Wallpaper Active'));
    assert.ok(html.includes('Explicit assignment'));

    const changeBtn = findNodeByTestId(cardNode, `display-change-wallpaper-btn-${display.id}`);
    assert.ok(changeBtn);
    clickNode(changeBtn);
    assert.strictEqual(changed, true);

    const clearBtn = findNodeByTestId(cardNode, `display-clear-assignment-btn-${display.id}`);
    assert.ok(clearBtn);
    clickNode(clearBtn);
    assert.strictEqual(cleared, true);
  });

  it('8. WallpaperPickerModal: Accessible modal enforces catalog availability filters', () => {
    const topology = createMockTopology();
    const wallpapers = createMockCatalogWallpapers();
    let selectedWpId = '';
    let closed = false;

    const modalNode = WallpaperPickerModal({
      isOpen: true,
      targetDisplay: topology.displays[0],
      wallpapers,
      isAssigning: false,
      onSelectWallpaper: (id) => {
        selectedWpId = id;
      },
      onClose: () => {
        closed = true;
      },
    });

    assert.ok(modalNode);
    const html = renderToHtml(modalNode);
    assert.ok(html.includes('Assign Wallpaper'));
    assert.ok(html.includes('Synthwave Neon Drive'));
    assert.ok(html.includes('Broken Missing Asset'));

    // Available wallpaper assign button is active
    const availableBtn = findNodeByTestId(modalNode, 'picker-select-btn-wp-synthwave-neon');
    assert.ok(availableBtn);
    assert.strictEqual(availableBtn.attrs.disabled, false);
    clickNode(availableBtn);
    assert.strictEqual(selectedWpId, 'wp-synthwave-neon');

    // Missing wallpaper assign button is disabled
    const missingBtn = findNodeByTestId(modalNode, 'picker-select-btn-wp-broken-asset');
    assert.ok(missingBtn);
    assert.strictEqual(missingBtn.attrs.disabled, true);

    // Cancel button closes modal
    const cancelBtn = findNodeByTestId(modalNode, 'wallpaper-picker-cancel-btn');
    assert.ok(cancelBtn);
    clickNode(cancelBtn);
    assert.strictEqual(closed, true);
  });

  it('9. DisplaysView Integration: Renders full multi-monitor layout, handles refresh, and applies layout', async () => {
    const transport = createMockDisplayIpcTransport();
    const client = new DisplayIpcClient(transport);
    const displayStore = new DisplayStore(client);
    await displayStore.refreshTopology();

    const catalogStore = new CatalogStore(
      new CatalogIpcClient({
        async invokeGetLibrarySnapshot() {
          return { entries: createMockCatalogWallpapers(), integrity: {}, load_issues: [] };
        },
        async invokeRegisterWallpapers() {
          return { registered_count: 0, items: [] };
        },
      }),
    );
    await catalogStore.loadCatalog();

    const viewNode = DisplaysView({ displayStore, catalogStore });
    const html = renderToHtml(viewNode);

    assert.ok(html.includes('Displays &amp; Layout') || html.includes('Displays & Layout'));
    assert.ok(html.includes('2 Displays Detected'));
    assert.ok(html.includes('1 Live Wallpaper'));
    assert.ok(html.includes('Display Arrangement'));
    assert.ok(html.includes('Connected Monitors'));

    // Apply layout button interaction triggers async application
    const applyBtn = findNodeByTestId(viewNode, 'displays-apply-layout-btn');
    assert.ok(applyBtn);
    clickNode(applyBtn);
    // In flight
    assert.strictEqual(displayStore.getState().isApplying, true);
    // Wait for in-memory IPC completion
    await new Promise((resolve) => setTimeout(resolve, 20));
    assert.strictEqual(displayStore.getState().isApplying, false);
  });

  it('10. AppShell & DashboardView: Navigation to displays works seamlessly', async () => {
    const routerStore = new RouterStore('dashboard', 1920, 1080);
    const catalogStore = new CatalogStore(
      new CatalogIpcClient({
        async invokeGetLibrarySnapshot() {
          return { entries: [], integrity: {}, load_issues: [] };
        },
        async invokeRegisterWallpapers() {
          return { registered_count: 0, items: [] };
        },
      }),
    );
    const displayStore = new DisplayStore(new DisplayIpcClient(createMockDisplayIpcTransport()));
    await displayStore.refreshTopology();

    const shell = new AppShellController({ routerStore, catalogStore, displayStore });

    // Verify Dashboard contains Displays summary card and quick navigation button
    const dashboardHtml = renderToHtml(shell.render());
    assert.ok(dashboardHtml.includes('Displays &amp; Layout') || dashboardHtml.includes('Displays & Layout'));
    assert.ok(dashboardHtml.includes('Inspect connected monitors'));

    // Navigate to displays
    const navigated = shell.navigate('displays');
    assert.strictEqual(navigated, true);
    assert.strictEqual(routerStore.getState().activeRoute, 'displays');

    // Verify shell now renders DisplaysView
    const displaysHtml = renderToHtml(shell.render());
    assert.ok(displaysHtml.includes('Displays &amp; Layout') || displaysHtml.includes('Displays & Layout'));
    assert.ok(displaysHtml.includes('data-testid="displays-view"'));
    assert.ok(displaysHtml.includes('data-testid="display-arrangement-canvas"'));
    assert.ok(displaysHtml.includes('data-testid="layout-mode-selector"'));
    assert.ok(displaysHtml.includes('data-testid="smart-playback-policy-section"'));
    assert.ok(displaysHtml.includes('Smart Pause on Fullscreen Apps'));
    assert.ok(displaysHtml.includes('data-testid="power-policy-card"'));
    assert.ok(displaysHtml.includes('data-testid="power-status-badge"'));
    assert.ok(displaysHtml.includes('data-testid="power-mode-selector"'));
    assert.ok(displaysHtml.includes('Power-State &amp; Battery-Sensitive Playback') || displaysHtml.includes('Power-State & Battery-Sensitive Playback'));
  });

  it('toggles smart pause fullscreen playback policy in DisplayStore and UI', async () => {
    const displayStore = new DisplayStore(new DisplayIpcClient(createMockDisplayIpcTransport()));
    await displayStore.refreshTopology();

    assert.strictEqual(displayStore.getState().fullscreenPolicy.enabled, true);

    // Toggle off
    const toggledOff = await displayStore.toggleSmartPauseFullscreen();
    assert.strictEqual(toggledOff, true);
    assert.strictEqual(displayStore.getState().fullscreenPolicy.enabled, false);

    // Toggle on
    const toggledOn = await displayStore.toggleSmartPauseFullscreen();
    assert.strictEqual(toggledOn, true);
    assert.strictEqual(displayStore.getState().fullscreenPolicy.enabled, true);
  });

  it('loads power snapshot and switches power policy modes in DisplayStore', async () => {
    const displayStore = new DisplayStore(new DisplayIpcClient(createMockDisplayIpcTransport()));
    await displayStore.refreshTopology();

    // Verify initial power state from mock
    assert.strictEqual(displayStore.getState().powerSnapshot.powerSource, 'ac');
    assert.strictEqual(displayStore.getState().powerSnapshot.batteryPercent, 85);
    assert.strictEqual(displayStore.getState().powerSettings.mode, 'off');

    // Switch to balanced mode
    const setBalanced = await displayStore.setPowerPolicyMode('balanced');
    assert.strictEqual(setBalanced, true);
    assert.strictEqual(displayStore.getState().powerSettings.mode, 'balanced');

    // Switch to save_battery mode
    const setSaveBattery = await displayStore.setPowerPolicyMode('save_battery');
    assert.strictEqual(setSaveBattery, true);
    assert.strictEqual(displayStore.getState().powerSettings.mode, 'save_battery');

    // Switch back to off
    const setOff = await displayStore.setPowerPolicyMode('off');
    assert.strictEqual(setOff, true);
    assert.strictEqual(displayStore.getState().powerSettings.mode, 'off');
  });

  it('renders static fallback session badge in DisplayCard and DisplaysView', () => {
    const display = {
      id: '\\\\.\\DISPLAY1',
      deviceName: '\\\\.\\DISPLAY1',
      friendlyName: 'LG UltraGear 27"',
      isPrimary: true,
      isAttached: true,
      geometry: { left: 0, top: 0, right: 2560, bottom: 1440, width: 2560, height: 1440 },
    };

    const fallbackSession = {
      sessionId: 'sess-fb-1',
      displayId: '\\\\.\\DISPLAY1',
      wallpaperId: 'wp-cyberpunk-city',
      state: 'static_fallback' as const,
      fallbackSource: 'Catalog Poster',
      geometry: display.geometry,
      generation: 1,
      createdAtMillis: Date.now() - 10000,
      lastUpdatedMillis: Date.now(),
    };

    const cardNode = DisplayCard({
      display,
      session: fallbackSession,
      isSelected: false,
      layoutMode: 'independent',
      onSelect: () => {},
      onChangeWallpaper: () => {},
      onClearAssignment: () => {},
    });

    const html = renderToHtml(cardNode);
    assert.ok(html.includes('Fallback: Catalog Poster'));
    assert.ok(html.includes('🖼'));
  });

  it('toggles static fallback setting in DisplayStore and reflects in UI', async () => {
    const displayStore = new DisplayStore(new DisplayIpcClient(createMockDisplayIpcTransport()));
    await displayStore.refreshTopology();

    assert.strictEqual(displayStore.getState().powerSettings.enableStaticFallback, true);

    // Toggle off
    const toggledOff = await displayStore.toggleStaticFallback();
    assert.strictEqual(toggledOff, true);
    assert.strictEqual(displayStore.getState().powerSettings.enableStaticFallback, false);

    // Toggle back on
    const toggledOn = await displayStore.toggleStaticFallback();
    assert.strictEqual(toggledOn, true);
    assert.strictEqual(displayStore.getState().powerSettings.enableStaticFallback, true);
  });

  it('toggles adaptive quality setting in DisplayStore and maintains opt-in default', async () => {
    const displayStore = new DisplayStore(new DisplayIpcClient(createMockDisplayIpcTransport()));
    await displayStore.refreshTopology();

    // STRICT MANDATE: Must be disabled by default (opt-in)
    assert.strictEqual(displayStore.getState().adaptiveSettings.enabled, false);
    assert.strictEqual(displayStore.getState().adaptiveSettings.targetFpsFloor, 30);

    // Toggle on
    const toggledOn = await displayStore.toggleAdaptiveQuality();
    assert.strictEqual(toggledOn, true);
    assert.strictEqual(displayStore.getState().adaptiveSettings.enabled, true);

    // Toggle back off
    const toggledOff = await displayStore.toggleAdaptiveQuality();
    assert.strictEqual(toggledOff, true);
    assert.strictEqual(displayStore.getState().adaptiveSettings.enabled, false);
  });

  it('renders adaptive quality badge on DisplayCard when reduced tier is active', () => {
    const display = {
      id: '\\\\.\\DISPLAY1',
      deviceName: '\\\\.\\DISPLAY1',
      friendlyName: 'LG UltraGear 27"',
      isPrimary: true,
      isAttached: true,
      geometry: { left: 0, top: 0, right: 2560, bottom: 1440, width: 2560, height: 1440 },
    };

    const throttledSession = {
      sessionId: 'sess-adaptive-1',
      displayId: '\\\\.\\DISPLAY1',
      wallpaperId: 'wp-cyberpunk-city',
      state: 'active' as const,
      effectiveQualityLevel: 'reduced' as const,
      effectiveFps: 30,
      geometry: display.geometry,
      generation: 1,
      createdAtMillis: Date.now() - 10000,
      lastUpdatedMillis: Date.now(),
    };

    const cardNode = DisplayCard({
      display,
      session: throttledSession,
      isSelected: false,
      layoutMode: 'independent',
      onSelect: () => {},
      onChangeWallpaper: () => {},
      onClearAssignment: () => {},
    });

    const html = renderToHtml(cardNode);
    assert.ok(html.includes('data-testid="display-adaptive-badge-\\\\.\\DISPLAY1"'));
    assert.ok(html.includes('Adaptive (30 FPS)'));
    assert.ok(html.includes('⚡'));
  });

  it('initializes resource budget disabled by default and supports toggle via store', async () => {
    const transport = createMockDisplayIpcTransport();
    const client = new DisplayIpcClient(transport);
    const displayStore = new DisplayStore(client);
    await displayStore.refreshTopology();

    const initialState = displayStore.getState();
    assert.strictEqual(initialState.resourceBudget.enabled, false);
    assert.strictEqual(initialState.budgetState, 'disabled');
    assert.strictEqual(initialState.resourceBudget.targetAggregateCpuPercent, 75.0);

    const toggledOn = await displayStore.toggleResourceBudget();
    assert.strictEqual(toggledOn, true);
    assert.strictEqual(displayStore.getState().resourceBudget.enabled, true);
    assert.strictEqual(displayStore.getState().budgetState, 'within_budget');

    const toggledOff = await displayStore.toggleResourceBudget();
    assert.strictEqual(toggledOff, true);
    assert.strictEqual(displayStore.getState().resourceBudget.enabled, false);
    assert.strictEqual(displayStore.getState().budgetState, 'disabled');
  });

  it('DisplaysView renders dynamic resource budget info and toggle', async () => {
    const transport = createMockDisplayIpcTransport();
    const client = new DisplayIpcClient(transport);
    const displayStore = new DisplayStore(client);
    await displayStore.refreshTopology();

    const catalogStore = new CatalogStore(
      new CatalogIpcClient({
        async invokeGetLibrarySnapshot() {
          return { entries: createMockCatalogWallpapers(), integrity: {}, load_issues: [] };
        },
        async invokeRegisterWallpapers() {
          return { registered_count: 0, items: [] };
        },
      }),
    );
    await catalogStore.loadCatalog();

    const viewNode = DisplaysView({ displayStore, catalogStore });
    const html = renderToHtml(viewNode);

    assert.ok(html.includes('data-testid="resource-budget-info"'));
    assert.ok(html.includes('Resource Budget Arbiter'));
    assert.ok(html.includes('Target: 75% CPU'));
    assert.ok(html.includes('data-testid="toggle-resource-budget-button"'));
    assert.ok(html.includes('Disabled (Default)'));
  });

  it('displayStore session settings default to disabled and toggleLockPause updates state', async () => {
    const transport = createMockDisplayIpcTransport();
    const client = new DisplayIpcClient(transport);
    const displayStore = new DisplayStore(client);
    await displayStore.refreshTopology();

    const initialState = displayStore.getState();
    assert.strictEqual(initialState.sessionSettings.enabled, false);
    assert.strictEqual(initialState.sessionSettings.pauseOnLock, true);
    assert.strictEqual(initialState.sessionSnapshot.lockState, 'unlocked');

    const toggledOn = await displayStore.toggleLockPause();
    assert.strictEqual(toggledOn, true);
    assert.strictEqual(displayStore.getState().sessionSettings.enabled, true);

    const toggledOff = await displayStore.toggleLockPause();
    assert.strictEqual(toggledOff, true);
    assert.strictEqual(displayStore.getState().sessionSettings.enabled, false);
  });

  it('DisplaysView renders session lifecycle policy info, status, and toggle button', async () => {
    const transport = createMockDisplayIpcTransport();
    const client = new DisplayIpcClient(transport);
    const displayStore = new DisplayStore(client);
    await displayStore.refreshTopology();

    const catalogStore = new CatalogStore(
      new CatalogIpcClient({
        async invokeGetLibrarySnapshot() {
          return { entries: createMockCatalogWallpapers(), integrity: {}, load_issues: [] };
        },
        async invokeRegisterWallpapers() {
          return { registered_count: 0, items: [] };
        },
      }),
    );
    await catalogStore.loadCatalog();

    const viewNode = DisplaysView({ displayStore, catalogStore });
    const html = renderToHtml(viewNode);

    assert.ok(html.includes('data-testid="session-policy-info"'));
    assert.ok(html.includes('Session Lifecycle Awareness'));
    assert.ok(html.includes('Lock: unlocked'));
    assert.ok(html.includes('data-testid="toggle-lock-pause-button"'));
    assert.ok(html.includes('Disabled (Default)'));
  });

  it('displayStore display power settings default to disabled and toggleDisplayPowerPause updates state', async () => {
    const transport = createMockDisplayIpcTransport();
    const client = new DisplayIpcClient(transport);
    const displayStore = new DisplayStore(client);
    await displayStore.refreshTopology();

    const initialState = displayStore.getState();
    assert.strictEqual(initialState.displayPowerSettings.enabled, false);
    assert.strictEqual(initialState.displayPowerSettings.pauseOnDisplayOff, true);
    assert.strictEqual(initialState.displayPowerSnapshot.state, 'on');

    const toggledOn = await displayStore.toggleDisplayPowerPause();
    assert.strictEqual(toggledOn, true);
    assert.strictEqual(displayStore.getState().displayPowerSettings.enabled, true);

    const toggledOff = await displayStore.toggleDisplayPowerPause();
    assert.strictEqual(toggledOff, true);
    assert.strictEqual(displayStore.getState().displayPowerSettings.enabled, false);
  });

  it('DisplaysView renders display power policy info, status, and toggle button', async () => {
    const transport = createMockDisplayIpcTransport();
    const client = new DisplayIpcClient(transport);
    const displayStore = new DisplayStore(client);
    await displayStore.refreshTopology();

    const catalogStore = new CatalogStore(
      new CatalogIpcClient({
        async invokeGetLibrarySnapshot() {
          return { entries: createMockCatalogWallpapers(), integrity: {}, load_issues: [] };
        },
        async invokeRegisterWallpapers() {
          return { registered_count: 0, items: [] };
        },
      }),
    );
    await catalogStore.loadCatalog();

    const viewNode = DisplaysView({ displayStore, catalogStore });
    const html = renderToHtml(viewNode);

    assert.ok(html.includes('data-testid="display-power-policy-info"'));
    assert.ok(html.includes('Display-Power Awareness'));
    assert.ok(html.includes('Display: on'));
    assert.ok(html.includes('data-testid="toggle-display-power-button"'));
    assert.ok(html.includes('Disabled (Default)'));
  });

  it('displayStore inactivity settings default to disabled and toggleInactivityPause updates state', async () => {
    const transport = createMockDisplayIpcTransport();
    const client = new DisplayIpcClient(transport);
    const displayStore = new DisplayStore(client);
    await displayStore.refreshTopology();

    const initialState = displayStore.getState();
    assert.strictEqual(initialState.inactivitySettings.enabled, false);
    assert.strictEqual(initialState.inactivitySettings.idleThresholdMillis, 600000);
    assert.strictEqual(initialState.inactivitySnapshot.state, 'active');

    const toggledOn = await displayStore.toggleInactivityPause();
    assert.strictEqual(toggledOn, true);
    assert.strictEqual(displayStore.getState().inactivitySettings.enabled, true);

    const toggledOff = await displayStore.toggleInactivityPause();
    assert.strictEqual(toggledOff, true);
    assert.strictEqual(displayStore.getState().inactivitySettings.enabled, false);
  });

  it('DisplaysView renders user inactivity policy info, status, and toggle button', async () => {
    const transport = createMockDisplayIpcTransport();
    const client = new DisplayIpcClient(transport);
    const displayStore = new DisplayStore(client);
    await displayStore.refreshTopology();

    const catalogStore = new CatalogStore(
      new CatalogIpcClient({
        async invokeGetLibrarySnapshot() {
          return { entries: createMockCatalogWallpapers(), integrity: {}, load_issues: [] };
        },
        async invokeRegisterWallpapers() {
          return { registered_count: 0, items: [] };
        },
      }),
    );
    await catalogStore.loadCatalog();

    const viewNode = DisplaysView({ displayStore, catalogStore });
    const html = renderToHtml(viewNode);

    assert.ok(html.includes('data-testid="inactivity-policy-info"'));
    assert.ok(html.includes('User-Inactivity Awareness'));
    assert.ok(html.includes('State: active'));
    assert.ok(html.includes('data-testid="toggle-inactivity-button"'));
    assert.ok(html.includes('Disabled (Default)'));
  });
});

  it('renders policy decision diagnostics and divergence note on DisplayCard (Step 57)', () => {
    const display: DisplayDescriptorDto = {
      id: 'disp_1',
      deviceName: '\\\\.\\DISPLAY1',
      isPrimary: true,
      isAttached: true,
      geometry: { left: 0, top: 0, right: 1920, bottom: 1080, width: 1920, height: 1080 },
    };

    const cardNode = DisplayCard({
      display,
      isSelected: false,
      layoutMode: 'independent',
      policyDecision: {
        displayId: 'disp_1',
        desiredState: 'suspended',
        actualState: 'active',
        explanationCodes: ['fullscreen_active'],
        primaryExplanation: 'Suspended due to active foreground fullscreen application',
        primaryReasonCode: 'FULLSCREEN_ACTIVE',
        isStateDivergent: true,
        requiresTransition: true,
        evaluatedAtMillis: 1000,
      },
      onSelect: () => {},
      onChangeWallpaper: () => {},
      onClearAssignment: () => {},
    });

    const html = renderToHtml(cardNode);
    assert.ok(html.includes('data-testid="display-policy-diagnostics-disp_1"'));
    assert.ok(html.includes('FULLSCREEN_ACTIVE'));
    assert.ok(html.includes('Suspended due to active foreground fullscreen application'));
    assert.ok(html.includes('Reconciling transition: desired=suspended, backend=active'));
  });

  it('updates policy decisions in DisplayStore and displays them in DisplaysView (Step 57)', async () => {
    const displayStore = new DisplayStore(new DisplayIpcClient(createMockDisplayIpcTransport()));
    await displayStore.refreshTopology();

    const did = displayStore.getState().snapshot?.displays[0]?.id ?? 'disp_1';
    displayStore.updatePolicyDecision({
      displayId: did,
      desiredState: 'active',
      actualState: 'active',
      explanationCodes: ['unconstrained_active'],
      primaryExplanation: 'All constraints cleared; playback active',
      primaryReasonCode: 'UNCONSTRAINED_ACTIVE',
      isStateDivergent: false,
      requiresTransition: false,
      evaluatedAtMillis: 2000,
    });

    assert.strictEqual(displayStore.getState().policyDecisions[did]?.primaryReasonCode, 'UNCONSTRAINED_ACTIVE');

    const catalogStore = new CatalogStore(
      new CatalogIpcClient({
        async invokeGetLibrarySnapshot() {
          return { entries: [], integrity: {}, load_issues: [] };
        },
        async invokeRegisterWallpapers() {
          return { registered_count: 0, items: [] };
        },
      }),
    );

    const viewNode = DisplaysView({
      displayStore,
      catalogStore,
    });

    const html = renderToHtml(viewNode);
    assert.ok(html.includes(`data-testid="display-policy-diagnostics-${did}"`));
    assert.ok(html.includes('UNCONSTRAINED_ACTIVE'));
    assert.ok(html.includes('All constraints cleared; playback active'));
  });
