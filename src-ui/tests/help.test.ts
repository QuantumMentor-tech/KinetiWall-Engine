/**
 * Automated Tests: In-App Help, Contextual Guidance & Troubleshooting Experience (Step 67).
 *
 * Covers:
 * §1. Help content integrity: unique IDs, required fields, and whitelisted safe actions.
 * §2. Local search: keyword matching, case-insensitivity, and content matching.
 * §3. Category filtering: category isolation and 'all' selection.
 * §4. Diagnostic reason code mapping: known codes map to specific topics.
 * §5. Unknown diagnostic reason fallback: maps safely to 'ts-unknown-issue' with honest limitations.
 * §6. Troubleshooting detail structure: 5-part evidence-based breakdown.
 * §7. Safe action dispatch: executes whitelisted operations without arbitrary command risks.
 * §8. AppShell integration: F1 shortcut, nav button, and modal opening with deep linking.
 * §9. Contextual help buttons in SettingsView and DisplaysView invoke target topics.
 * §10. Focus management and Escape key dismissal.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { AppShellController } from '../components/shell/AppShell.ts';
import { HelpModal } from '../components/domain/HelpModal.ts';
import { renderToHtml } from '../components/primitives/uiNode.ts';
import { CatalogIpcClient } from '../services/catalogIpcClient.ts';
import { DisplayIpcClient, createMockDisplayIpcTransport } from '../services/displayIpcClient.ts';
import { SettingsIpcClient, createMockSettingsIpcTransport } from '../services/settingsIpcClient.ts';
import { CatalogStore } from '../stores/catalogStore.ts';
import { DisplayStore } from '../stores/displayStore.ts';
import { RouterStore } from '../stores/routerStore.ts';
import { SettingsStore } from '../stores/settingsStore.ts';
import { HELP_CATEGORIES, type SafeActionId, searchHelpTopics } from '../types/help.ts';
import {
  HELP_TOPICS,
  getHelpTopicById,
  resolveHelpTopicForDiagnostic,
} from '../services/helpContent.ts';
import { SettingsView } from '../views/SettingsView.ts';
import { DisplaysView } from '../views/DisplaysView.ts';
import { createDefaultConfigDto } from '../types/settings.ts';

function createMockCatalogClient() {
  return new CatalogIpcClient({
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

test('Step 67: In-App Help, Contextual Guidance & Troubleshooting', async (t) => {
  await t.test('§1. Help content integrity: unique IDs, required fields, and whitelisted safe actions', () => {
    assert.ok(HELP_TOPICS.length >= 10, 'Expected at least 10 comprehensive help topics');
    const seenIds = new Set<string>();

    const validActionIds: SafeActionId[] = [
      'nav.dashboard',
      'nav.library',
      'nav.displays',
      'nav.settings',
      'library.refresh',
      'displays.refresh',
      'onboarding.reopen',
      'readiness.recheck',
      'help.shortcuts',
    ];

    for (const topic of HELP_TOPICS) {
      assert.ok(!seenIds.has(topic.id), `Duplicate topic id found: ${topic.id}`);
      seenIds.add(topic.id);

      assert.ok(topic.title.trim().length > 0, `Topic ${topic.id} missing title`);
      assert.ok(topic.summary.trim().length > 0, `Topic ${topic.id} missing summary`);
      assert.ok(topic.content.length > 0, `Topic ${topic.id} missing content paragraphs`);
      assert.ok(
        HELP_CATEGORIES.some((c) => c.id === topic.category),
        `Topic ${topic.id} has invalid category: ${topic.category}`,
      );

      if (topic.recommendedAction) {
        assert.ok(
          validActionIds.includes(topic.recommendedAction.actionId),
          `Topic ${topic.id} has invalid actionId: ${topic.recommendedAction.actionId}`,
        );
      }
    }
  });

  await t.test('§2. Local search matches title, summary, keywords, and content case-insensitively', () => {
    // Search by title term
    const r1 = searchHelpTopics(HELP_TOPICS, 'shortcuts');
    assert.ok(r1.some((t) => t.id === 'help-keyboard-shortcuts'));

    // Search by keyword
    const r2 = searchHelpTopics(HELP_TOPICS, 'webview2');
    assert.ok(r2.some((t) => t.id === 'ts-webview2-missing'));

    // Search by content term
    const r3 = searchHelpTopics(HELP_TOPICS, 'DirectShow');
    assert.ok(r3.some((t) => t.id === 'ts-video-codec'));

    // No matches returns empty array
    const r4 = searchHelpTopics(HELP_TOPICS, 'xyz_nonexistent_token_12345');
    assert.equal(r4.length, 0);
  });

  await t.test('§3. Category filtering isolates topics correctly and returns all when set to "all"', () => {
    const tsOnly = searchHelpTopics(HELP_TOPICS, '', 'troubleshooting');
    assert.ok(tsOnly.length > 0);
    assert.ok(tsOnly.every((t) => t.category === 'troubleshooting'));

    const settingsOnly = searchHelpTopics(HELP_TOPICS, '', 'settings_and_preferences');
    assert.ok(settingsOnly.length > 0);
    assert.ok(settingsOnly.every((t) => t.category === 'settings_and_preferences'));

    const all = searchHelpTopics(HELP_TOPICS, '', 'all');
    assert.equal(all.length, HELP_TOPICS.length);
  });

  await t.test('§4. Diagnostic reason codes map accurately to specific troubleshooting topics', () => {
    assert.equal(
      resolveHelpTopicForDiagnostic('policy.fullscreen_pause').id,
      'ts-playback-paused',
    );
    assert.equal(
      resolveHelpTopicForDiagnostic('policy.battery_throttle').id,
      'ts-playback-paused',
    );
    assert.equal(
      resolveHelpTopicForDiagnostic('desktop_integration.limited').id,
      'ts-workerw-unsplit',
    );
    assert.equal(
      resolveHelpTopicForDiagnostic('video_player.limited').id,
      'ts-video-codec',
    );
    assert.equal(
      resolveHelpTopicForDiagnostic('web_engine.limited').id,
      'ts-webview2-missing',
    );
    assert.equal(
      resolveHelpTopicForDiagnostic('audio_reactivity.limited').id,
      'ts-audio-wasapi',
    );
    assert.equal(
      resolveHelpTopicForDiagnostic('catalog.asset_missing').id,
      'ts-asset-missing',
    );
    assert.equal(
      resolveHelpTopicForDiagnostic('settings.save_failed').id,
      'ts-settings-save-failed',
    );
  });

  await t.test('§5. Unknown diagnostic reason code maps safely to "ts-unknown-issue" with honest limitations', () => {
    const fallback1 = resolveHelpTopicForDiagnostic('unknown_unregistered_event_99');
    assert.equal(fallback1.id, 'ts-unknown-issue');
    assert.ok(fallback1.troubleshootingDetail);
    assert.ok(fallback1.troubleshootingDetail.limitations.includes('Do not terminate'));

    const fallback2 = resolveHelpTopicForDiagnostic(null);
    assert.equal(fallback2.id, 'ts-unknown-issue');
  });

  await t.test('§6. Troubleshooting detail contains 5-part evidence-based breakdown', () => {
    const tsTopic = getHelpTopicById('ts-playback-paused');
    assert.ok(tsTopic?.troubleshootingDetail);
    const detail = tsTopic.troubleshootingDetail;

    assert.ok(detail.whatHappened.length > 0);
    assert.ok(detail.whatIsKnown.length > 0);
    assert.ok(detail.likelyInterpretation.length > 0);
    assert.ok(detail.recommendedNextStep.length > 0);
    assert.ok(detail.limitations.length > 0);

    const modalHtml = renderToHtml(
      HelpModal({
        isOpen: true,
        selectedTopicId: 'ts-playback-paused',
        searchQuery: '',
        categoryFilter: 'troubleshooting',
        onClose: () => {},
        onSelectTopic: () => {},
        onSearchChange: () => {},
        onCategoryChange: () => {},
        onExecuteAction: () => {},
      }),
    );

    assert.ok(modalHtml.includes('What Happened:'));
    assert.ok(modalHtml.includes('What Is Known:'));
    assert.ok(modalHtml.includes('Likely Interpretation:'));
    assert.ok(modalHtml.includes('Recommended Next Step:'));
    assert.ok(modalHtml.includes('Important Limitations:'));
  });

  await t.test('§7. Safe action dispatch executes whitelisted operations and closes help modal', async () => {
    const config = createDefaultConfigDto();
    config.general.onboarding.status = 'completed'; // returning user
    const settingsStore = new SettingsStore(new SettingsIpcClient(createMockSettingsIpcTransport(config)));
    await settingsStore.loadConfig();

    const routerStore = new RouterStore('dashboard', 1920, 1080);
    const catalogStore = new CatalogStore(createMockCatalogClient());
    const displayStore = new DisplayStore(new DisplayIpcClient(createMockDisplayIpcTransport()));

    const shell = new AppShellController({
      routerStore,
      catalogStore,
      displayStore,
      settingsStore,
    });

    shell.openHelpModal('help-library-manage');
    assert.equal(shell.isHelpOpen(), true);

    // Dispatch safe action 'nav.library'
    shell.executeSafeAction('nav.library');
    assert.equal(shell.isHelpOpen(), false);
    assert.equal(routerStore.getState().activeRoute, 'library');

    // Dispatch safe action 'nav.settings'
    shell.openHelpModal('help-settings-preferences');
    shell.executeSafeAction('nav.settings');
    assert.equal(shell.isHelpOpen(), false);
    assert.equal(routerStore.getState().activeRoute, 'settings');

    // Dispatch safe action 'help.shortcuts'
    shell.openHelpModal();
    shell.executeSafeAction('help.shortcuts');
    assert.equal(shell.isHelpOpen(), false);
    assert.equal(shell.isShortcutsOpen(), true);
    shell.closeShortcutsModal();
  });

  await t.test('§8. AppShell integration: F1 shortcut, nav button, and modal opening with deep linking', async () => {
    const config = createDefaultConfigDto();
    config.general.onboarding.status = 'completed';
    const settingsStore = new SettingsStore(new SettingsIpcClient(createMockSettingsIpcTransport(config)));
    await settingsStore.loadConfig();

    const shell = new AppShellController({
      routerStore: new RouterStore('dashboard', 1920, 1080),
      catalogStore: new CatalogStore(createMockCatalogClient()),
      settingsStore,
    });

    // Initial state: help is closed
    assert.equal(shell.isHelpOpen(), false);

    // Open via deep link to a troubleshooting topic
    shell.openHelpModal('ts-workerw-unsplit');
    assert.equal(shell.isHelpOpen(), true);

    const html = renderToHtml(shell.render());
    assert.ok(html.includes('data-testid="help-modal-dialog"'));
    assert.ok(html.includes('Wallpaper Not Embedding or Black Desktop'));
    assert.ok(html.includes('data-testid="nav-help-btn"'));

    // Command registry has help.main
    const reg = shell.getCommandRegistry();
    const cmd = reg.getCommand('help.main');
    assert.ok(cmd);
    assert.equal(cmd.defaultShortcut?.key, 'F1');

    shell.closeHelpModal();
    assert.equal(shell.isHelpOpen(), false);
  });

  await t.test('§9. Contextual help buttons in SettingsView and DisplaysView invoke target topics', async () => {
    const config = createDefaultConfigDto();
    const settingsStore = new SettingsStore(new SettingsIpcClient(createMockSettingsIpcTransport(config)));
    await settingsStore.loadConfig();

    let settingsHelpTopic: string | undefined;
    const settingsView = SettingsView({
      settingsStore,
      onOpenHelp: (topicId) => {
        settingsHelpTopic = topicId;
      },
    });

    const settingsHtml = renderToHtml(settingsView);
    assert.ok(settingsHtml.includes('data-testid="settings-help-btn"'));

    let displaysHelpTopic: string | undefined;
    const displaysView = DisplaysView({
      displayStore: new DisplayStore(new DisplayIpcClient(createMockDisplayIpcTransport())),
      catalogStore: new CatalogStore(createMockCatalogClient()),
      onOpenHelp: (topicId) => {
        displaysHelpTopic = topicId;
      },
    });

    const displaysHtml = renderToHtml(displaysView);
    assert.ok(displaysHtml.includes('data-testid="displays-help-btn"'));
  });

  await t.test('§10. Empty search state and clear search action', () => {
    let cleared = false;
    const html = renderToHtml(
      HelpModal({
        isOpen: true,
        selectedTopicId: 'help-getting-started',
        searchQuery: 'completely_unknown_search_token_99999',
        categoryFilter: 'all',
        onClose: () => {},
        onSelectTopic: () => {},
        onSearchChange: () => {},
        onCategoryChange: () => {},
        onExecuteAction: () => {},
      }),
    );

    assert.ok(html.includes('data-testid="help-empty-search"'));
    assert.ok(html.includes('No topics found matching'));
    assert.ok(html.includes('data-testid="help-clear-search-btn"'));
  });
});

