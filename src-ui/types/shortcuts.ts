/**
 * Keyboard Shortcut Architecture & Command Registry Types (Step 64).
 * Authoritative types, modifier definitions, and canonical matching utilities.
 */

export type ShortcutModifier = 'ctrl' | 'alt' | 'shift' | 'meta';

export type CommandCategory = 'navigation' | 'library' | 'displays' | 'settings' | 'general';

export type CommandScope = 'global' | 'library' | 'displays' | 'settings' | 'modal';

export interface ShortcutKeyCombo {
  /** Normalized key value (e.g., '1', 'f', 'r', 's', '?', 'Escape', 'F1') */
  key: string;
  /** Physical key code fallback (e.g., 'Digit1', 'KeyF', 'KeyR') */
  code?: string;
  ctrl?: boolean;
  alt?: boolean;
  shift?: boolean;
  meta?: boolean;
}

export interface ApplicationCommand {
  /** Stable unique identifier (e.g., 'nav.dashboard', 'library.search') */
  id: string;
  /** Human-readable title */
  label: string;
  /** Detailed description for help dialog and screen readers */
  description: string;
  /** Functional category for grouping in help reference */
  category: CommandCategory;
  /** Authoritative default key combination */
  defaultShortcut: ShortcutKeyCombo;
  /** Active shortcut key combination */
  shortcut?: ShortcutKeyCombo;
  /** Execution scope: global or restricted to specific views */
  scope?: CommandScope;
  /** Whether this command is permitted to execute when focus is inside text inputs */
  allowInInputs?: boolean;
  /** Whether this command allows continuous execution when key is held down (repeat) */
  allowRepeat?: boolean;
  /** Established application action */
  action: () => void | Promise<void>;
  /** Optional predicate to check if command is currently available */
  isEnabled?: () => boolean;
}

export interface ShortcutDispatchContext {
  activeRoute?: string;
  isModalOpen?: boolean;
  targetElement?: EventTarget | null;
}

/**
 * Checks whether an event target is an interactive text entry control.
 */
export function isInputElement(target: EventTarget | null): boolean {
  if (!target || typeof target !== 'object' || !('tagName' in target)) {
    return false;
  }
  const el = target as HTMLElement;
  const tagName = el.tagName ? el.tagName.toUpperCase() : '';

  if (tagName === 'INPUT') {
    const inputType = (el as HTMLInputElement).type ? (el as HTMLInputElement).type.toLowerCase() : 'text';
    // Non-text input types that don't capture typing
    const nonTextTypes = ['checkbox', 'radio', 'button', 'submit', 'reset', 'range', 'color'];
    return !nonTextTypes.includes(inputType);
  }

  if (tagName === 'TEXTAREA' || tagName === 'SELECT') {
    return true;
  }

  if (el.isContentEditable || el.getAttribute('contenteditable') === 'true') {
    return true;
  }

  return false;
}

/**
 * Normalizes a ShortcutKeyCombo into a canonical string hash for conflict detection and comparison.
 * Format: [CTRL+][ALT+][SHIFT+][META+]<NORMALIZED_KEY>
 */
export function normalizeShortcutKeyCombo(combo: ShortcutKeyCombo): string {
  const parts: string[] = [];
  if (combo.ctrl) parts.push('CTRL');
  if (combo.alt) parts.push('ALT');
  if (combo.shift) parts.push('SHIFT');
  if (combo.meta) parts.push('META');

  // Normalize key name
  let keyNorm = combo.key.toUpperCase();
  if (keyNorm === ' ' || keyNorm === 'SPACEBAR') {
    keyNorm = 'SPACE';
  } else if (keyNorm === 'ESC') {
    keyNorm = 'ESCAPE';
  }

  parts.push(keyNorm);
  return parts.join('+');
}

/**
 * Formats a key combo into a user-friendly display string (e.g. "Alt+1", "Ctrl+Shift+R").
 */
export function formatShortcutKeyCombo(combo: ShortcutKeyCombo): string {
  const parts: string[] = [];
  if (combo.ctrl) parts.push('Ctrl');
  if (combo.alt) parts.push('Alt');
  if (combo.shift) parts.push('Shift');
  if (combo.meta) parts.push('Win');

  let keyDisplay = combo.key;
  if (keyDisplay.length === 1) {
    keyDisplay = keyDisplay.toUpperCase();
  } else if (keyDisplay === 'Escape') {
    keyDisplay = 'Esc';
  }

  parts.push(keyDisplay);
  return parts.join('+');
}

/**
 * Deterministically checks whether a DOM KeyboardEvent matches a ShortcutKeyCombo.
 */
export function matchesKeyEvent(combo: ShortcutKeyCombo, event: KeyboardEvent): boolean {
  const ctrlRequired = Boolean(combo.ctrl);
  const altRequired = Boolean(combo.alt);
  const shiftRequired = Boolean(combo.shift);
  const metaRequired = Boolean(combo.meta);

  // Exact modifier matching
  if (event.ctrlKey !== ctrlRequired) return false;
  if (event.altKey !== altRequired) return false;
  if (event.shiftKey !== shiftRequired) return false;
  if (event.metaKey !== metaRequired) return false;

  // Key matching: check event.key (case-insensitive) or event.code
  const targetKeyLower = combo.key.toLowerCase();
  const eventKeyLower = event.key ? event.key.toLowerCase() : '';

  if (targetKeyLower === eventKeyLower) {
    return true;
  }

  // Handle common key aliases
  if (
    (targetKeyLower === 'escape' && eventKeyLower === 'esc') ||
    (targetKeyLower === 'esc' && eventKeyLower === 'escape')
  ) {
    return true;
  }

  // Physical code matching if specified (e.g. Digit1 vs 1)
  if (combo.code && event.code && combo.code.toLowerCase() === event.code.toLowerCase()) {
    return true;
  }

  return false;
}
