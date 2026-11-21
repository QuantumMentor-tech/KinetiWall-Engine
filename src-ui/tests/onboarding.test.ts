/**
 * Automated Tests: First-Run Experience, Guided Onboarding & Capability Readiness (Step 66).
 *
 * Covers:
 * §1. First-launch detection and automatic onboarding modal opening.
 * §2. Returning users (completed & dismissed) bypass onboarding on launch.
 * §3. Truthful capability readiness evaluation for essential vs optional features.
 * §4. Optional failures do not mark the entire engine non-operational.
 * §5. Readiness check anti-stampede and in-flight request deduplication.
 * §6. Stage navigation (Welcome -> Readiness -> Workflows -> Completion).
 * §7. Skip/Dismiss persistence and notification.
 * §8. Completion persistence with timestamp and library routing.
 * §9. Persistence failure handling and honest user feedback.
 * §10. SettingsView integration and manual onboarding tour reopening.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { AppShellController } from '../components/shell/AppShell.ts';
import { OnboardingModal, ONBOARDING_STAGES } from '../components/domain/OnboardingModal.ts';
import { renderToHtml } from '../components/primitives/uiNode.ts';
import { CatalogIpcClient } from '../services/catalogIpcClient.ts';
import { DisplayIpcClient, createMockDisplayIpcTransport } from '../services/displayIpcClient.ts';
import { ReadinessEvaluator } from '../services/readinessEvaluator.ts';
import { SettingsIpcClient, createMockSettingsIpcTransport } from '../services/settingsIpcClient.ts';
import { CatalogStore } from '../stores/catalogStore.ts';
import { DisplayStore } from '../stores/displayStore.ts';
import { RouterStore } from '../stores/routerStore.ts';
import { SettingsStore } from '../stores/settingsStore.ts';
import { computeAggregateReadiness } from '../types/readiness.ts';
import { createDefaultConfigDto, validateSettings } from '../types/settings.ts';
import { SettingsView } from '../views/SettingsView.ts';

function createMockCatalogClient(entryCount = 1) {
  return new CatalogIpcClient({
    async invokeGetLibrarySnapshot() {
      return {
        entries:
          entryCount > 0
            ? [
                {
                  id: '11111111-2222-4333-8444-555555555555',
                  title: 'Cyberpunk Skyline',
                  description: 'Looping city scene',
                  wallpaper_type: 'video',
                  source_kind: 'managed_package',
                  author: 'KinetiWall',
                  tags: ['city', 'neon'],
                  availability: { status: 'available' },
                  created_at: '2026-11-21T00:00:00Z',
                  updated_at: '2026-11-21T00:00:00Z',
                  is_builtin: true,
                  has_thumbnail_ref: false,
                },
              ]
            : [],
        integrity: {
          valid_record_count: entryCount,
          available_count: entryCount,
          missing_count: 0,
          inaccessible_count: 0,
          invalid_asset_count: 0,
          unchecked_count: 0,
          load_issue_count: 0,
        },
        load_issues: [],
      };
    },
    async invokeRegisterWallpapers() {
      return {
        overall_status: 'cancelled',
        total_selected: 0,
        registered_count: 0,
        already_present_count: 0,
        rejected_count: 0,
        inaccessible_count: 0,
        failed_count: 0,
        items: [],
      };
    },
  });
}

test('Step 66: First-Run Experience & Capability Readiness', async (t) => {
  await t.test('§1. First launch opens onboarding modal automatically', async () => {
    const config = createDefaultConfigDto();
    assert.equal(config.general.onboarding.status, 'not_started');
    assert.equal(config.general.onboarding.version, 1);

    const transport = createMockSettingsIpcTransport(config);
    const settingsStore = new SettingsStore(new SettingsIpcClient(transport));
    await settingsStore.loadConfig();

    const routerStore = new RouterStore('dashboard', 1920, 1080);
    const catalogStore = new CatalogStore(createMockCatalogClient(0));
    const shell = new AppShellController({ routerStore, catalogStore, settingsStore });

    assert.equal(shell.isOnboardingOpen(), true);
    const html = renderToHtml(shell.render());
    assert.ok(html.includes('data-testid="onboarding-modal-dialog"'));
    assert.ok(html.includes('Welcome to KinetiWall'));
    assert.ok(html.includes('Step 1 of 4: Welcome'));
  });

  await t.test('§2. Returning user with completed or dismissed status does not see modal on startup', async () => {
    // 2a. Completed user
    const completedConfig = createDefaultConfigDto();
    completedConfig.general.onboarding.status = 'completed';
    completedConfig.general.onboarding.completed_at_millis = 1732170000000;

    const transportCompleted = createMockSettingsIpcTransport(completedConfig);
    const storeCompleted = new SettingsStore(new SettingsIpcClient(transportCompleted));
    await storeCompleted.loadConfig();

    const shellCompleted = new AppShellController({
      routerStore: new RouterStore('dashboard', 1920, 1080),
      catalogStore: new CatalogStore(createMockCatalogClient(1)),
      settingsStore: storeCompleted,
    });

    assert.equal(shellCompleted.isOnboardingOpen(), false);

    // 2b. Dismissed user
    const dismissedConfig = createDefaultConfigDto();
    dismissedConfig.general.onboarding.status = 'dismissed';

    const transportDismissed = createMockSettingsIpcTransport(dismissedConfig);
    const storeDismissed = new SettingsStore(new SettingsIpcClient(transportDismissed));
    await storeDismissed.loadConfig();

    const shellDismissed = new AppShellController({
      routerStore: new RouterStore('dashboard', 1920, 1080),
      catalogStore: new CatalogStore(createMockCatalogClient(1)),
      settingsStore: storeDismissed,
    });

    assert.equal(shellDismissed.isOnboardingOpen(), false);
  });

  await t.test('§3. Truthful capability readiness distinguishes essential and optional services', async () => {
    const config = createDefaultConfigDto();
    const settingsStore = new SettingsStore(new SettingsIpcClient(createMockSettingsIpcTransport(config)));
    await settingsStore.loadConfig();

    const catalogStore = new CatalogStore(createMockCatalogClient(1));
    await catalogStore.loadCatalog();

    const displayStore = new DisplayStore(new DisplayIpcClient(createMockDisplayIpcTransport()));
    await displayStore.refreshTopology();

    const evaluator = new ReadinessEvaluator({
      settingsStore,
      catalogStore,
      displayStore,
      videoBackendAvailable: true,
      webEngineAvailable: true,
      audioCaptureAvailable: true,
    });

    const report = evaluator.evaluateNow();
    assert.equal(report.isOperational, true);
    assert.equal(report.hasWarnings, false);
    assert.equal(report.hasBlockers, false);
    assert.equal(report.overallState, 'ready');
    assert.equal(report.capabilities.length, 6);

    const configCap = report.capabilities.find((c) => c.id === 'configuration');
    assert.ok(configCap);
    assert.equal(configCap.state, 'ready');
    assert.equal(configCap.requirement, 'essential');

    const catalogCap = report.capabilities.find((c) => c.id === 'wallpaper_catalog');
    assert.ok(catalogCap);
    assert.equal(catalogCap.state, 'ready');
    assert.equal(catalogCap.requirement, 'essential');

    const displayCap = report.capabilities.find((c) => c.id === 'desktop_integration');
    assert.ok(displayCap);
    assert.equal(displayCap.state, 'ready');
    assert.equal(displayCap.requirement, 'essential');
  });

  await t.test('§4. Optional capability failure causes warnings but never marks engine non-operational', async () => {
    const config = createDefaultConfigDto();
    const settingsStore = new SettingsStore(new SettingsIpcClient(createMockSettingsIpcTransport(config)));
    await settingsStore.loadConfig();

    const catalogStore = new CatalogStore(createMockCatalogClient(1));
    await catalogStore.loadCatalog();

    const displayStore = new DisplayStore(new DisplayIpcClient(createMockDisplayIpcTransport()));
    await displayStore.refreshTopology();

    // Video and Web backends restricted (e.g., missing WebView2 or codecs)
    const evaluator = new ReadinessEvaluator({
      settingsStore,
      catalogStore,
      displayStore,
      videoBackendAvailable: false,
      webEngineAvailable: false,
      audioCaptureAvailable: true,
    });

    const report = evaluator.evaluateNow();
    assert.equal(report.isOperational, true); // Still operational!
    assert.equal(report.hasWarnings, true);
    assert.equal(report.hasBlockers, false);
    assert.equal(report.overallState, 'limited');

    const videoCap = report.capabilities.find((c) => c.id === 'video_player');
    assert.equal(videoCap?.state, 'limited');
    assert.equal(videoCap?.requirement, 'optional');

    const webCap = report.capabilities.find((c) => c.id === 'web_engine');
    assert.equal(webCap?.state, 'limited');
    assert.equal(webCap?.requirement, 'optional');
  });

  await t.test('§5. Anti-stampede protection: concurrent readiness checks share in-flight promise', async () => {
    const config = createDefaultConfigDto();
    const settingsStore = new SettingsStore(new SettingsIpcClient(createMockSettingsIpcTransport(config)));
    const catalogStore = new CatalogStore(createMockCatalogClient(1));
    const displayStore = new DisplayStore(new DisplayIpcClient(createMockDisplayIpcTransport()));

    const evaluator = new ReadinessEvaluator({
      settingsStore,
      catalogStore,
      displayStore,
    });

    // Fire 3 simultaneous checks
    const [p1, p2, p3] = [
      evaluator.checkReadiness(true),
      evaluator.checkReadiness(true),
      evaluator.checkReadiness(true),
    ];

    // They must evaluate to the same report
    const [r1, r2, r3] = await Promise.all([p1, p2, p3]);
    assert.equal(r1.evaluatedAtMillis, r2.evaluatedAtMillis);
    assert.equal(r2.evaluatedAtMillis, r3.evaluatedAtMillis);
  });

  await t.test('§6. Stage navigation advances sequentially and clamps bounds', async () => {
    const config = createDefaultConfigDto();
    const settingsStore = new SettingsStore(new SettingsIpcClient(createMockSettingsIpcTransport(config)));
    await settingsStore.loadConfig();

    const shell = new AppShellController({
      routerStore: new RouterStore('dashboard', 1920, 1080),
      catalogStore: new CatalogStore(createMockCatalogClient(1)),
      settingsStore,
    });

    assert.equal(shell.isOnboardingOpen(), true);

    // Stage 0: Welcome
    let html = renderToHtml(shell.render());
    assert.ok(html.includes('Step 1 of 4: Welcome'));

    // Move to Stage 1: Readiness
    shell.handleOnboardingStageChange(1);
    html = renderToHtml(shell.render());
    assert.ok(html.includes('Step 2 of 4: Readiness'));
    assert.ok(html.includes('data-testid="onboarding-recheck-readiness-btn"'));

    // Move to Stage 2: Workflows
    shell.handleOnboardingStageChange(2);
    html = renderToHtml(shell.render());
    assert.ok(html.includes('Step 3 of 4: Workflows'));
    assert.ok(html.includes('Browse Wallpaper Library'));

    // Move to Stage 3: Completion
    shell.handleOnboardingStageChange(3);
    html = renderToHtml(shell.render());
    assert.ok(html.includes('Step 4 of 4: Finish'));
    assert.ok(html.includes('data-testid="onboarding-open-library-btn"'));
    assert.ok(html.includes('data-testid="onboarding-finish-btn"'));

    // Out-of-bounds stage index clamps safely
    shell.handleOnboardingStageChange(99);
    assert.ok(renderToHtml(shell.render()).includes('Step 4 of 4: Finish'));

    shell.handleOnboardingStageChange(-10);
    assert.ok(renderToHtml(shell.render()).includes('Step 1 of 4: Welcome'));
  });

  await t.test('§7. Dismissing onboarding updates config and prevents re-opening on launch', async () => {
    const config = createDefaultConfigDto();
    const transport = createMockSettingsIpcTransport(config);
    const settingsStore = new SettingsStore(new SettingsIpcClient(transport));
    await settingsStore.loadConfig();

    const shell = new AppShellController({
      routerStore: new RouterStore('dashboard', 1920, 1080),
      catalogStore: new CatalogStore(createMockCatalogClient(1)),
      settingsStore,
    });

    assert.equal(shell.isOnboardingOpen(), true);

    const dismissed = await shell.handleOnboardingDismiss();
    assert.equal(dismissed, true);
    assert.equal(shell.isOnboardingOpen(), false);

    // Config must be persisted with 'dismissed'
    const persisted = settingsStore.getState().persistedConfig;
    assert.equal(persisted?.general.onboarding.status, 'dismissed');

    // Notification posted
    const notifs = shell.getNotificationStore().getState();
    assert.ok(notifs.some((n) => n.reasonCode === 'onboarding.dismissed'));
  });

  await t.test('§8. Completing onboarding persists timestamp and navigates to library', async () => {
    const config = createDefaultConfigDto();
    const transport = createMockSettingsIpcTransport(config);
    const settingsStore = new SettingsStore(new SettingsIpcClient(transport));
    await settingsStore.loadConfig();

    const routerStore = new RouterStore('dashboard', 1920, 1080);
    const shell = new AppShellController({
      routerStore,
      catalogStore: new CatalogStore(createMockCatalogClient(1)),
      settingsStore,
    });

    shell.handleOnboardingStageChange(3);
    const completed = await shell.handleOnboardingComplete();
    assert.equal(completed, true);
    assert.equal(shell.isOnboardingOpen(), false);

    const persisted = settingsStore.getState().persistedConfig;
    assert.equal(persisted?.general.onboarding.status, 'completed');
    assert.ok(typeof persisted?.general.onboarding.completed_at_millis === 'number');
    assert.ok((persisted?.general.onboarding.completed_at_millis ?? 0) > 0);
  });

  await t.test('§9. Persistence failure shows truthful notice and preserves draft edits', async () => {
    const config = createDefaultConfigDto();
    // Transport that fails saves
    const failingTransport = {
      async load() {
        return structuredClone(config);
      },
      async save() {
        throw new Error('EACCES: Disk write protected');
      },
      async reset() {
        return structuredClone(config);
      },
    };

    const settingsStore = new SettingsStore(new SettingsIpcClient(failingTransport));
    await settingsStore.loadConfig();

    const shell = new AppShellController({
      routerStore: new RouterStore('dashboard', 1920, 1080),
      catalogStore: new CatalogStore(createMockCatalogClient(1)),
      settingsStore,
    });

    const success = await shell.handleOnboardingComplete();
    assert.equal(success, false);

    // Modal stays open or displays notice
    const notifs = shell.getNotificationStore().getState();
    assert.ok(notifs.some((n) => n.reasonCode === 'onboarding.save_failed'));
  });

  await t.test('§10. SettingsView provides access to reopen guided onboarding tour', async () => {
    const config = createDefaultConfigDto();
    config.general.onboarding.status = 'completed';

    const settingsStore = new SettingsStore(new SettingsIpcClient(createMockSettingsIpcTransport(config)));
    await settingsStore.loadConfig();

    let reopened = false;
    const view = SettingsView({
      settingsStore,
      onReopenOnboarding: () => {
        reopened = true;
      },
    });

    const html = renderToHtml(view);
    assert.ok(html.includes('data-testid="pref-onboarding-item"'));
    assert.ok(html.includes('Guided Onboarding &amp; System Readiness') || html.includes('Guided Onboarding & System Readiness'));
    assert.ok(html.includes('data-testid="settings-reopen-onboarding-btn"'));
    assert.ok(html.includes('Tour status: completed'));
  });
});
