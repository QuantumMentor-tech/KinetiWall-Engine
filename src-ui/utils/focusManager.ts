/**
 * Accessible Focus Management & Keyboard Navigation Utilities (Step 63).
 * Provides modal focus trapping, focus stack restoration, and roving keyboard navigation.
 */

const FOCUSABLE_SELECTOR = [
  'a[href]',
  'button:not([disabled])',
  'input:not([disabled])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]:not([tabindex="-1"])',
  '[role="button"]:not([disabled])',
  '[role="switch"]:not([disabled])',
  '[role="tab"]:not([disabled])',
].join(', ');

/**
 * Returns all interactive, focusable child elements within a container.
 */
export function getFocusableElements(container: HTMLElement): HTMLElement[] {
  if (!container || typeof container.querySelectorAll !== 'function') {
    return [];
  }

  const elements = Array.from(container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR));
  return elements.filter((el) => {
    // Exclude elements that are hidden or disabled via aria
    if (el.getAttribute('aria-hidden') === 'true') {
      return false;
    }
    if (el.hasAttribute('disabled')) {
      return false;
    }
    return true;
  });
}

/**
 * Traps Tab / Shift+Tab cycling within a container (e.g. a modal or dialog).
 * Returns true if the key event was intercepted and handled.
 */
export function trapFocus(container: HTMLElement, event: KeyboardEvent): boolean {
  if (event.key !== 'Tab') {
    return false;
  }

  const focusables = getFocusableElements(container);
  if (focusables.length === 0) {
    event.preventDefault();
    return true;
  }

  const first = focusables[0];
  const last = focusables[focusables.length - 1];
  const active = typeof document !== 'undefined' ? (document.activeElement as HTMLElement) : null;

  if (event.shiftKey) {
    if (active === first || !container.contains(active)) {
      last.focus();
      event.preventDefault();
      return true;
    }
  } else {
    if (active === last || !container.contains(active)) {
      first.focus();
      event.preventDefault();
      return true;
    }
  }

  return false;
}

/**
 * Stack-based focus restoration for modals, dropdowns, and transient dialogs.
 */
export class FocusRestorationStack {
  private static instance: FocusRestorationStack | null = null;
  private readonly stack: HTMLElement[] = [];

  public static getInstance(): FocusRestorationStack {
    if (!FocusRestorationStack.instance) {
      FocusRestorationStack.instance = new FocusRestorationStack();
    }
    return FocusRestorationStack.instance;
  }

  /**
   * Pushes the current active element (or specified element) onto the restoration stack.
   */
  public push(element?: HTMLElement | null): void {
    const el =
      element ?? (typeof document !== 'undefined' ? (document.activeElement as HTMLElement) : null);
    if (el) {
      this.stack.push(el);
    }
  }

  /**
   * Pops the most recent element from the stack and restores focus to it.
   */
  public popAndRestore(): boolean {
    const target = this.stack.pop();
    if (target && typeof target.focus === 'function') {
      try {
        target.focus();
        return true;
      } catch {
        return false;
      }
    }
    return false;
  }

  /**
   * Returns current stack depth.
   */
  public depth(): number {
    return this.stack.length;
  }

  /**
   * Clears the stack without restoring focus.
   */
  public clear(): void {
    this.stack.length = 0;
  }
}

