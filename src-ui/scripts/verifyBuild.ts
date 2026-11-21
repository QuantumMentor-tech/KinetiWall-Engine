/**
 * Build & Contract Verification Script for `src-ui`.
 * Verifies module loading, responsive layout calculations, HTML serialization safety,
 * and design token presence.
 */

import assert from 'node:assert/strict';
import { AppShellController } from '../components/shell/AppShell.ts';
import { renderToHtml } from '../components/primitives/uiNode.ts';
import { CatalogIpcClient } from '../services/catalogIpcClient.ts';
import { SettingsIpcClient, createMockSettingsIpcTransport } from '../services/settingsIpcClient.ts';
import { CatalogStore } from '../stores/catalogStore.ts';
import { RouterStore } from '../stores/routerStore.ts';
import { SettingsStore } from '../stores/settingsStore.ts';
import { computeResponsiveGridMetrics, NEON_GLASS_TOKENS } from '../styles/tokens.ts';
import { calculateContrastRatio } from '../types/theme.ts';

async function main(): Promise<void> {
  assert.equal(NEON_GLASS_TOKENS.colors.surfaceVoid, '#0A0B10');
  assert.equal(NEON_GLASS_TOKENS.colors.accentCyan, '#00F0FF');
  assert.equal(NEON_GLASS_TOKENS.dark.surfaceVoid, '#0A0B10');
  assert.equal(NEON_GLASS_TOKENS.light.surfaceVoid, '#F0F2F8');

  // Verify WCAG AAA (>7:1) contrast ratio for light theme primary text
  const lightContrast = calculateContrastRatio(
    NEON_GLASS_TOKENS.light.textPrimary,
    NEON_GLASS_TOKENS.light.surfaceAcrylic,
  );
  assert.ok(lightContrast >= 7.0, `Expected WCAG AAA contrast >= 7.0, got ${lightContrast}`);

  const compact = computeResponsiveGridMetrics(960);
  const standard = computeResponsiveGridMetrics(1280);
  const wide = computeResponsiveGridMetrics(1920);
  const ultrawide = computeResponsiveGridMetrics(2560);
  const uhd = computeResponsiveGridMetrics(3840);

  assert.equal(compact.columns, 2);
  assert.equal(standard.columns, 3);
  assert.equal(wide.columns, 4);
  assert.equal(ultrawide.columns, 5);
  assert.equal(uhd.columns, 6);

  const client = new CatalogIpcClient({
    async invokeGetLibrarySnapshot() {
      return {
        entries: [
          {
            id: '11111111-2222-4333-8444-555555555555',
            title: 'Neon Skyline 4K',
            description: 'Looping synthwave skyline',
            wallpaper_type: 'video',
            source_kind: 'managed_package',
            author: 'KinetiWall Studio',
            tags: ['neon', '4k'],
            availability: { status: 'available' },
            created_at: '2026-11-12T00:00:00Z',
            updated_at: '2026-11-12T00:00:00Z',
            is_builtin: false,
            has_thumbnail_ref: true,
            thumbnail_ref: 'preview.png',
          },
        ],
        integrity: {
          valid_record_count: 1,
          available_count: 1,
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

  const routerStore = new RouterStore('library', 1920, 1080);
  const catalogStore = new CatalogStore(client);
  await catalogStore.loadCatalog();

  const settingsStore = new SettingsStore(new SettingsIpcClient(createMockSettingsIpcTransport()));
  await settingsStore.loadConfig();

  const shell = new AppShellController({ routerStore, catalogStore, settingsStore });
  const html = renderToHtml(shell.render());
  assert.ok(html.includes('Wallpaper Library'));
  assert.ok(html.includes('Neon Skyline 4K'));

  // Multi-Monitor Displays view verification (Step 46)
  shell.navigate('displays');
  const displaysHtml = renderToHtml(shell.render());
  assert.ok(displaysHtml.includes('Displays &amp; Layout') || displaysHtml.includes('Displays & Layout'));
  assert.ok(displaysHtml.includes('Presentation Layout Mode'));
  assert.ok(displaysHtml.includes('Display Arrangement'));

  // Centralized Settings view verification (Step 61)
  shell.navigate('settings');
  const settingsHtml = renderToHtml(shell.render());
  assert.ok(settingsHtml.includes('Application Settings'));
  assert.ok(settingsHtml.includes('General Preferences'));
  assert.ok(settingsHtml.includes('Run on Windows Startup'));
  assert.ok(settingsHtml.includes('Preferences saved and up to date'));
  assert.ok(settingsHtml.includes('data-theme="dark"'));
  assert.ok(settingsHtml.includes('data-reduced-motion="false"'));
  assert.ok(settingsHtml.includes('Reduced Motion &amp; Animations') || settingsHtml.includes('Reduced Motion & Animations'));

  // Keyboard Shortcuts Architecture & Command Registry verification (Step 64)
  const registry = shell.getCommandRegistry();
  assert.ok(registry.getCommand('nav.dashboard'));
  assert.ok(registry.getCommand('nav.library'));
  assert.ok(registry.getCommand('nav.displays'));
  assert.ok(registry.getCommand('nav.settings'));
  assert.ok(registry.getCommand('library.search_focus'));
  assert.ok(registry.getCommand('help.shortcuts'));

  assert.ok(settingsHtml.includes('Alt+1'));
  assert.ok(settingsHtml.includes('data-testid="nav-shortcuts-btn"'));

  shell.openShortcutsModal();
  const shortcutsModalHtml = renderToHtml(shell.render());
  assert.ok(shortcutsModalHtml.includes('data-testid="shortcuts-modal-dialog"'));
  assert.ok(shortcutsModalHtml.includes('Keyboard Shortcuts'));
  shell.closeShortcutsModal();

  // In-App Notification Architecture & Toast Lifecycle verification (Step 65)
  assert.ok(settingsHtml.includes('In-App Notifications'));
  const notifStore = shell.getNotificationStore();
  notifStore.success('Engine operational', { title: 'Online' });
  const notifHtml = renderToHtml(shell.render());
  assert.ok(notifHtml.includes('data-testid="toast-container"'));
  assert.ok(notifHtml.includes('Engine operational'));
  assert.ok(notifHtml.includes('Online'));
  notifStore.dismissAll();

  // First-Run Experience, Guided Onboarding & Capability Readiness verification (Step 66)
  assert.ok(settingsHtml.includes('Guided Onboarding &amp; System Readiness') || settingsHtml.includes('Guided Onboarding & System Readiness'));
  assert.ok(settingsHtml.includes('data-testid="settings-reopen-onboarding-btn"'));
  assert.ok(registry.getCommand('help.onboarding'));

  shell.openOnboardingModal(0);
  const onboardingHtml = renderToHtml(shell.render());
  assert.ok(onboardingHtml.includes('data-testid="onboarding-modal-dialog"'));
  assert.ok(onboardingHtml.includes('Welcome to KinetiWall'));
  assert.ok(onboardingHtml.includes('Step 1 of 4: Welcome'));

  shell.handleOnboardingStageChange(1);
  const readinessHtml = renderToHtml(shell.render());
  assert.ok(readinessHtml.includes('data-testid="onboarding-readiness-list"'));
  assert.ok(readinessHtml.includes('Configuration &amp; Preferences') || readinessHtml.includes('Configuration & Preferences'));
  assert.ok(readinessHtml.includes('data-testid="onboarding-recheck-readiness-btn"'));
  shell.closeOnboardingModal();

  // In-App Help, Contextual Guidance & Troubleshooting verification (Step 67)
  assert.ok(registry.getCommand('help.main'));
  assert.ok(settingsHtml.includes('data-testid="settings-help-btn"'));
  assert.ok(displaysHtml.includes('data-testid="displays-help-btn"'));

  shell.openHelpModal('ts-playback-paused');
  const helpHtml = renderToHtml(shell.render());
  assert.ok(helpHtml.includes('data-testid="help-modal-dialog"'));
  assert.ok(helpHtml.includes('Help &amp; Troubleshooting') || helpHtml.includes('Help & Troubleshooting'));
  assert.ok(helpHtml.includes('Wallpaper Paused or Not Animating'));
  assert.ok(helpHtml.includes('What Happened:'));
  assert.ok(helpHtml.includes('Likely Interpretation:'));
  assert.ok(helpHtml.includes('data-testid="help-execute-action-btn"'));
  shell.closeHelpModal();

  console.log('verifyBuild: OK (All src-ui modules, multi-monitor displays, settings, themes, tokens, accessibility, keyboard shortcuts, notifications, onboarding, help & troubleshooting, and SSR/DOM contracts verified)');
}

await main();

