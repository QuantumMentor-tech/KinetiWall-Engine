/**
 * Wallpaper Library Browsing UI & Responsive Catalog Grid (`Step 27`).
 *
 * Connects the Step 22 Application Shell to the Step 25/26 `kinetiwall-catalog` service:
 * - Displays real catalog records (`CatalogEntrySummary`) keyed by stable `WallpaperId`.
 * - Distinctly represents loading, empty, error, stale-refresh, and partial-availability states.
 * - Adapts column layout across supported desktop window widths (`960px` to `3840px`).
 * - Supports library-level inspection selection without triggering playback or desktop changes.
 */

import { WallpaperCard } from '../components/domain/WallpaperCard.ts';
import { WallpaperDetailsPanel } from '../components/domain/WallpaperDetailsPanel.ts';
import {
  AlertBanner,
  Badge,
  Button,
  EmptyStatePanel,
  SearchInput,
  SegmentedFilterGroup,
  SortSelect,
  Spinner,
  type FilterOption,
  type SortOption,
} from '../components/primitives/controls.ts';
import { h, type UiNode } from '../components/primitives/uiNode.ts';
import type { CatalogStore, CatalogStoreState } from '../stores/catalogStore.ts';
import { computeResponsiveGridMetrics } from '../styles/tokens.ts';
import type { RegistrationBatchReport } from '../types/catalog.ts';
import type {
  CatalogAvailabilityFilter,
  CatalogSortOrder,
  CatalogTypeFilter,
} from '../utils/catalogQuery.ts';

const TYPE_FILTER_OPTIONS: readonly FilterOption<CatalogTypeFilter>[] = [
  { value: 'all', label: 'All Types', testId: 'library-type-filter-all' },
  { value: 'video', label: 'Video', testId: 'library-type-filter-video' },
  { value: 'web', label: 'Web', testId: 'library-type-filter-web' },
] as const;

const AVAILABILITY_FILTER_OPTIONS: readonly FilterOption<CatalogAvailabilityFilter>[] = [
  { value: 'all', label: 'All Statuses', testId: 'library-availability-filter-all' },
  { value: 'available', label: 'Available', testId: 'library-availability-filter-available' },
  { value: 'unavailable', label: 'Unavailable', testId: 'library-availability-filter-unavailable' },
  { value: 'unchecked', label: 'Unverified', testId: 'library-availability-filter-unchecked' },
] as const;

const SORT_OPTIONS: readonly SortOption<CatalogSortOrder>[] = [
  { value: 'oldest_asc', label: 'Oldest Added' },
  { value: 'recent_desc', label: 'Recently Added' },
  { value: 'title_asc', label: 'Title (A–Z)' },
  { value: 'title_desc', label: 'Title (Z–A)' },
] as const;

export interface LibraryViewProps {
  store: CatalogStore;
  state: CatalogStoreState;
  viewportWidthPx: number;
}

function formatRegistrationSummary(report: RegistrationBatchReport): {
  tone: 'success' | 'warning' | 'error' | 'info';
  title: string;
  message: string;
} {
  switch (report.overall_status) {
    case 'succeeded':
      return {
        tone: 'success',
        title: 'Wallpaper Registration Complete',
        message: `Registered ${report.registered_count} new item(s)${
          report.already_present_count > 0
            ? ` (${report.already_present_count} already in catalog)`
            : ''
        }.`,
      };
    case 'partial_success':
      return {
        tone: 'warning',
        title: 'Partial Wallpaper Registration',
        message: `Registered ${report.registered_count} of ${report.total_selected} selected item(s). ${
          report.rejected_count + report.inaccessible_count + report.failed_count
        } item(s) could not be registered.`,
      };
    case 'all_failed':
      return {
        tone: 'error',
        title: 'Wallpaper Registration Failed',
        message: `None of the ${report.total_selected} selected item(s) passed validation or availability checks.`,
      };
    default:
      return {
        tone: 'info',
        title: 'Registration Status',
        message: 'No items were registered.',
      };
  }
}

export function LibraryView(props: LibraryViewProps): UiNode {
  const { store, state, viewportWidthPx } = props;
  const gridMetrics = computeResponsiveGridMetrics(viewportWidthPx);
  const selectedEntry = store.getSelectedWallpaper();
  const queryResult = store.getQueryResult();
  const isSelectedHiddenByFilter =
    selectedEntry !== null &&
    !queryResult.visibleEntries.some((entry) => entry.id === selectedEntry.id);

  const unavailableCount =
    state.integrity.missing_count +
    state.integrity.inaccessible_count +
    state.integrity.invalid_asset_count;

  const isInitialLoading =
    (state.status === 'idle' || state.status === 'loading') && state.entries.length === 0;

  return h(
    'section',
    {
      attrs: {
        'data-testid': 'library-view',
        'data-status': state.status,
        'data-responsive-tier': gridMetrics.tier,
        'data-grid-columns': String(gridMetrics.columns),
        'aria-labelledby': 'library-page-heading',
        class: 'kw-library-view',
      },
    },
    // Page Header & Primary Actions (Step 27 §5.2, §9.4)
    h(
      'header',
      {
        attrs: {
          'data-testid': 'library-header',
          class: 'kw-page-header',
        },
      },
      h(
        'div',
        { attrs: { class: 'kw-page-header-titles' } },
        h(
          'h1',
          {
            attrs: {
              id: 'library-page-heading',
              'data-testid': 'library-heading',
              class: 'kw-page-title',
            },
          },
          'Wallpaper Library',
        ),
        h(
          'p',
          {
            attrs: {
              'data-testid': 'library-subtitle',
              class: 'kw-page-subtitle',
            },
          },
          state.status === 'ready'
            ? `${state.entries.length} registered wallpaper${
                state.entries.length === 1 ? '' : 's'
              } • ${state.integrity.available_count} verified available`
            : 'Browse registered local video and interactive web wallpapers',
        ),
      ),
      h(
        'div',
        { attrs: { class: 'kw-page-header-actions' } },
        Button({
          testId: 'library-refresh-btn',
          label: state.isRefreshing ? 'Refreshing...' : 'Refresh Catalog',
          variant: 'secondary',
          size: 'md',
          isLoading: state.isRefreshing,
          disabled: isInitialLoading,
          onClick: () => {
            void store.loadCatalog({ forceRefresh: true });
          },
        }),
        Button({
          testId: 'library-add-wallpaper-btn',
          label: state.registrationInFlight ? 'Adding Wallpaper...' : 'Add Wallpaper',
          ariaLabel: 'Add local wallpaper to catalog',
          variant: 'primary',
          size: 'md',
          isLoading: state.registrationInFlight,
          onClick: () => {
            void store.triggerRegistration();
          },
        }),
      ),
    ),

    // Registration Error Banner (Step 26/27 §8.5, §9.4)
    state.registrationErrorMessage
      ? AlertBanner({
          testId: 'library-registration-error-banner',
          tone: 'error',
          title: 'Could Not Complete Wallpaper Registration',
          message: state.registrationErrorMessage,
          dismissButton: Button({
            testId: 'library-dismiss-reg-error-btn',
            label: 'Dismiss',
            variant: 'ghost',
            size: 'sm',
            onClick: () => store.dismissRegistrationReport(),
          }),
        })
      : null,

    // Step 30 Top-Level Catalog Management Feedback Banner (shown when details panel is closed, e.g. after removal)
    !selectedEntry && state.managementFeedback
      ? AlertBanner({
          testId: 'library-management-feedback-banner',
          tone: state.managementFeedback.kind,
          title:
            state.managementFeedback.kind === 'error'
              ? 'Catalog Management Error'
              : state.managementFeedback.kind === 'warning'
                ? 'Catalog Safeguard Notice'
                : 'Catalog Management Status',
          message: state.managementFeedback.message,
          dismissButton: Button({
            testId: 'library-dismiss-mgmt-feedback-btn',
            label: 'Dismiss',
            variant: 'ghost',
            size: 'sm',
            onClick: () => store.dismissManagementFeedback(),
          }),
        })
      : null,

    // Registration Batch Outcome Banner (Step 26/27 §8.5)
    state.lastRegistrationReport
      ? (() => {
          const summary = formatRegistrationSummary(state.lastRegistrationReport);
          return AlertBanner({
            testId: 'library-registration-report-banner',
            tone: summary.tone,
            title: summary.title,
            message: summary.message,
            dismissButton: Button({
              testId: 'library-dismiss-reg-report-btn',
              label: 'Dismiss',
              variant: 'ghost',
              size: 'sm',
              onClick: () => store.dismissRegistrationReport(),
            }),
          });
        })()
      : null,

    // Stale Refresh Failure Warning (Step 27 §8.4: preserves existing records on refresh failure)
    state.isStale && state.errorMessage
      ? AlertBanner({
          testId: 'library-stale-warning-banner',
          tone: 'warning',
          title: 'Showing Previously Loaded Catalog Records',
          message: `Latest catalog refresh failed: ${state.errorMessage}`,
          actionButton: Button({
            testId: 'library-stale-retry-btn',
            label: 'Retry Refresh',
            variant: 'secondary',
            size: 'sm',
            onClick: () => {
              void store.loadCatalog({ forceRefresh: true });
            },
          }),
        })
      : null,

    // Partial-Availability / Manifest Load Issue Banner (Step 27 §4.1, §9.3)
    state.status === 'ready' && (unavailableCount > 0 || state.loadIssues.length > 0)
      ? AlertBanner({
          testId: 'library-integrity-warning-banner',
          tone: 'warning',
          title: 'Catalog Availability & Integrity Notice',
          message: [
            unavailableCount > 0
              ? `${unavailableCount} catalog record(s) have missing, inaccessible, or invalid source files.`
              : null,
            state.loadIssues.length > 0
              ? `${state.loadIssues.length} package manifest(s) could not be loaded and were preserved on disk for diagnostics.`
              : null,
          ]
            .filter(Boolean)
            .join(' '),
        })
      : null,

    // Main Content Body: Loading / Error / Empty / Populated Responsive Grid
    isInitialLoading
      ? h(
          'div',
          {
            attrs: {
              'data-testid': 'library-loading-state',
              role: 'status',
              'aria-live': 'polite',
              class: 'kw-library-loading',
            },
          },
          Spinner({ size: 'lg', label: 'Loading wallpaper catalog records' }),
          h(
            'p',
            { attrs: { class: 'kw-loading-text' } },
            'Loading registered wallpapers from local catalog...',
          ),
        )
      : state.status === 'error' && state.entries.length === 0
        ? AlertBanner({
            testId: 'library-error-state',
            tone: 'error',
            title: 'Unable to Load Wallpaper Library',
            message:
              state.errorMessage ??
              'An unexpected error occurred while reading the local wallpaper catalog.',
            actionButton: Button({
              testId: 'library-error-retry-btn',
              label: 'Retry Loading Catalog',
              variant: 'primary',
              size: 'sm',
              onClick: () => {
                void store.loadCatalog({ forceRefresh: true });
              },
            }),
          })
        : state.entries.length === 0
          ? EmptyStatePanel({
              testId: 'library-empty-state',
              heading: 'No Wallpapers in Your Library Yet',
              description:
                'Register a local video (.mp4, .webm, .avi, .mkv, .mov) or interactive web (.html) wallpaper to begin building your catalog.',
              primaryAction: Button({
                testId: 'library-empty-add-btn',
                label: state.registrationInFlight ? 'Adding Wallpaper...' : 'Add Wallpaper',
                variant: 'primary',
                size: 'md',
                isLoading: state.registrationInFlight,
                onClick: () => {
                  void store.triggerRegistration();
                },
              }),
            })
          : h(
              'div',
              { attrs: { class: 'kw-library-populated-layout' } },
              // Step 28 Discovery Toolbar: Search, Type Filter, Availability Filter, and Sort
              h(
                'div',
                {
                  attrs: {
                    'data-testid': 'library-discovery-toolbar',
                    role: 'toolbar',
                    'aria-label': 'Wallpaper search, filtering, and sorting controls',
                    class: 'kw-discovery-toolbar',
                  },
                },
                SearchInput({
                  id: 'library-search-field',
                  testId: 'library-search-input',
                  clearButtonTestId: 'library-search-clear-btn',
                  value: state.query.searchQuery,
                  placeholder: 'Search by title, author, description, or tag...',
                  ariaLabel: 'Search wallpapers by title, author, description, or tag',
                  onChange: (value) => store.setSearchQuery(value),
                  onClear: () => store.clearSearchQuery(),
                }),
                SegmentedFilterGroup({
                  testId: 'library-type-filter-group',
                  groupLabel: 'Filter by wallpaper type',
                  selectedValue: state.query.typeFilter,
                  options: TYPE_FILTER_OPTIONS,
                  onSelect: (value) => store.setTypeFilter(value),
                }),
                SegmentedFilterGroup({
                  testId: 'library-availability-filter-group',
                  groupLabel: 'Filter by asset availability',
                  selectedValue: state.query.availabilityFilter,
                  options: AVAILABILITY_FILTER_OPTIONS,
                  onSelect: (value) => store.setAvailabilityFilter(value),
                }),
                SortSelect({
                  id: 'library-sort-select',
                  testId: 'library-sort-select',
                  ariaLabel: 'Sort wallpapers',
                  selectedValue: state.query.sortOrder,
                  options: SORT_OPTIONS,
                  onChange: (value) => store.setSortOrder(value),
                }),
              ),
              // Step 28 Active Filter & Result Count Bar
              h(
                'div',
                {
                  attrs: {
                    'data-testid': 'library-results-bar',
                    class: 'kw-results-bar',
                  },
                },
                h(
                  'div',
                  {
                    attrs: {
                      'data-testid': 'library-results-count',
                      role: 'status',
                      'aria-live': 'polite',
                      class: 'kw-results-count',
                    },
                  },
                  queryResult.isRestricted
                    ? `Showing ${queryResult.matchingCount} of ${queryResult.totalCatalogCount} registered wallpaper${
                        queryResult.totalCatalogCount === 1 ? '' : 's'
                      }`
                    : `Showing all ${queryResult.totalCatalogCount} registered wallpaper${
                        queryResult.totalCatalogCount === 1 ? '' : 's'
                      }`,
                ),
                queryResult.isRestricted
                  ? h(
                      'div',
                      { attrs: { class: 'kw-active-filters-actions' } },
                      Badge({
                        testId: 'library-active-filter-indicator',
                        label: 'Filters Active',
                        iconSymbol: '◆',
                        tone: 'cyan',
                      }),
                      Button({
                        testId: 'library-clear-filters-btn',
                        label: 'Clear Search & Filters',
                        variant: 'ghost',
                        size: 'sm',
                        onClick: () => store.clearSearchAndFilters(),
                      }),
                    )
                  : null,
              ),
              // Step 28 §8.2: Distinct No-Matching-Results state vs Populated Grid
              queryResult.visibleEntries.length === 0
                ? EmptyStatePanel({
                    testId: 'library-no-matches-state',
                    heading: 'No Matching Wallpapers Found',
                    description:
                      'No registered wallpapers match your active search text or filter selections. Try relaxing your query or clearing active filters.',
                    primaryAction: Button({
                      testId: 'library-no-matches-clear-btn',
                      label: 'Clear Search & Filters',
                      variant: 'secondary',
                      size: 'md',
                      onClick: () => store.clearSearchAndFilters(),
                    }),
                  })
                : h(
                    'div',
                    {
                      attrs: {
                        'data-testid': 'library-catalog-grid',
                        'data-columns': String(gridMetrics.columns),
                        'data-gap-px': String(gridMetrics.gapPx),
                        role: 'region',
                        'aria-label': 'Registered Wallpapers Grid',
                        class: 'kw-library-grid',
                        style: `--kw-grid-cols: ${gridMetrics.columns}; --kw-grid-gap: ${gridMetrics.gapPx}px;`,
                      },
                    },
                    ...queryResult.visibleEntries.map((entry) =>
                      WallpaperCard({
                        entry,
                        isSelected: state.selectedWallpaperId === entry.id,
                        isFocusRestored: state.focusReturnWallpaperId === entry.id,
                        thumbnailFailed: state.failedThumbnailIds.has(entry.id),
                        onSelect: (id) => store.selectWallpaper(id),
                        onThumbnailError: (id) => store.markThumbnailFailed(id),
                      }),
                    ),
                  ),
              selectedEntry
                ? WallpaperDetailsPanel({
                    entry: selectedEntry,
                    thumbnailFailed: state.failedThumbnailIds.has(selectedEntry.id),
                    isHiddenByActiveQuery: isSelectedHiddenByFilter,
                    isEditingMetadata: state.editingWallpaperId === selectedEntry.id,
                    metadataDraft:
                      state.editingWallpaperId === selectedEntry.id ? state.metadataDraft : null,
                    isPendingRemoval: state.pendingRemovalWallpaperId === selectedEntry.id,
                    isActiveAssigned: state.activeWallpaperIds.has(selectedEntry.id),
                    isManagementInFlight: state.managementInFlight,
                    managementFeedback: state.managementFeedback,
                    onClose: () => store.closeDetailsPanel(),
                    onThumbnailError: (id) => store.markThumbnailFailed(id),
                    onStartEditMetadata: (id) => store.startMetadataEdit(id),
                    onUpdateMetadataDraft: (partial) => store.updateMetadataDraft(partial),
                    onSaveMetadata: () => {
                      void store.submitMetadataEdit();
                    },
                    onCancelEditMetadata: () => store.cancelMetadataEdit(),
                    onRelinkAsset: (id) => {
                      void store.relinkWallpaperAsset(id);
                    },
                    onRequestRemove: (id) => store.requestRemoveWallpaper(id),
                    onConfirmRemove: (id) => {
                      void store.confirmRemoveWallpaper(id);
                    },
                    onCancelRemove: () => store.cancelRemoveWallpaper(),
                    onDismissFeedback: () => store.dismissManagementFeedback(),
                  })
                : null,
            ),
  );
}

