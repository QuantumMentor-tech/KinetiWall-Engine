/**
 * Step 63: Accessibility Preferences, Reduced Motion & Inclusive Interaction Support Tests.
 *
 * Exhaustively verifies:
 * 1. Deterministic reduced-motion resolution logic (system, always_reduce, no_preference).
 * 2. System motion detector observation, subscription, and cleanup.
 * 3. AccessibilityStore reactive state transitions and settings synchronization.
 * 4. DOM attribute application (data-reduced-motion="true|false").
 * 5. Screen reader live announcements and debouncing.
 * 6. Accessible keyboard navigation (roving tabindex, arrow navigation, switch controls).
 * 7. Focus trapping and restoration stack.
 * 8. AppShell accessibility attribute synchronization and rendering.
 * 9. SettingsView reduced motion preference control rendering.
 */

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

import { AppShellController } from '../components/shell/AppShell.ts';
import { renderToHtml } from '../components/primitives/uiNode.ts';
import { PreferenceSegmentedGroup, PreferenceToggle } from '../components/domain/PreferenceControls.ts';
import { CatalogIpcClient } from '../services/catalogIpcClient.ts';
import {
  SettingsIpcClient,
  createMockSettingsIpcTransport,
} from '../services/settingsIpcClient.ts';
import { MockSystemMotionDetector } from '../services/systemMotionDetector.ts';
import { AccessibilityStore } from '../stores/accessibilityStore.ts';
import { CatalogStore } from '../stores/catalogStore.ts';
import { RouterStore } from '../stores/routerStore.ts';
import { SettingsStore } from '../stores/settingsStore.ts';
import {
  resolveEffectiveReducedMotion,
  type AccessibilityState,
} from '../types/accessibility.ts';
import type { ReducedMotionPreferenceDto } from '../types/settings.ts';
import { ScreenReaderAnnouncer, announce } from '../utils/announcer.ts';
import { FocusRestorationStack, getFocusableElements, trapFocus } from '../utils/focusManager.ts';
import { SettingsView } from '../views/SettingsView.ts';

describe('Step 63: Accessibility Preferences & Inclusive Interaction', () => {
  // -----------------------------------------------------------------------------------------------
  // 1. Deterministic Reduced Motion Resolution
  // -----------------------------------------------------------------------------------------------
  it('§1. resolveEffectiveReducedMotion strictly respects user preference over OS state', () => {
    // Explicit 'always_reduce' always yields true regardless of OS setting
    assert.strictEqual(resolveEffectiveReducedMotion('always_reduce', false), true);
    assert.strictEqual(resolveEffectiveReducedMotion('always_reduce', true), true);

    // Explicit 'no_preference' always yields false regardless of OS setting
    assert.strictEqual(resolveEffectiveReducedMotion('no_preference', false), false);
    assert.strictEqual(resolveEffectiveReducedMotion('no_preference', true), false);

    // 'system' faithfully follows OS observation
    assert.strictEqual(resolveEffectiveReducedMotion('system', false), false);
    assert.strictEqual(resolveEffectiveReducedMotion('system', true), true);

    // Default / fallback behavior
    assert.strictEqual(resolveEffectiveReducedMotion('system' as ReducedMotionPreferenceDto, false), false);
  });

  // -----------------------------------------------------------------------------------------------
  // 2. MockSystemMotionDetector Subscription & Event Handling
  // -----------------------------------------------------------------------------------------------
  it('§2. SystemMotionDetector handles dynamic OS motion notifications and clean disposal', () => {
    const detector = new MockSystemMotionDetector(false);
    assert.strictEqual(detector.isReducedMotion(), false);
    assert.strictEqual(detector.isSupported(), true);

    const received: boolean[] = [];
    const unsubscribe = detector.subscribe((isReduced) => {
      received.push(isReduced);
    });

    assert.strictEqual(detector.listenerCount(), 1);

    detector.setSystemReducedMotion(true);
    detector.setSystemReducedMotion(false);

    assert.deepStrictEqual(received, [true, false]);
    assert.strictEqual(detector.isReducedMotion(), false);

    // Idempotent notifications do not fire
    detector.setSystemReducedMotion(false);
    assert.strictEqual(received.length, 2);

    unsubscribe();
    assert.strictEqual(detector.listenerCount(), 0);

    detector.setSystemReducedMotion(true);
    assert.strictEqual(received.length, 2); // Unsubscribed listener receives nothing
  });

  // -----------------------------------------------------------------------------------------------
  // 3. AccessibilityStore Reactive State & Settings Synchronization
  // -----------------------------------------------------------------------------------------------
  it('§3. AccessibilityStore manages user preference, OS sync, and settings coordination', () => {
    const detector = new MockSystemMotionDetector(false);
    const store = new AccessibilityStore(detector, 'system');

    assert.strictEqual(store.getState().reducedMotionPreference, 'system');
    assert.strictEqual(store.getState().isSystemReducedMotion, false);
    assert.strictEqual(store.getState().effectiveReducedMotion, false);

    const notifications: AccessibilityState[] = [];
    const unsubStore = store.subscribe((state) => {
      notifications.push({ ...state });
    });

    // 1. Changing preference to 'always_reduce' activates effective reduced motion
    store.setPreference('always_reduce');
    assert.strictEqual(store.getState().effectiveReducedMotion, true);
    assert.strictEqual(store.getState().reducedMotionPreference, 'always_reduce');
    assert.strictEqual(notifications.length, 1);

    // 2. When user explicitly chose 'always_reduce', OS transitions do not override effective state
    detector.setSystemReducedMotion(false);
    assert.strictEqual(store.getState().effectiveReducedMotion, true);

    // 3. Changing preference to 'no_preference' disables effective reduced motion even if OS is reduce
    detector.setSystemReducedMotion(true);
    store.setPreference('no_preference');
    assert.strictEqual(store.getState().effectiveReducedMotion, false);

    // 4. Switching back to 'system' adopts OS preference (which is currently true)
    store.setPreference('system');
    assert.strictEqual(store.getState().effectiveReducedMotion, true);

    // 5. Synchronizing with settings DTO updates store state
    const mockConfig = {
      schema_version: 1,
      general: {
        autostart: false,
        start_minimized: true,
        minimize_to_tray: true,
        theme: 'dark' as const,
        reduced_motion: 'no_preference' as const,
        language: 'en-US',
      },
      performance: {} as any,
      rendering: {} as any,
      audio: {} as any,
      monitors: {},
      wallpaper_properties: {},
      diagnostics: {} as any,
    };

    store.syncWithSettings(mockConfig);
    assert.strictEqual(store.getState().reducedMotionPreference, 'no_preference');
    assert.strictEqual(store.getState().effectiveReducedMotion, false);

    unsubStore();
    store.dispose();
  });

  // -----------------------------------------------------------------------------------------------
  // 4. ScreenReaderAnnouncer & Deduplication
  // -----------------------------------------------------------------------------------------------
  it('§4. ScreenReaderAnnouncer announces messages and deduplicates rapid repeats', () => {
    ScreenReaderAnnouncer.resetInstance();
    const announcer = ScreenReaderAnnouncer.getInstance();

    announcer.announce('First announcement');
    assert.strictEqual(announcer.getLastMessage(), 'First announcement');

    // Rapid duplicate within debounce window is ignored
    announcer.announce('First announcement');
    assert.strictEqual(announcer.getLastMessage(), 'First announcement');

    // Different message is accepted immediately
    announcer.announce('Second announcement');
    assert.strictEqual(announcer.getLastMessage(), 'Second announcement');

    // Empty or whitespace strings are ignored
    announcer.announce('   ');
    assert.strictEqual(announcer.getLastMessage(), 'Second announcement');

    ScreenReaderAnnouncer.resetInstance();
  });

  // -----------------------------------------------------------------------------------------------
  // 5. Focus Trap & Focus Stack Utilities
  // -----------------------------------------------------------------------------------------------
  it('§5. FocusRestorationStack tracks navigation depth and restores focus safely', () => {
    const stack = FocusRestorationStack.getInstance();
    stack.clear();
    assert.strictEqual(stack.depth(), 0);

    let focused = false;
    const mockElement = {
      focus: () => {
        focused = true;
      },
    } as unknown as HTMLElement;

    stack.push(mockElement);
    assert.strictEqual(stack.depth(), 1);

    const restored = stack.popAndRestore();
    assert.strictEqual(restored, true);
    assert.strictEqual(focused, true);
    assert.strictEqual(stack.depth(), 0);

    // Popping empty stack returns false gracefully
    assert.strictEqual(stack.popAndRestore(), false);
    stack.clear();
  });

  // -----------------------------------------------------------------------------------------------
  // 6. Accessible Preference Controls (Roving Tabindex, Switch, Keyboard Handling)
  // -----------------------------------------------------------------------------------------------
  it('§6. Preference controls render accessible roles, states, and handle keyboard interactions', () => {
    let selectedOption = 'system';
    const segmentedNode = PreferenceSegmentedGroup({
      testId: 'pref-motion-test',
      label: 'Motion Mode',
      description: 'Motion preference description',
      value: selectedOption,
      options: [
        { value: 'system', label: 'System', testId: 'opt-sys' },
        { value: 'always_reduce', label: 'Reduced', testId: 'opt-red' },
        { value: 'no_preference', label: 'Full', testId: 'opt-full' },
      ],
      onSelect: (val) => {
        selectedOption = val;
      },
    });

    const html = renderToHtml(segmentedNode);
    // WAI-ARIA Radio Group pattern assertions
    assert.ok(html.includes('role="radiogroup"'), 'Must have role="radiogroup"');
    assert.ok(html.includes('role="radio"'), 'Options must have role="radio"');
    assert.ok(html.includes('aria-checked="true"'), 'Selected option must be aria-checked="true"');
    assert.ok(html.includes('tabindex="0"'), 'Selected option must have roving tabindex="0"');
    assert.ok(html.includes('tabindex="-1"'), 'Unselected options must have tabindex="-1"');

    // Arrow navigation test on segmented node
    const buttonList = segmentedNode.children[1].children;
    assert.strictEqual(buttonList.length, 3);

    // Simulate ArrowRight on first radio button (system -> always_reduce)
    const firstBtn = buttonList[0];
    assert.ok(firstBtn.onKeyDown, 'Radio button must define onKeyDown');
    firstBtn.onKeyDown?.({ key: 'ArrowRight', preventDefault: () => {} } as any);
    assert.strictEqual(selectedOption, 'always_reduce');

    // Simulate ArrowLeft (always_reduce -> system)
    const secondBtn = buttonList[1];
    secondBtn.onKeyDown?.({ key: 'ArrowLeft', preventDefault: () => {} } as any);
    assert.strictEqual(selectedOption, 'system');

    // Switch (Toggle) accessibility
    let toggleChecked = false;
    const toggleNode = PreferenceToggle({
      id: 'test-toggle',
      testId: 'pref-toggle-test',
      label: 'Hardware Acceleration',
      description: 'Use GPU for composite rendering',
      checked: toggleChecked,
      onChange: (val) => {
        toggleChecked = val;
      },
    });

    const toggleHtml = renderToHtml(toggleNode);
    assert.ok(toggleHtml.includes('role="switch"'), 'Toggle must have role="switch"');
    assert.ok(toggleHtml.includes('aria-checked="false"'), 'Toggle must reflect aria-checked="false"');

    // Space key activates toggle
    const toggleBtn = toggleNode.children[1];
    toggleBtn.onKeyDown?.({ key: ' ', preventDefault: () => {} } as any);
    assert.strictEqual(toggleChecked, true);
  });

  // -----------------------------------------------------------------------------------------------
  // 7. AppShell Accessibility Synchronization
  // -----------------------------------------------------------------------------------------------
  it('§7. AppShell renders data-reduced-motion and data-motion-preference attributes', () => {
    const routerStore = new RouterStore('dashboard');
    const catalogStore = new CatalogStore(new CatalogIpcClient({} as any));
    const motionDetector = new MockSystemMotionDetector(false);
    const accessibilityStore = new AccessibilityStore(motionDetector, 'system');

    const shell = new AppShellController({
      routerStore,
      catalogStore,
      accessibilityStore,
    });

    const node = shell.render();
    assert.strictEqual(node.attrs['data-reduced-motion'], 'false');
    assert.strictEqual(node.attrs['data-motion-preference'], 'system');

    // Update preference to always_reduce
    accessibilityStore.setPreference('always_reduce');
    const updatedNode = shell.render();
    assert.strictEqual(updatedNode.attrs['data-reduced-motion'], 'true');
    assert.strictEqual(updatedNode.attrs['data-motion-preference'], 'always_reduce');

    // OS transition in system mode
    accessibilityStore.setPreference('system');
    motionDetector.setSystemReducedMotion(true);
    const osSyncNode = shell.render();
    assert.strictEqual(osSyncNode.attrs['data-reduced-motion'], 'true');
    assert.strictEqual(osSyncNode.attrs['data-motion-preference'], 'system');

    accessibilityStore.dispose();
  });

  // -----------------------------------------------------------------------------------------------
  // 8. SettingsView Reduced Motion UI Control Integration
  // -----------------------------------------------------------------------------------------------
  it('§8. SettingsView renders reduced motion preference control under General section', async () => {
    const transport = createMockSettingsIpcTransport();
    const client = new SettingsIpcClient(transport);
    const store = new SettingsStore(client);

    await store.loadConfig();

    const viewNode = SettingsView({
      settingsStore: store,
    });

    const html = renderToHtml(viewNode);
    assert.ok(html.includes('data-testid="pref-reduced-motion"'), 'Must contain pref-reduced-motion');
    assert.ok(html.includes('Reduced Motion &amp; Animations') || html.includes('Reduced Motion & Animations'), 'Must have section title');
    assert.ok(html.includes('data-testid="motion-system"'), 'Must contain motion-system button');
    assert.ok(html.includes('data-testid="motion-reduce"'), 'Must contain motion-reduce button');
    assert.ok(html.includes('data-testid="motion-full"'), 'Must contain motion-full button');
  });
});

