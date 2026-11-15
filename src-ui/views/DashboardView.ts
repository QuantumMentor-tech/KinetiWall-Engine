/**
 * Dashboard & Home Screen Overview (`Step 23 KW-DSH-001`).
 * Provides truthful catalog summary metrics and direct navigation to the Wallpaper Library (`Step 27 §3.2, §5.1`).
 */

import { Badge, Button } from '../components/primitives/controls.ts';
import { h, type UiNode } from '../components/primitives/uiNode.ts';
import type { CatalogStoreState } from '../stores/catalogStore.ts';
import type { RouterStore } from '../stores/routerStore.ts';

export interface DashboardViewProps {
  routerStore: RouterStore;
  catalogState: CatalogStoreState;
  onAddWallpaper: () => void;
}

export function DashboardView(props: DashboardViewProps): UiNode {
  const { routerStore, catalogState } = props;
  const totalCount = catalogState.entries.length;
  const availableCount = catalogState.integrity.available_count;

  return h(
    'section',
    {
      attrs: {
        'data-testid': 'dashboard-view',
        'aria-labelledby': 'dashboard-page-heading',
        class: 'kw-dashboard-view',
      },
    },
    h(
      'header',
      { attrs: { class: 'kw-page-header' } },
      h(
        'div',
        { attrs: { class: 'kw-page-header-titles' } },
        h(
          'h1',
          {
            attrs: {
              id: 'dashboard-page-heading',
              'data-testid': 'dashboard-heading',
              class: 'kw-page-title',
            },
          },
          'KinetiWall Dashboard',
        ),
        h(
          'p',
          { attrs: { class: 'kw-page-subtitle' } },
          'Local wallpaper catalog overview and quick actions',
        ),
      ),
      h(
        'div',
        { attrs: { class: 'kw-page-header-actions' } },
        Badge({
          testId: 'dashboard-catalog-status-badge',
          label:
            catalogState.status === 'ready'
              ? `${totalCount} Registered (${availableCount} Available)`
              : 'Catalog Ready on Demand',
          iconSymbol: '●',
          tone: catalogState.status === 'error' ? 'warning' : 'cyan',
        }),
      ),
    ),
    h(
      'div',
      { attrs: { class: 'kw-dashboard-cards' } },
      h(
        'article',
        {
          attrs: {
            'data-testid': 'dashboard-library-summary-card',
            class: 'kw-dashboard-card',
          },
        },
        h('h2', { attrs: { class: 'kw-card-heading' } }, 'Wallpaper Library'),
        h(
          'p',
          { attrs: { class: 'kw-card-copy' } },
          totalCount > 0
            ? `Your local catalog contains ${totalCount} registered wallpaper(s), with ${availableCount} verified available on disk.`
            : 'Browse your local wallpaper catalog or register new video and web assets.',
        ),
        h(
          'div',
          { attrs: { class: 'kw-dashboard-actions' } },
          Button({
            testId: 'dashboard-browse-library-btn',
            label: 'Browse Wallpaper Library',
            variant: 'primary',
            onClick: () => {
              routerStore.navigate('library');
            },
          }),
          Button({
            testId: 'dashboard-add-wallpaper-btn',
            label: 'Add Wallpaper',
            variant: 'secondary',
            isLoading: catalogState.registrationInFlight,
            onClick: props.onAddWallpaper,
          }),
        ),
      ),
      h(
        'article',
        {
          attrs: {
            'data-testid': 'dashboard-displays-summary-card',
            class: 'kw-dashboard-card',
          },
        },
        h('h2', { attrs: { class: 'kw-card-heading' } }, 'Displays & Layout'),
        h(
          'p',
          { attrs: { class: 'kw-card-copy' } },
          'Inspect connected monitors, configure multi-monitor presentation modes (Independent, Span, Clone), and assign per-display wallpapers.',
        ),
        h(
          'div',
          { attrs: { class: 'kw-dashboard-actions' } },
          Button({
            testId: 'dashboard-manage-displays-btn',
            label: 'Manage Displays',
            variant: 'secondary',
            onClick: () => {
              routerStore.navigate('displays');
            },
          }),
        ),
      ),
    ),
  );
}

