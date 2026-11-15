/**
 * Root Application Shell (`Step 22 KW-SHL-001` & `Step 27 §5.1`).
 *
 * Provides the persistent Neon Glass navigation rail, active-route indication (`aria-current="page"`),
 * disabled-state guardrails for unimplemented routes (`DEC-SHL-04`), top-level error boundary
 * containment (`Principle J`), and the main content viewport.
 */

import { AlertBanner, Button } from '../primitives/controls.ts';
import { h, type UiNode } from '../primitives/uiNode.ts';
import { NAVIGATION_ROUTES, type RouteId } from '../../routes.ts';
import { DisplayIpcClient, createMockDisplayIpcTransport } from '../../services/displayIpcClient.ts';
import type { CatalogStore } from '../../stores/catalogStore.ts';
import { DisplayStore } from '../../stores/displayStore.ts';
import type { RouterStore } from '../../stores/routerStore.ts';
import { DashboardView } from '../../views/DashboardView.ts';
import { DisplaysView } from '../../views/DisplaysView.ts';
import { LibraryView } from '../../views/LibraryView.ts';

export interface AppShellProps {
  routerStore: RouterStore;
  catalogStore: CatalogStore;
  displayStore?: DisplayStore;
}

export class AppShellController {
  private readonly routerStore: RouterStore;
  private readonly catalogStore: CatalogStore;
  private readonly displayStore: DisplayStore;
  private viewError: string | null = null;

  constructor(props: AppShellProps) {
    this.routerStore = props.routerStore;
    this.catalogStore = props.catalogStore;
    this.displayStore =
      props.displayStore ?? new DisplayStore(new DisplayIpcClient(createMockDisplayIpcTransport()));

    // If launched directly on the library route, trigger initial catalog load once if still idle
    if (
      this.routerStore.getState().activeRoute === 'library' &&
      this.catalogStore.getState().status === 'idle'
    ) {
      void this.catalogStore.loadCatalog();
    }

    // If launched directly on the displays route, trigger initial display topology discovery
    if (
      this.routerStore.getState().activeRoute === 'displays' &&
      !this.displayStore.getState().snapshot
    ) {
      void this.displayStore.refreshTopology();
    }

    // Automatically ensure store data is loaded when navigating
    this.routerStore.subscribe((routerState) => {
      if (
        routerState.activeRoute === 'library' &&
        this.catalogStore.getState().status === 'idle'
      ) {
        void this.catalogStore.loadCatalog();
      } else if (
        routerState.activeRoute === 'displays' &&
        !this.displayStore.getState().snapshot
      ) {
        void this.displayStore.refreshTopology();
      }
    });
  }

  navigate(routeId: RouteId): boolean {
    this.viewError = null;
    return this.routerStore.navigate(routeId);
  }

  render(): UiNode {
    const routerState = this.routerStore.getState();
    const catalogState = this.catalogStore.getState();

    let mainViewNode: UiNode;
    try {
      if (this.viewError) {
        throw new Error(this.viewError);
      }
      if (routerState.activeRoute === 'library') {
        mainViewNode = LibraryView({
          store: this.catalogStore,
          state: catalogState,
          viewportWidthPx: routerState.viewportWidthPx,
        });
      } else if (routerState.activeRoute === 'displays') {
        mainViewNode = DisplaysView({
          displayStore: this.displayStore,
          catalogStore: this.catalogStore,
        });
      } else {
        mainViewNode = DashboardView({
          routerStore: this.routerStore,
          catalogState,
          onAddWallpaper: () => {
            this.routerStore.navigate('library');
            void this.catalogStore.triggerRegistration();
          },
        });
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      mainViewNode = AlertBanner({
        testId: 'shell-error-boundary',
        tone: 'error',
        title: 'View Rendering Error',
        message: msg,
        actionButton: Button({
          testId: 'shell-error-reset-btn',
          label: 'Reset View',
          variant: 'primary',
          size: 'sm',
          onClick: () => {
            this.viewError = null;
            this.routerStore.navigate('dashboard');
          },
        }),
      });
    }

    return h(
      'div',
      {
        attrs: {
          'data-testid': 'kinetiwall-app-shell',
          'data-active-route': routerState.activeRoute,
          'data-viewport-width': String(routerState.viewportWidthPx),
          class: 'kw-shell',
        },
      },
      // Left Navigation Rail
      h(
        'nav',
        {
          attrs: {
            role: 'navigation',
            'aria-label': 'Primary Navigation',
            'data-testid': 'shell-nav-rail',
            class: 'kw-nav-rail',
          },
        },
        h(
          'div',
          { attrs: { class: 'kw-brand-header', 'data-testid': 'shell-brand-header' } },
          h('span', { attrs: { class: 'kw-brand-mark', 'aria-hidden': 'true' } }, '◆'),
          h('span', { attrs: { class: 'kw-brand-name' } }, 'KinetiWall'),
        ),
        h(
          'ul',
          { attrs: { class: 'kw-nav-list', role: 'list' } },
          ...NAVIGATION_ROUTES.map((route) => {
            const isActive = routerState.activeRoute === route.id;
            const isDisabled = !route.isAvailable;
            return h(
              'li',
              { key: route.id, attrs: { class: 'kw-nav-item' } },
              h(
                'button',
                {
                  attrs: {
                    type: 'button',
                    'data-testid': `nav-item-${route.id}`,
                    'data-route-id': route.id,
                    'data-active': isActive ? 'true' : 'false',
                    class: `kw-nav-btn kw-focus-ring${isActive ? ' kw-nav-btn-active' : ''}${
                      isDisabled ? ' kw-nav-btn-disabled' : ''
                    }`,
                    disabled: isDisabled,
                    'aria-disabled': isDisabled ? 'true' : undefined,
                    'aria-current': isActive ? 'page' : undefined,
                    'aria-label': isDisabled
                      ? `${route.label} (Unavailable: ${route.unavailableReason})`
                      : route.label,
                    title: isDisabled ? route.unavailableReason : route.shortDescription,
                  },
                  onClick: isDisabled
                    ? undefined
                    : () => {
                        this.navigate(route.id);
                      },
                  onKeyDown: isDisabled
                    ? undefined
                    : (e) => {
                        if (e.key === 'Enter' || e.key === ' ') {
                          e.preventDefault?.();
                          this.navigate(route.id);
                        }
                      },
                },
                h('span', { attrs: { class: 'kw-nav-label' } }, route.label),
              ),
            );
          }),
        ),
      ),
      // Main Content Viewport
      h(
        'main',
        {
          attrs: {
            role: 'main',
            'data-testid': 'shell-main-viewport',
            class: 'kw-main-viewport',
          },
        },
        mainViewNode,
      ),
    );
  }
}
