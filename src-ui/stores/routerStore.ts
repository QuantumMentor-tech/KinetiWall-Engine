/**
 * Authoritative Client View Router Store (Step 22 `KW-SHL-001` & Principle I Anti-Thrashing).
 * Preserves view context across navigation switches without reinitializing native processes.
 */

import { getRouteDefinition, NAVIGATION_ROUTES, type RouteId } from '../routes.ts';

export interface RouterState {
  activeRoute: RouteId;
  previousRoute: RouteId | null;
  navigationCount: number;
  viewportWidthPx: number;
  viewportHeightPx: number;
  scrollPositions: Record<RouteId, number>;
}

export type RouterListener = (state: RouterState) => void;

export class RouterStore {
  private state: RouterState;
  private listeners: Set<RouterListener> = new Set();

  constructor(initialRoute: RouteId = 'dashboard', widthPx = 1280, heightPx = 800) {
    this.state = {
      activeRoute: initialRoute,
      previousRoute: null,
      navigationCount: 0,
      viewportWidthPx: Math.max(960, widthPx),
      viewportHeightPx: Math.max(600, heightPx),
      scrollPositions: {
        dashboard: 0,
        library: 0,
        displays: 0,
        settings: 0,
      },
    };
  }

  getState(): RouterState {
    return {
      ...this.state,
      scrollPositions: { ...this.state.scrollPositions },
    };
  }

  subscribe(listener: RouterListener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  /**
   * Navigates to a route if it is available and not already active (`Principle I: Anti-Thrashing`).
   * Returns `true` if a navigation transition occurred, or `false` if ignored.
   */
  navigate(targetRoute: RouteId): boolean {
    const routeDef = getRouteDefinition(targetRoute);
    if (!routeDef.isAvailable) {
      return false;
    }
    if (this.state.activeRoute === targetRoute) {
      // Anti-thrashing no-op: clicking the active route must not remount or refetch.
      return false;
    }
    this.state = {
      ...this.state,
      previousRoute: this.state.activeRoute,
      activeRoute: targetRoute,
      navigationCount: this.state.navigationCount + 1,
    };
    this.emit();
    return true;
  }

  setScrollPosition(route: RouteId, scrollTop: number): void {
    this.state = {
      ...this.state,
      scrollPositions: {
        ...this.state.scrollPositions,
        [route]: Math.max(0, scrollTop),
      },
    };
  }

  setViewportSize(widthPx: number, heightPx: number): void {
    this.state = {
      ...this.state,
      viewportWidthPx: Math.max(960, Math.round(widthPx)),
      viewportHeightPx: Math.max(600, Math.round(heightPx)),
    };
    this.emit();
  }

  getAvailableRoutes(): readonly typeof NAVIGATION_ROUTES[number][] {
    return NAVIGATION_ROUTES;
  }

  private emit(): void {
    const snapshot = this.getState();
    for (const listener of this.listeners) {
      listener(snapshot);
    }
  }
}

