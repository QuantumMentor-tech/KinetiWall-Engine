/**
 * Step 64: Keyboard Shortcut Architecture, Command Registry & Conflict-Safe Shortcut Handling Tests.
 *
 * Exhaustively verifies:
 * 1. Shortcut key combo normalization and display formatting.
 * 2. Exact modifier and key event matching (event.key, event.code, case-insensitivity, aliases).
 * 3. Command registration, invariant validation, and unregistration.
 * 4. Conflict detection in overlapping vs distinct scopes.
 * 5. Action routing and dispatch to established application functions.
 * 6. Protection of focused text inputs and editable controls.
 * 7. Modal dialog suppression of global shortcuts.
 * 8. Key repeat suppression for high-impact actions.
 * 9. DOM event listener attachment, dispatching, and clean lifecycle detachment.
 * 10. AppShell default command registry integration, navigation hints, and shortcuts modal.
 */

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

import { AppShellController } from '../components/shell/AppShell.ts';
import { renderToHtml } from '../components/primitives/uiNode.ts';
import { CommandRegistry } from '../services/commandRegistry.ts';
import { CatalogIpcClient } from '../services/catalogIpcClient.ts';
import { CatalogStore } from '../stores/catalogStore.ts';
import { RouterStore } from '../stores/routerStore.ts';
import {
  formatShortcutKeyCombo,
  isInputElement,
  matchesKeyEvent,
  normalizeShortcutKeyCombo,
  type ApplicationCommand,
  type ShortcutKeyCombo,
} from '../types/shortcuts.ts';
import { KeyboardShortcutsModal } from '../components/domain/KeyboardShortcutsModal.ts';

describe('Step 64: Keyboard Shortcut Architecture & Command Registry', () => {
  // -----------------------------------------------------------------------------------------------
  // 1. Normalization & Display Formatting
  // -----------------------------------------------------------------------------------------------
  it('§1. normalizeShortcutKeyCombo and formatShortcutKeyCombo produce canonical representations', () => {
    const combo1: ShortcutKeyCombo = { key: '1', code: 'Digit1', alt: true };
    assert.strictEqual(normalizeShortcutKeyCombo(combo1), 'ALT+1');
    assert.strictEqual(formatShortcutKeyCombo(combo1), 'Alt+1');

    const combo2: ShortcutKeyCombo = { key: 'f', code: 'KeyF', ctrl: true };
    assert.strictEqual(normalizeShortcutKeyCombo(combo2), 'CTRL+F');
    assert.strictEqual(formatShortcutKeyCombo(combo2), 'Ctrl+F');

    const combo3: ShortcutKeyCombo = { key: 'r', ctrl: true, shift: true };
    assert.strictEqual(normalizeShortcutKeyCombo(combo3), 'CTRL+SHIFT+R');
    assert.strictEqual(formatShortcutKeyCombo(combo3), 'Ctrl+Shift+R');

    const combo4: ShortcutKeyCombo = { key: 'Escape' };
    assert.strictEqual(normalizeShortcutKeyCombo(combo4), 'ESCAPE');
    assert.strictEqual(formatShortcutKeyCombo(combo4), 'Esc');

    const combo5: ShortcutKeyCombo = { key: ' ' };
    assert.strictEqual(normalizeShortcutKeyCombo(combo5), 'SPACE');
    assert.strictEqual(formatShortcutKeyCombo(combo5), ' ');
  });

  // -----------------------------------------------------------------------------------------------
  // 2. Keyboard Event Matching
  // -----------------------------------------------------------------------------------------------
  it('§2. matchesKeyEvent verifies exact modifier combinations and key identity', () => {
    const combo: ShortcutKeyCombo = { key: 's', ctrl: true };

    // Exact match
    const validEvent = {
      key: 's',
      ctrlKey: true,
      altKey: false,
      shiftKey: false,
      metaKey: false,
    } as unknown as KeyboardEvent;
    assert.strictEqual(matchesKeyEvent(combo, validEvent), true);

    // Case-insensitivity (e.g. user holds Shift or CapsLock)
    const upperEvent = {
      key: 'S',
      ctrlKey: true,
      altKey: false,
      shiftKey: false,
      metaKey: false,
    } as unknown as KeyboardEvent;
    assert.strictEqual(matchesKeyEvent(combo, upperEvent), true);

    // Modifier mismatch (Alt pressed as well)
    const extraModEvent = {
      key: 's',
      ctrlKey: true,
      altKey: true,
      shiftKey: false,
      metaKey: false,
    } as unknown as KeyboardEvent;
    assert.strictEqual(matchesKeyEvent(combo, extraModEvent), false);

    // Key mismatch
    const wrongKeyEvent = {
      key: 'a',
      ctrlKey: true,
      altKey: false,
      shiftKey: false,
      metaKey: false,
    } as unknown as KeyboardEvent;
    assert.strictEqual(matchesKeyEvent(combo, wrongKeyEvent), false);

    // Code fallback match (Digit1 vs 1)
    const digitCombo: ShortcutKeyCombo = { key: '1', code: 'Digit1', alt: true };
    const codeEvent = {
      key: '!', // on some international layouts Shift+1 yields !
      code: 'Digit1',
      ctrlKey: false,
      altKey: true,
      shiftKey: false,
      metaKey: false,
    } as unknown as KeyboardEvent;
    assert.strictEqual(matchesKeyEvent(digitCombo, codeEvent), true);
  });

  // -----------------------------------------------------------------------------------------------
  // 3. Command Registration, Validation & Conflict Detection
  // -----------------------------------------------------------------------------------------------
  it('§3. CommandRegistry registers valid commands, rejects malformed definitions, and detects conflicts', () => {
    const registry = new CommandRegistry();

    // 1. Valid registration
    let actionFired = false;
    const unregister = registry.registerCommand({
      id: 'test.cmd1',
      label: 'Test Command 1',
      description: 'First test command',
      category: 'general',
      defaultShortcut: { key: 'k', ctrl: true },
      action: () => {
        actionFired = true;
      },
    });

    assert.ok(registry.getCommand('test.cmd1'));
    assert.strictEqual(registry.getAllCommands().length, 1);

    // 2. Rejection of invalid command (empty id)
    assert.throws(
      () =>
        registry.registerCommand({
          id: '',
          label: 'Invalid',
          description: '',
          category: 'general',
          defaultShortcut: { key: 'x' },
          action: () => {},
        }),
      /Invalid command definition/,
    );

    // 3. Collision detection: registering another command with the same global shortcut throws
    assert.throws(
      () =>
        registry.registerCommand({
          id: 'test.cmd2_conflict',
          label: 'Test Command 2 Conflict',
          description: 'Colliding command',
          category: 'general',
          defaultShortcut: { key: 'k', ctrl: true },
          action: () => {},
        }),
      /Shortcut collision/,
    );

    // 4. Same shortcut in a non-overlapping view scope is permitted
    const unregLibrary = registry.registerCommand({
      id: 'test.library_scoped',
      label: 'Library Scoped',
      description: 'Scoped to library',
      category: 'library',
      scope: 'library',
      defaultShortcut: { key: 'l', ctrl: true },
      action: () => {},
    });

    const unregSettings = registry.registerCommand({
      id: 'test.settings_scoped',
      label: 'Settings Scoped',
      description: 'Scoped to settings',
      category: 'settings',
      scope: 'settings',
      defaultShortcut: { key: 'l', ctrl: true }, // Same combo, distinct scopes
      action: () => {},
    });

    assert.strictEqual(registry.getAllCommands().length, 3);

    // 5. Unregistration removes command cleanly
    unregister();
    assert.strictEqual(registry.getCommand('test.cmd1'), undefined);
    unregLibrary();
    unregSettings();
    assert.strictEqual(registry.getAllCommands().length, 0);
  });

  // -----------------------------------------------------------------------------------------------
  // 4. Dispatch & Execution
  // -----------------------------------------------------------------------------------------------
  it('§4. CommandRegistry dispatches matching events and invokes established actions', () => {
    const registry = new CommandRegistry();
    let invoked = false;
    let preventedDefault = false;

    registry.registerCommand({
      id: 'test.action',
      label: 'Action',
      description: 'Invokes action',
      category: 'general',
      defaultShortcut: { key: 'b', alt: true },
      action: () => {
        invoked = true;
      },
    });

    const fakeEvent = {
      key: 'b',
      altKey: true,
      ctrlKey: false,
      shiftKey: false,
      metaKey: false,
      preventDefault: () => {
        preventedDefault = true;
      },
    } as unknown as KeyboardEvent;

    const handled = registry.dispatch(fakeEvent);
    assert.strictEqual(handled, true);
    assert.strictEqual(invoked, true);
    assert.strictEqual(preventedDefault, true);

    // Unrecognized key returns false and does not prevent default
    let unhandledPrevented = false;
    const unhandledEvent = {
      key: 'z',
      altKey: true,
      ctrlKey: false,
      shiftKey: false,
      metaKey: false,
      preventDefault: () => {
        unhandledPrevented = true;
      },
    } as unknown as KeyboardEvent;

    const notHandled = registry.dispatch(unhandledEvent);
    assert.strictEqual(notHandled, false);
    assert.strictEqual(unhandledPrevented, false);
  });

  // -----------------------------------------------------------------------------------------------
  // 5. Input Field & Text Entry Protection
  // -----------------------------------------------------------------------------------------------
  it('§5. Text inputs protect ordinary typing from global shortcut interception', () => {
    const registry = new CommandRegistry();
    let globalFired = false;
    let escapeFired = false;

    // Single-key shortcut (like ? or /)
    registry.registerCommand({
      id: 'help.search',
      label: 'Search',
      description: 'Global search',
      category: 'general',
      defaultShortcut: { key: '/' },
      allowInInputs: false,
      action: () => {
        globalFired = true;
      },
    });

    // Escape shortcut permitted in inputs
    registry.registerCommand({
      id: 'dialog.close',
      label: 'Close',
      description: 'Dismiss',
      category: 'general',
      defaultShortcut: { key: 'Escape' },
      allowInInputs: true,
      action: () => {
        escapeFired = true;
      },
    });

    // Mock HTMLInputElement target
    const mockInput = {
      tagName: 'INPUT',
      type: 'text',
      isContentEditable: false,
    };

    assert.strictEqual(isInputElement(mockInput as any), true);

    // Typing '/' inside text input is NOT intercepted
    const slashInInputEvent = {
      key: '/',
      ctrlKey: false,
      altKey: false,
      shiftKey: false,
      metaKey: false,
      target: mockInput,
    } as unknown as KeyboardEvent;

    const slashHandled = registry.dispatch(slashInInputEvent, { targetElement: mockInput as any });
    assert.strictEqual(slashHandled, false);
    assert.strictEqual(globalFired, false);

    // Pressing 'Escape' inside text input IS handled
    const escapeInInputEvent = {
      key: 'Escape',
      ctrlKey: false,
      altKey: false,
      shiftKey: false,
      metaKey: false,
      target: mockInput,
    } as unknown as KeyboardEvent;

    const escapeHandled = registry.dispatch(escapeInInputEvent, { targetElement: mockInput as any });
    assert.strictEqual(escapeHandled, true);
    assert.strictEqual(escapeFired, true);
  });

  // -----------------------------------------------------------------------------------------------
  // 6. Modal Dialog Boundary Protection
  // -----------------------------------------------------------------------------------------------
  it('§6. Active modal dialogs suppress underlying global shortcuts', () => {
    let activeRoute = 'dashboard';
    const registry = new CommandRegistry(() => ({
      activeRoute,
      isModalOpen: true, // Modal is active!
    }));

    let navigated = false;
    registry.registerCommand({
      id: 'nav.library',
      label: 'Library',
      description: 'Navigate',
      category: 'navigation',
      defaultShortcut: { key: '2', alt: true },
      action: () => {
        navigated = true;
      },
    });

    let modalClosed = false;
    registry.registerCommand({
      id: 'modal.dismiss',
      label: 'Dismiss Modal',
      description: 'Close dialog',
      category: 'general',
      scope: 'modal',
      defaultShortcut: { key: 'Escape' },
      action: () => {
        modalClosed = true;
      },
    });

    // 1. Navigation shortcut while modal is open is suppressed
    const navEvent = {
      key: '2',
      altKey: true,
      ctrlKey: false,
      shiftKey: false,
      metaKey: false,
    } as unknown as KeyboardEvent;

    const navHandled = registry.dispatch(navEvent);
    assert.strictEqual(navHandled, false);
    assert.strictEqual(navigated, false);

    // 2. Modal-scoped Escape command succeeds
    const escapeEvent = {
      key: 'Escape',
      ctrlKey: false,
      altKey: false,
      shiftKey: false,
      metaKey: false,
    } as unknown as KeyboardEvent;

    const escapeHandled = registry.dispatch(escapeEvent);
    assert.strictEqual(escapeHandled, true);
    assert.strictEqual(modalClosed, true);
  });

  // -----------------------------------------------------------------------------------------------
  // 7. Key Repeat Suppression
  // -----------------------------------------------------------------------------------------------
  it('§7. Held-down key repeat does not trigger repeated high-impact actions', () => {
    const registry = new CommandRegistry();
    let saveCount = 0;

    registry.registerCommand({
      id: 'settings.save',
      label: 'Save',
      description: 'Save settings',
      category: 'settings',
      defaultShortcut: { key: 's', ctrl: true },
      allowRepeat: false, // Disallow repeat
      action: () => {
        saveCount++;
      },
    });

    // First keypress
    const firstPress = {
      key: 's',
      ctrlKey: true,
      altKey: false,
      shiftKey: false,
      metaKey: false,
      repeat: false,
    } as unknown as KeyboardEvent;

    assert.strictEqual(registry.dispatch(firstPress), true);
    assert.strictEqual(saveCount, 1);

    // Repeated event while held down
    const repeatPress = {
      key: 's',
      ctrlKey: true,
      altKey: false,
      shiftKey: false,
      metaKey: false,
      repeat: true,
    } as unknown as KeyboardEvent;

    assert.strictEqual(registry.dispatch(repeatPress), false);
    assert.strictEqual(saveCount, 1); // Not incremented
  });

  // -----------------------------------------------------------------------------------------------
  // 8. Lifecycle & DOM Event Listener Attachment
  // -----------------------------------------------------------------------------------------------
  it('§8. CommandRegistry attaches and detaches DOM listeners cleanly', () => {
    const registry = new CommandRegistry();
    let listenerCount = 0;
    let registeredListener: ((e: any) => void) | null = null;

    const mockTarget = {
      addEventListener: (type: string, listener: any) => {
        if (type === 'keydown') {
          listenerCount++;
          registeredListener = listener;
        }
      },
      removeEventListener: (type: string, listener: any) => {
        if (type === 'keydown' && registeredListener === listener) {
          listenerCount--;
          registeredListener = null;
        }
      },
    };

    let actionFired = false;
    registry.registerCommand({
      id: 'cmd.test',
      label: 'Test',
      description: 'Test',
      category: 'general',
      defaultShortcut: { key: 't', ctrl: true },
      action: () => {
        actionFired = true;
      },
    });

    // Attach
    const cleanup = registry.attach(mockTarget as any);
    assert.strictEqual(listenerCount, 1);

    // Trigger mock event through attached listener
    registeredListener?.({
      key: 't',
      ctrlKey: true,
      altKey: false,
      shiftKey: false,
      metaKey: false,
      preventDefault: () => {},
    });
    assert.strictEqual(actionFired, true);

    // Detach via cleanup
    cleanup();
    assert.strictEqual(listenerCount, 0);

    // Additional calls are idempotent
    registry.detach();
    assert.strictEqual(listenerCount, 0);
  });

  // -----------------------------------------------------------------------------------------------
  // 9. AppShell Default Registry Integration & Shortcuts Modal
  // -----------------------------------------------------------------------------------------------
  it('§9. AppShell initializes default command registry, renders nav hints, and manages shortcuts modal', () => {
    const routerStore = new RouterStore('dashboard');
    const catalogStore = new CatalogStore(new CatalogIpcClient({} as any));

    const shell = new AppShellController({
      routerStore,
      catalogStore,
    });

    const registry = shell.getCommandRegistry();
    assert.ok(registry.getCommand('nav.dashboard'), 'Must have nav.dashboard');
    assert.ok(registry.getCommand('nav.library'), 'Must have nav.library');
    assert.ok(registry.getCommand('nav.displays'), 'Must have nav.displays');
    assert.ok(registry.getCommand('nav.settings'), 'Must have nav.settings');
    assert.ok(registry.getCommand('library.search_focus'), 'Must have library.search_focus');
    assert.ok(registry.getCommand('help.shortcuts'), 'Must have help.shortcuts');

    // Executing nav.library through registry changes router route
    const navLibCmd = registry.getCommand('nav.library')!;
    navLibCmd.action();
    assert.strictEqual(routerStore.getState().activeRoute, 'library');

    // Render HTML and check nav hints
    const html = renderToHtml(shell.render());
    assert.ok(html.includes('Alt+1'), 'Nav button must show Alt+1 hint');
    assert.ok(html.includes('Alt+2'), 'Nav button must show Alt+2 hint');
    assert.ok(html.includes('data-testid="nav-shortcuts-btn"'), 'Must have shortcuts footer button');

    // Test shortcuts modal toggle
    assert.strictEqual(shell.isShortcutsOpen(), false);
    shell.openShortcutsModal();
    assert.strictEqual(shell.isShortcutsOpen(), true);

    const openModalHtml = renderToHtml(shell.render());
    assert.ok(openModalHtml.includes('data-testid="shortcuts-modal-dialog"'));
    assert.ok(openModalHtml.includes('Keyboard Shortcuts'));
    assert.ok(openModalHtml.includes('<kbd class="kw-kbd">Alt</kbd>'));

    shell.closeShortcutsModal();
    assert.strictEqual(shell.isShortcutsOpen(), false);

    shell.dispose();
  });
});

