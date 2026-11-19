/**
 * Operating System Reduced Motion Detection & Media Query Synchronization (Step 63).
 * Provides event-driven, leak-free subscriptions to Windows/OS prefers-reduced-motion changes.
 */

export interface SystemMotionDetector {
  /** Returns the current OS reduced-motion state. */
  isReducedMotion(): boolean;
  /** Whether the current runtime environment supports OS motion preference detection. */
  isSupported(): boolean;
  /** Registers a listener for OS reduced-motion changes. Returns a cleanup disposal function. */
  subscribe(listener: (reducedMotion: boolean) => void): () => void;
}

/**
 * Standard browser / WebView2 implementation using `window.matchMedia('(prefers-reduced-motion: reduce)')`.
 */
export class BrowserSystemMotionDetector implements SystemMotionDetector {
  private mediaQueryList: MediaQueryList | null = null;

  constructor() {
    if (typeof window !== 'undefined' && typeof window.matchMedia === 'function') {
      try {
        this.mediaQueryList = window.matchMedia('(prefers-reduced-motion: reduce)');
      } catch {
        this.mediaQueryList = null;
      }
    }
  }

  isSupported(): boolean {
    return this.mediaQueryList !== null;
  }

  isReducedMotion(): boolean {
    if (!this.mediaQueryList) {
      return false; // Safe default
    }
    return this.mediaQueryList.matches;
  }

  subscribe(listener: (reducedMotion: boolean) => void): () => void {
    if (!this.mediaQueryList) {
      return () => {};
    }

    const handler = (e: MediaQueryListEvent | MediaQueryList) => {
      const reduced = 'matches' in e ? e.matches : false;
      listener(reduced);
    };

    const mql = this.mediaQueryList;
    if (typeof mql.addEventListener === 'function') {
      mql.addEventListener('change', handler);
      return () => {
        mql.removeEventListener('change', handler);
      };
    } else if (typeof (mql as unknown as { addListener?: (cb: typeof handler) => void }).addListener === 'function') {
      // Fallback for older WebView2 runtimes
      (mql as unknown as { addListener: (cb: typeof handler) => void }).addListener(handler);
      return () => {
        (mql as unknown as { removeListener: (cb: typeof handler) => void }).removeListener(handler);
      };
    }

    return () => {};
  }
}

/**
 * Mock system motion detector for headless unit tests and controlled test scenarios.
 */
export class MockSystemMotionDetector implements SystemMotionDetector {
  private reducedMotion: boolean;
  private readonly listeners: Set<(reducedMotion: boolean) => void> = new Set();
  private supported: boolean;

  constructor(initialReducedMotion: boolean = false, isSupported: boolean = true) {
    this.reducedMotion = initialReducedMotion;
    this.supported = isSupported;
  }

  isSupported(): boolean {
    return this.supported;
  }

  isReducedMotion(): boolean {
    return this.reducedMotion;
  }

  setSupported(supported: boolean): void {
    this.supported = supported;
  }

  setSystemReducedMotion(newReducedMotion: boolean): void {
    if (this.reducedMotion === newReducedMotion) {
      return;
    }
    this.reducedMotion = newReducedMotion;
    for (const listener of this.listeners) {
      try {
        listener(newReducedMotion);
      } catch (err) {
        console.error('[MockSystemMotionDetector] Error notifying listener:', err);
      }
    }
  }

  subscribe(listener: (reducedMotion: boolean) => void): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  listenerCount(): number {
    return this.listeners.size;
  }
}

