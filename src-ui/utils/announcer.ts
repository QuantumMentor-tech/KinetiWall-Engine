/**
 * Screen Reader Live Region Announcer (Step 63).
 * Provides polite and assertive live announcements for assistive technologies
 * without disrupting visual layout or injecting duplicate noise.
 */

export type Politeness = 'polite' | 'assertive';

export class ScreenReaderAnnouncer {
  private static instance: ScreenReaderAnnouncer | null = null;
  private container: HTMLElement | null = null;
  private lastMessage: string = '';
  private lastTimestamp: number = 0;
  private debounceMs: number = 400;

  constructor(debounceMs: number = 400) {
    this.debounceMs = debounceMs;
  }

  public static getInstance(): ScreenReaderAnnouncer {
    if (!ScreenReaderAnnouncer.instance) {
      ScreenReaderAnnouncer.instance = new ScreenReaderAnnouncer();
    }
    return ScreenReaderAnnouncer.instance;
  }

  public static resetInstance(): void {
    if (ScreenReaderAnnouncer.instance) {
      ScreenReaderAnnouncer.instance.cleanup();
      ScreenReaderAnnouncer.instance = null;
    }
  }

  /**
   * Dispatches an audible announcement for assistive technologies.
   */
  public announce(message: string, politeness: Politeness = 'polite'): void {
    const trimmed = message.trim();
    if (!trimmed) {
      return;
    }

    const now = Date.now();
    // Deduplicate identical rapid messages within the debounce window
    if (trimmed === this.lastMessage && now - this.lastTimestamp < this.debounceMs) {
      return;
    }

    this.lastMessage = trimmed;
    this.lastTimestamp = now;

    if (typeof document === 'undefined') {
      return;
    }

    const container = this.getOrCreateContainer(politeness);
    if (!container) {
      return;
    }

    // Set text after a brief tick to ensure screen readers detect attribute/text mutation
    container.textContent = '';
    setTimeout(() => {
      if (container) {
        container.textContent = trimmed;
      }
    }, 50);
  }

  public getLastMessage(): string {
    return this.lastMessage;
  }

  private getOrCreateContainer(politeness: Politeness): HTMLElement | null {
    if (typeof document === 'undefined' || !document.body) {
      return null;
    }

    const id = `kw-live-announcer-${politeness}`;
    let el = document.getElementById(id);
    if (!el) {
      el = document.createElement('div');
      el.id = id;
      el.className = 'kw-sr-only';
      el.setAttribute('role', politeness === 'assertive' ? 'alert' : 'status');
      el.setAttribute('aria-live', politeness);
      el.setAttribute('aria-atomic', 'true');
      document.body.appendChild(el);
    }
    this.container = el;
    return el;
  }

  public cleanup(): void {
    if (typeof document !== 'undefined') {
      const politeEl = document.getElementById('kw-live-announcer-polite');
      if (politeEl && politeEl.parentNode) {
        politeEl.parentNode.removeChild(politeEl);
      }
      const assertiveEl = document.getElementById('kw-live-announcer-assertive');
      if (assertiveEl && assertiveEl.parentNode) {
        assertiveEl.parentNode.removeChild(assertiveEl);
      }
    }
    this.container = null;
    this.lastMessage = '';
    this.lastTimestamp = 0;
  }
}

/**
 * Convenience helper to announce a polite message.
 */
export function announce(message: string, politeness: Politeness = 'polite'): void {
  ScreenReaderAnnouncer.getInstance().announce(message, politeness);
}

