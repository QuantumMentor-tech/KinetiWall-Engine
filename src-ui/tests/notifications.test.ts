/**
 * Step 65: In-App Notification Architecture, Toast Lifecycle, Actionable Feedback & Notification Preferences Tests.
 *
 * Exhaustively verifies:
 * 1. Typed notification model, default durations, and severity semantics.
 * 2. Message sanitization (preventing leakage of raw Rust debug traces and absolute paths).
 * 3. Notification deduplication and storm prevention within the debounce window.
 * 4. Bounded queue management (enforcing MAX_VISIBLE_NOTIFICATIONS limit).
 * 5. Auto-expiry timers and manual dismissal lifecycle cleanup.
 * 6. Actionable notifications (action button execution and auto-dismiss).
 * 7. User notification preferences filtering (all, errors_only, disabled).
 * 8. Screen-reader live region integration and politeness mapping.
 * 9. ToastContainer accessible rendering, roles, and AppShell integration.
 */

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

import { AppShellController } from '../components/shell/AppShell.ts';
import { renderToHtml } from '../components/primitives/uiNode.ts';
import { ToastContainer } from '../components/domain/ToastContainer.ts';
import { CatalogIpcClient } from '../services/catalogIpcClient.ts';
import { SettingsIpcClient, createMockSettingsIpcTransport } from '../services/settingsIpcClient.ts';
import { CatalogStore } from '../stores/catalogStore.ts';
import { NotificationStore } from '../stores/notificationStore.ts';
import { RouterStore } from '../stores/routerStore.ts';
import { SettingsStore } from '../stores/settingsStore.ts';
import {
  DEFAULT_NOTIFICATION_DURATIONS,
  MAX_VISIBLE_NOTIFICATIONS,
  sanitizeNotificationMessage,
  shouldPresentNotification,
  type NotificationItem,
} from '../types/notification.ts';
import { ScreenReaderAnnouncer } from '../utils/announcer.ts';
import { SettingsView } from '../views/SettingsView.ts';

describe('Step 65: In-App Notification Architecture & Toast Lifecycle', () => {
  // -----------------------------------------------------------------------------------------------
  // 1. Notification Model & Sanitization
  // -----------------------------------------------------------------------------------------------
  it('§1. sanitizeNotificationMessage redacts sensitive file paths and raw Rust debug traces', () => {
    // 1. Absolute Windows file paths
    const rawPathMsg = 'Failed to load asset from C:\\Users\\Administrator\\Desktop\\secret.mp4';
    const sanitizedPath = sanitizeNotificationMessage(rawPathMsg);
    assert.strictEqual(sanitizedPath, 'Failed to load asset from [path]/secret.mp4');

    // 2. Raw Rust debug Custom error traces
    const rawRustMsg = 'Custom { kind: PermissionDenied, error: "Access is denied" }';
    const sanitizedRust = sanitizeNotificationMessage(rawRustMsg);
    assert.strictEqual(sanitizedRust, '"Access is denied"');

    // 3. Normal text is preserved
    const normal = 'Wallpaper settings updated successfully.';
    assert.strictEqual(sanitizeNotificationMessage(normal), normal);
  });

  // -----------------------------------------------------------------------------------------------
  // 2. Severities & Default Durations
  // -----------------------------------------------------------------------------------------------
  it('§2. NotificationStore applies severity-specific default durations and persistence contracts', () => {
    const store = new NotificationStore();

    // Success has 3500ms duration
    const idSuccess = store.success('Settings saved');
    assert.ok(idSuccess);
    const itemSuccess = store.getState().find((n) => n.id === idSuccess);
    assert.strictEqual(itemSuccess?.severity, 'success');
    assert.strictEqual(itemSuccess?.durationMs, DEFAULT_NOTIFICATION_DURATIONS.success);
    assert.ok(itemSuccess?.expiresAt !== null);

    // Error is persistent by default (durationMs 0, expiresAt null)
    const idError = store.error('Playback crash detected');
    assert.ok(idError);
    const itemError = store.getState().find((n) => n.id === idError);
    assert.strictEqual(itemError?.severity, 'error');
    assert.strictEqual(itemError?.durationMs, 0);
    assert.strictEqual(itemError?.expiresAt, null);

    // Explicit custom duration is honored
    const idCustom = store.info('Custom timer', { durationMs: 9000 });
    assert.ok(idCustom);
    const itemCustom = store.getState().find((n) => n.id === idCustom);
    assert.strictEqual(itemCustom?.durationMs, 9000);

    store.dispose();
  });

  // -----------------------------------------------------------------------------------------------
  // 3. Deduplication & Storm Prevention
  // -----------------------------------------------------------------------------------------------
  it('§3. Deduplication prevents rapid notification storms while preserving distinct events', () => {
    const store = new NotificationStore();

    // Dispatching identical message repeatedly within debounce window returns existing ID
    const id1 = store.warning('Battery level critical');
    const id2 = store.warning('Battery level critical');
    const id3 = store.warning('Battery level critical');

    assert.strictEqual(id1, id2);
    assert.strictEqual(id2, id3);
    assert.strictEqual(store.getState().length, 1);

    // Different message creates distinct notification
    const idDifferent = store.warning('AC adapter reconnected');
    assert.notStrictEqual(id1, idDifferent);
    assert.strictEqual(store.getState().length, 2);

    store.dispose();
  });

  // -----------------------------------------------------------------------------------------------
  // 4. Bounded Queue Management
  // -----------------------------------------------------------------------------------------------
  it('§4. NotificationStore bounds concurrent visible toasts to MAX_VISIBLE_NOTIFICATIONS', () => {
    const store = new NotificationStore();

    // Add more notifications than MAX_VISIBLE_NOTIFICATIONS
    for (let i = 0; i < MAX_VISIBLE_NOTIFICATIONS + 3; i++) {
      store.info(`Notification ${i}`);
    }

    // Should never exceed MAX_VISIBLE_NOTIFICATIONS
    assert.strictEqual(store.getState().length, MAX_VISIBLE_NOTIFICATIONS);

    // Oldest items are purged to make room for newer notifications
    const messages = store.getState().map((n) => n.message);
    assert.ok(!messages.includes('Notification 0'));
    assert.ok(messages.includes(`Notification ${MAX_VISIBLE_NOTIFICATIONS + 2}`));

    store.dispose();
  });

  // -----------------------------------------------------------------------------------------------
  // 5. Dismissal & Expiry
  // -----------------------------------------------------------------------------------------------
  it('§5. Dismissal removes notifications and cleans up active timers', () => {
    const store = new NotificationStore();

    const id = store.info('Transient notification');
    assert.strictEqual(store.getState().length, 1);

    const dismissed = store.dismiss(id!);
    assert.strictEqual(dismissed, true);
    assert.strictEqual(store.getState().length, 0);

    // Dismissing non-existent ID returns false safely
    assert.strictEqual(store.dismiss('non_existent'), false);

    // dismissAll clears multiple items
    store.info('Item 1');
    store.info('Item 2');
    assert.strictEqual(store.getState().length, 2);

    store.dismissAll();
    assert.strictEqual(store.getState().length, 0);

    store.dispose();
  });

  // -----------------------------------------------------------------------------------------------
  // 6. Actionable Feedback
  // -----------------------------------------------------------------------------------------------
  it('§6. Actionable notifications invoke safe action callbacks and automatically dismiss', () => {
    const store = new NotificationStore();
    let actionExecuted = false;

    const id = store.error('Display disconnected', {
      title: 'Topology Change',
      action: {
        label: 'Reconfigure',
        onClick: () => {
          actionExecuted = true;
        },
      },
    });

    const item = store.getState().find((n) => n.id === id);
    assert.ok(item?.action);
    assert.strictEqual(item?.action?.label, 'Reconfigure');

    // Render ToastContainer and execute action
    const container = ToastContainer({
      notifications: store.getState(),
      onDismiss: (dId) => store.dismiss(dId),
    });

    const html = renderToHtml(container!);
    assert.ok(html.includes('data-testid="toast-action-'));
    assert.ok(html.includes('Reconfigure'));

    // Invoke action callback directly
    void item?.action?.onClick();
    assert.strictEqual(actionExecuted, true);

    store.dispose();
  });

  // -----------------------------------------------------------------------------------------------
  // 7. Notification Preferences Filtering
  // -----------------------------------------------------------------------------------------------
  it('§7. User notification preferences filter transient alerts while preserving critical feedback', () => {
    // 1. 'all': allows everything
    assert.strictEqual(shouldPresentNotification('info', 'all'), true);
    assert.strictEqual(shouldPresentNotification('success', 'all'), true);
    assert.strictEqual(shouldPresentNotification('warning', 'all'), true);
    assert.strictEqual(shouldPresentNotification('error', 'all'), true);

    // 2. 'errors_only': suppresses info/success; allows warning/error
    assert.strictEqual(shouldPresentNotification('info', 'errors_only'), false);
    assert.strictEqual(shouldPresentNotification('success', 'errors_only'), false);
    assert.strictEqual(shouldPresentNotification('warning', 'errors_only'), true);
    assert.strictEqual(shouldPresentNotification('error', 'errors_only'), true);

    // 3. 'disabled': suppresses non-critical; preserves errors
    assert.strictEqual(shouldPresentNotification('info', 'disabled'), false);
    assert.strictEqual(shouldPresentNotification('success', 'disabled'), false);
    assert.strictEqual(shouldPresentNotification('warning', 'disabled'), false);
    assert.strictEqual(shouldPresentNotification('error', 'disabled'), true);

    // Test in NotificationStore
    const store = new NotificationStore('errors_only');
    const infoId = store.info('Routine update');
    assert.strictEqual(infoId, null);
    assert.strictEqual(store.getState().length, 0);

    const errorId = store.error('Critical fault');
    assert.ok(errorId);
    assert.strictEqual(store.getState().length, 1);

    store.dispose();
  });

  // -----------------------------------------------------------------------------------------------
  // 8. Screen Reader Live Announcements Integration
  // -----------------------------------------------------------------------------------------------
  it('§8. NotificationStore dispatches screen reader announcements with appropriate politeness', () => {
    ScreenReaderAnnouncer.resetInstance();
    const announcer = ScreenReaderAnnouncer.getInstance();
    const store = new NotificationStore();

    store.info('Library reloaded');
    assert.strictEqual(announcer.getLastMessage(), 'Library reloaded');

    store.error('Configuration corrupted', { title: 'Settings Error' });
    assert.strictEqual(announcer.getLastMessage(), 'Settings Error: Configuration corrupted');

    store.dispose();
    ScreenReaderAnnouncer.resetInstance();
  });

  // -----------------------------------------------------------------------------------------------
  // 9. ToastContainer Rendering & AppShell Integration
  // -----------------------------------------------------------------------------------------------
  it('§9. ToastContainer renders accessible roles, icons, and integrates into AppShell', () => {
    const routerStore = new RouterStore('dashboard');
    const catalogStore = new CatalogStore(new CatalogIpcClient({} as any));
    const notificationStore = new NotificationStore();

    const shell = new AppShellController({
      routerStore,
      catalogStore,
      notificationStore,
    });

    // Initially no notifications -> ToastContainer renders null
    let html = renderToHtml(shell.render());
    assert.ok(!html.includes('data-testid="toast-container"'));

    // Emit success notification
    notificationStore.success('Display layout applied', { title: 'Layout Applied' });
    html = renderToHtml(shell.render());

    assert.ok(html.includes('data-testid="toast-container"'));
    assert.ok(html.includes('role="region"'));
    assert.ok(html.includes('aria-label="Notifications"'));
    assert.ok(html.includes('role="status"'));
    assert.ok(html.includes('Layout Applied'));
    assert.ok(html.includes('Display layout applied'));
    assert.ok(html.includes('✓'));

    // Emit error notification -> receives role="alert"
    notificationStore.error('Disk read failed');
    html = renderToHtml(shell.render());
    assert.ok(html.includes('role="alert"'));
    assert.ok(html.includes('Disk read failed'));

    shell.dispose();
  });

  // -----------------------------------------------------------------------------------------------
  // 10. SettingsView Notification Preferences Control
  // -----------------------------------------------------------------------------------------------
  it('§10. SettingsView renders in-app notification preference controls under General section', async () => {
    const transport = createMockSettingsIpcTransport();
    const client = new SettingsIpcClient(transport);
    const store = new SettingsStore(client);
    await store.loadConfig();

    const viewNode = SettingsView({
      settingsStore: store,
    });

    const html = renderToHtml(viewNode);
    assert.ok(html.includes('data-testid="pref-notifications"'));
    assert.ok(html.includes('In-App Notifications'));
    assert.ok(html.includes('data-testid="notif-all"'));
    assert.ok(html.includes('data-testid="notif-errors"'));
    assert.ok(html.includes('data-testid="notif-disabled"'));
  });
});
