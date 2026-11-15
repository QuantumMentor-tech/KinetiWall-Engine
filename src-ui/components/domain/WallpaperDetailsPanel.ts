/**
 * WallpaperDetailsPanel & Metadata Inspector (`Step 29`).
 *
 * Read-only details experience presenting verified `CatalogEntrySummary` metadata,
 * real thumbnail references (or deliberate non-alarming fallbacks), honest `AvailabilityState`
 * semantics, and technical/provenance fields without mutating source files or starting playback.
 */

import type {
  CatalogManagementFeedback,
  MetadataEditDraft,
} from '../../stores/catalogStore.ts';
import {
  describeAudioStatus,
  type AudioCaptureStatus,
  type AudioReactivitySettings,
} from '../../types/audio.ts';
import {
  formatCatalogTimestampUtc,
  resolveSafeThumbnailUrl,
  type CatalogEntrySummary,
  type WallpaperId,
} from '../../types/catalog.ts';
import { AlertBanner, Badge, Button, ConfirmationDialog } from '../primitives/controls.ts';
import { h, type UiNode } from '../primitives/uiNode.ts';
import {
  describeAvailability,
  describeSourceKind,
  describeWallpaperType,
} from './WallpaperCard.ts';

export interface WallpaperDetailsPanelProps {
  entry: CatalogEntrySummary;
  thumbnailFailed: boolean;
  isHiddenByActiveQuery: boolean;
  onClose: () => void;
  onThumbnailError: (id: WallpaperId) => void;
  isEditingMetadata?: boolean;
  metadataDraft?: MetadataEditDraft | null;
  isPendingRemoval?: boolean;
  isActiveAssigned?: boolean;
  isManagementInFlight?: boolean;
  managementFeedback?: CatalogManagementFeedback | null;
  onStartEditMetadata?: (id: WallpaperId) => void;
  onUpdateMetadataDraft?: (partial: Partial<MetadataEditDraft>) => void;
  onSaveMetadata?: () => void;
  onCancelEditMetadata?: () => void;
  onRelinkAsset?: (id: WallpaperId) => void;
  onRequestRemove?: (id: WallpaperId) => void;
  onConfirmRemove?: (id: WallpaperId) => void;
  onCancelRemove?: () => void;
  onDismissFeedback?: () => void;
  audioSettings?: AudioReactivitySettings;
  audioStatus?: AudioCaptureStatus;
  audioErrorDetail?: string | null;
  onToggleAudioReactivity?: (enabled: boolean) => void;
  onUpdateAudioSensitivity?: (val: number) => void;
  onUpdateAudioSmoothing?: (val: number) => void;
}

export function WallpaperDetailsPanel(props: WallpaperDetailsPanelProps): UiNode {
  const { entry, thumbnailFailed, isHiddenByActiveQuery } = props;
  const typeInfo = describeWallpaperType(entry.wallpaper_type);
  const availInfo = describeAvailability(entry.availability);
  const sourceLabel = describeSourceKind(entry.source_kind);
  const safeThumbnailUrl = !thumbnailFailed ? resolveSafeThumbnailUrl(entry) : null;

  const hasDescription =
    typeof entry.description === 'string' && entry.description.trim().length > 0;
  const createdFormatted = formatCatalogTimestampUtc(entry.created_at);
  const updatedFormatted = formatCatalogTimestampUtc(entry.updated_at);

  return h(
    'aside',
    {
      attrs: {
        role: 'region',
        'data-testid': 'library-selection-inspector',
        'data-selected-id': entry.id,
        'data-hidden-by-filter': isHiddenByActiveQuery ? 'true' : 'false',
        'aria-labelledby': 'details-panel-title',
        class: 'kw-library-inspector',
      },
      onKeyDown: (e) => {
        if (e.key === 'Escape') {
          e.preventDefault?.();
          props.onClose();
        }
      },
    },
    // 1. Identity Header & Close Action
    h(
      'div',
      { attrs: { class: 'kw-inspector-header' } },
      h(
        'div',
        { attrs: { class: 'kw-inspector-title-group' } },
        h('span', { attrs: { class: 'kw-inspector-eyebrow' } }, 'Selected Catalog Record'),
        h(
          'h2',
          {
            attrs: {
              id: 'details-panel-title',
              'data-testid': 'inspector-title',
              class: 'kw-inspector-title',
              title: entry.title,
            },
          },
          entry.title,
        ),
      ),
      Button({
        testId: 'inspector-clear-selection-btn',
        label: 'Close Details',
        ariaLabel: `Close details panel for ${entry.title}`,
        variant: 'ghost',
        size: 'sm',
        onClick: props.onClose,
      }),
    ),

    // Optional notice when active search/filter hides the selected card from the grid (Step 29 §5.4)
    isHiddenByActiveQuery
      ? AlertBanner({
          testId: 'inspector-hidden-by-filter-banner',
          tone: 'info',
          title: 'Filtered Out of Grid View',
          message:
            'This selected wallpaper does not match your current search or filter criteria, but remains open for inspection.',
        })
      : null,

    // Step 30 Management Feedback Banner
    props.managementFeedback
      ? AlertBanner({
          testId: 'inspector-management-feedback',
          tone: props.managementFeedback.kind,
          title:
            props.managementFeedback.kind === 'error'
              ? 'Catalog Management Error'
              : props.managementFeedback.kind === 'warning'
                ? 'Catalog Safeguard Notice'
                : 'Catalog Management Status',
          message: props.managementFeedback.message,
          dismissButton: props.onDismissFeedback
            ? Button({
                testId: 'inspector-dismiss-feedback-btn',
                label: 'Dismiss',
                variant: 'ghost',
                size: 'sm',
                onClick: props.onDismissFeedback,
              })
            : undefined,
        })
      : null,

    // 2. Visual Reference (16:9 Preview Area — Step 29 §7.1, §7.2, §7.3)
    h(
      'div',
      {
        attrs: {
          'data-testid': 'inspector-preview-container',
          class: 'kw-inspector-preview',
        },
      },
      safeThumbnailUrl
        ? h('img', {
            attrs: {
              src: safeThumbnailUrl,
              alt: `Preview image for ${entry.title}`,
              loading: 'lazy',
              decoding: 'async',
              width: 360,
              height: 202,
              'data-testid': 'inspector-preview-img',
              class: 'kw-inspector-preview-img',
            },
            onImageError: () => props.onThumbnailError(entry.id),
          })
        : h(
            'div',
            {
              attrs: {
                'data-testid': 'inspector-preview-placeholder',
                'data-reason': thumbnailFailed ? 'load_failed' : 'absent',
                role: 'img',
                'aria-label': thumbnailFailed
                  ? `Preview image could not be loaded for ${entry.title}`
                  : `No preview image referenced for ${entry.title}`,
                class: 'kw-inspector-preview-placeholder',
              },
            },
            h(
              'span',
              { attrs: { 'aria-hidden': 'true', class: 'kw-placeholder-icon' } },
              typeInfo.iconSymbol,
            ),
            h(
              'span',
              { attrs: { class: 'kw-placeholder-text' } },
              thumbnailFailed
                ? 'Preview Image Could Not Be Loaded'
                : 'No Preview Image Referenced',
            ),
          ),
    ),

    // 3. Type & Availability Status Section (Step 29 §6.3, §6.4)
    h(
      'section',
      {
        attrs: {
          'data-testid': 'inspector-status-section',
          'aria-label': 'Wallpaper type and asset availability',
          class: 'kw-inspector-section',
        },
      },
      h(
        'div',
        { attrs: { class: 'kw-inspector-badges' } },
        Badge({
          testId: 'inspector-type-badge',
          label: typeInfo.label,
          iconSymbol: typeInfo.iconSymbol,
          tone: typeInfo.tone,
          ariaLabel: `Wallpaper type: ${typeInfo.label}`,
        }),
        Badge({
          testId: 'inspector-availability-badge',
          label: availInfo.label,
          iconSymbol: availInfo.iconSymbol,
          tone: availInfo.tone,
          ariaLabel: `Asset availability: ${availInfo.label}`,
        }),
      ),
      h(
        'p',
        {
          attrs: {
            'data-testid': 'inspector-availability-detail',
            class: 'kw-inspector-availability-detail',
          },
        },
        availInfo.detailText,
      ),
      h(
        'p',
        {
          attrs: {
            'data-testid': 'inspector-playback-distinction',
            class: 'kw-inspector-subnote',
          },
        },
        'Asset availability reflects local file verification only and does not indicate active desktop assignment or live playback.',
      ),
    ),

    // 4. Description & Tags Section (Step 29 §6.1, §6.2)
    h(
      'section',
      {
        attrs: {
          'data-testid': 'inspector-descriptive-section',
          'aria-label': 'Description and tags',
          class: 'kw-inspector-section',
        },
      },
      h('h3', { attrs: { class: 'kw-inspector-section-heading' } }, 'Description'),
      hasDescription
        ? h(
            'p',
            {
              attrs: {
                'data-testid': 'inspector-description',
                'data-empty': 'false',
                class: 'kw-inspector-description',
              },
            },
            entry.description!.trim(),
          )
        : h(
            'p',
            {
              attrs: {
                'data-testid': 'inspector-description-empty',
                'data-empty': 'true',
                class: 'kw-inspector-empty-value',
              },
            },
            'No description provided in catalog metadata.',
          ),
      h('h3', { attrs: { class: 'kw-inspector-section-heading' } }, 'Tags'),
      entry.tags.length > 0
        ? h(
            'div',
            {
              attrs: {
                'data-testid': 'inspector-tags',
                class: 'kw-inspector-tags',
              },
            },
            ...entry.tags.map((tag) =>
              h('span', { key: tag, attrs: { class: 'kw-card-tag' } }, tag),
            ),
          )
        : h(
            'p',
            {
              attrs: {
                'data-testid': 'inspector-tags-empty',
                class: 'kw-inspector-empty-value',
              },
            },
            'No tags assigned.',
          ),
    ),

    // 5. Technical, Source & Timestamp Metadata (Step 29 §6.5, §6.6)
    h(
      'section',
      {
        attrs: {
          'data-testid': 'inspector-technical-section',
          'aria-label': 'Catalog metadata and provenance',
          class: 'kw-inspector-section',
        },
      },
      h('h3', { attrs: { class: 'kw-inspector-section-heading' } }, 'Catalog Metadata'),
      h(
        'dl',
        {
          attrs: {
            'data-testid': 'inspector-technical-metadata',
            class: 'kw-inspector-details',
          },
        },
        h('dt', null, 'Author'),
        h('dd', { attrs: { 'data-testid': 'inspector-author' } }, entry.author),
        h('dt', null, 'Canonical Type'),
        h(
          'dd',
          { attrs: { 'data-testid': 'inspector-canonical-type' } },
          typeInfo.isSupported ? entry.wallpaper_type : `unsupported (${entry.wallpaper_type})`,
        ),
        h('dt', null, 'Storage Model'),
        h('dd', { attrs: { 'data-testid': 'inspector-source-kind' } }, sourceLabel),
        h('dt', null, 'Catalog Origin'),
        h(
          'dd',
          { attrs: { 'data-testid': 'inspector-origin' } },
          entry.is_builtin ? 'Built-in Package' : 'User Registered',
        ),
        h('dt', null, 'Package Version'),
        h(
          'dd',
          { attrs: { 'data-testid': 'inspector-version' } },
          entry.version ?? '1.0.0',
        ),
        h('dt', null, 'Min Engine Version'),
        h(
          'dd',
          { attrs: { 'data-testid': 'inspector-min-engine' } },
          entry.min_engine_version ?? '1.0.0',
        ),
        h('dt', null, 'License'),
        h(
          'dd',
          { attrs: { 'data-testid': 'inspector-license' } },
          entry.license ?? 'Custom',
        ),
        h('dt', null, 'Manifest Schema'),
        h(
          'dd',
          { attrs: { 'data-testid': 'inspector-schema-version' } },
          `v${entry.schema_version ?? 1}`,
        ),
        h('dt', null, 'Catalog Registered'),
        h('dd', { attrs: { 'data-testid': 'inspector-created-at' } }, createdFormatted),
        h('dt', null, 'Metadata Updated'),
        h('dd', { attrs: { 'data-testid': 'inspector-updated-at' } }, updatedFormatted),
        h('dt', null, 'Catalog Record ID'),
        h('dd', { attrs: { 'data-testid': 'inspector-record-id' } }, entry.id),
      ),
    ),

    // 6. Step 30 Catalog Management Actions, Inline Metadata Editor & Safe Removal Dialog
    h(
      'section',
      {
        attrs: {
          'data-testid': 'inspector-management-section',
          'aria-label': 'Catalog management actions',
          class: 'kw-inspector-section kw-inspector-management',
        },
      },
      h('h3', { attrs: { class: 'kw-inspector-section-heading' } }, 'Catalog Management'),
      h(
        'div',
        {
          attrs: {
            'data-testid': 'inspector-management-actions',
            class: 'kw-inspector-management-actions',
          },
        },
        Button({
          testId: 'inspector-edit-metadata-btn',
          label: props.isEditingMetadata ? 'Editing Metadata...' : 'Edit Metadata',
          ariaLabel: `Edit metadata for ${entry.title}`,
          variant: 'secondary',
          size: 'sm',
          disabled:
            Boolean(entry.is_builtin) ||
            Boolean(props.isManagementInFlight) ||
            Boolean(props.isEditingMetadata),
          onClick: () => props.onStartEditMetadata?.(entry.id),
        }),
        Button({
          testId: 'inspector-relink-asset-btn',
          label:
            entry.availability.status === 'available' ? 'Relink Asset' : 'Locate & Relink Asset',
          ariaLabel: `Locate and relink local asset for ${entry.title}`,
          variant: entry.availability.status === 'available' ? 'secondary' : 'primary',
          size: 'sm',
          disabled:
            entry.source_kind === 'managed_package' || Boolean(props.isManagementInFlight),
          onClick: () => props.onRelinkAsset?.(entry.id),
        }),
        Button({
          testId: 'inspector-remove-entry-btn',
          label: 'Remove from Library',
          ariaLabel: `Remove ${entry.title} from KinetiWall library without deleting source media`,
          variant: 'danger',
          size: 'sm',
          disabled: Boolean(entry.is_builtin) || Boolean(props.isManagementInFlight),
          onClick: () => props.onRequestRemove?.(entry.id),
        }),
      ),

      props.isActiveAssigned
        ? h(
            'p',
            {
              attrs: {
                'data-testid': 'inspector-active-assignment-note',
                class: 'kw-inspector-subnote',
              },
            },
            'This wallpaper is currently active on a desktop monitor. Unassign or stop it before removing it from the library.',
          )
        : null,

      // Audio Reactivity Section (Step 34 §5.1, §5.2)
      entry.wallpaper_type === 'web'
        ? h(
            'section',
            {
              attrs: {
                'data-testid': 'inspector-audio-section',
                'aria-label': 'Audio reactivity controls and status',
                class: 'kw-inspector-section kw-inspector-audio',
              },
            },
            h('h3', { attrs: { class: 'kw-inspector-section-heading' } }, 'Audio Reactivity'),
            h(
              'p',
              {
                attrs: {
                  'data-testid': 'inspector-audio-disclosure',
                  class: 'kw-inspector-subnote',
                },
              },
              'Observe system playback audio using WASAPI loopback to animate compatible web wallpapers. Audio capture is strictly opt-in, processed entirely locally, and never recorded or uploaded.',
            ),
            h(
              'div',
              { attrs: { class: 'kw-inspector-badges', style: 'margin: 8px 0;' } },
              Badge({
                testId: 'inspector-audio-status-badge',
                label: describeAudioStatus(props.audioStatus ?? 'disabled', props.audioErrorDetail).label,
                tone: describeAudioStatus(props.audioStatus ?? 'disabled', props.audioErrorDetail).variant,
                ariaLabel: `Audio reactivity status: ${
                  describeAudioStatus(props.audioStatus ?? 'disabled', props.audioErrorDetail).label
                }`,
              }),
            ),
            h(
              'div',
              { attrs: { class: 'kw-inspector-management-actions' } },
              Button({
                testId: 'inspector-audio-toggle-btn',
                label: props.audioSettings?.enabled
                  ? 'Disable Audio Reactivity'
                  : 'Enable Audio Reactivity',
                ariaLabel: props.audioSettings?.enabled
                  ? 'Disable audio capture reactivity'
                  : 'Enable opt-in audio capture reactivity',
                variant: props.audioSettings?.enabled ? 'danger' : 'primary',
                size: 'sm',
                onClick: () => props.onToggleAudioReactivity?.(!props.audioSettings?.enabled),
              }),
            ),
          )
        : null,

      // Inline Metadata Editor (Step 30 §5)
      props.isEditingMetadata && props.metadataDraft
        ? h(
            'form',
            {
              attrs: {
                'data-testid': 'inspector-metadata-edit-form',
                'aria-label': `Edit metadata for ${entry.title}`,
                class: 'kw-inspector-edit-form',
              },
            },
            h(
              'label',
              { attrs: { for: 'inspector-edit-title', class: 'kw-form-label' } },
              'Title (required, max 120 chars)',
            ),
            h('input', {
              attrs: {
                id: 'inspector-edit-title',
                type: 'text',
                'data-testid': 'inspector-edit-title-input',
                value: props.metadataDraft.title,
                maxlength: 120,
                disabled: Boolean(props.isManagementInFlight),
                class: 'kw-form-input kw-focus-ring',
              },
              onInput: (val) => props.onUpdateMetadataDraft?.({ title: val }),
              onChange: (val) => props.onUpdateMetadataDraft?.({ title: val }),
            }),
            h(
              'label',
              { attrs: { for: 'inspector-edit-description', class: 'kw-form-label' } },
              'Description (optional, max 2000 chars)',
            ),
            h('textarea', {
              attrs: {
                id: 'inspector-edit-description',
                'data-testid': 'inspector-edit-description-input',
                value: props.metadataDraft.description,
                maxlength: 2000,
                disabled: Boolean(props.isManagementInFlight),
                class: 'kw-form-textarea kw-focus-ring',
              },
              onInput: (val) => props.onUpdateMetadataDraft?.({ description: val }),
              onChange: (val) => props.onUpdateMetadataDraft?.({ description: val }),
            }),
            h(
              'label',
              { attrs: { for: 'inspector-edit-tags', class: 'kw-form-label' } },
              'Tags (comma-separated, max 32 tags)',
            ),
            h('input', {
              attrs: {
                id: 'inspector-edit-tags',
                type: 'text',
                'data-testid': 'inspector-edit-tags-input',
                value: props.metadataDraft.tagsInput,
                disabled: Boolean(props.isManagementInFlight),
                class: 'kw-form-input kw-focus-ring',
              },
              onInput: (val) => props.onUpdateMetadataDraft?.({ tagsInput: val }),
              onChange: (val) => props.onUpdateMetadataDraft?.({ tagsInput: val }),
            }),
            h(
              'p',
              {
                attrs: {
                  'data-testid': 'inspector-edit-protected-note',
                  class: 'kw-inspector-subnote',
                },
              },
              'Protected fields (Record ID, Canonical Type, Storage Model, Created Timestamp, and Availability) cannot be modified through metadata editing.',
            ),
            h(
              'div',
              { attrs: { class: 'kw-inspector-edit-actions' } },
              Button({
                testId: 'inspector-cancel-metadata-btn',
                label: 'Cancel',
                variant: 'secondary',
                size: 'sm',
                disabled: Boolean(props.isManagementInFlight),
                onClick: () => props.onCancelEditMetadata?.(),
              }),
              Button({
                testId: 'inspector-save-metadata-btn',
                label: props.isManagementInFlight ? 'Saving...' : 'Save Metadata',
                variant: 'primary',
                size: 'sm',
                isLoading: Boolean(props.isManagementInFlight),
                onClick: () => props.onSaveMetadata?.(),
              }),
            ),
          )
        : null,

      // Catalog-Only Removal Confirmation Dialog (Step 30 §7.3, §7.4)
      props.isPendingRemoval
        ? ConfirmationDialog({
            testId: 'confirm-remove-dialog',
            title: `Remove "${entry.title}" from Library?`,
            description:
              'This removes only the catalog record from your KinetiWall library.',
            safetyNotice:
              'Original wallpaper media files on disk are NOT deleted, moved, or modified.',
            confirmLabel: 'Remove from Library',
            cancelLabel: 'Cancel',
            isConfirmLoading: Boolean(props.isManagementInFlight),
            onConfirm: () => props.onConfirmRemove?.(entry.id),
            onCancel: () => props.onCancelRemove?.(),
          })
        : null,
    ),

    // 7. Read-Only Boundary Notice (Step 29 §9.1, §9.3)
    h(
      'p',
      {
        attrs: {
          'data-testid': 'inspector-boundary-notice',
          class: 'kw-inspector-notice',
        },
      },
      'Selection highlights this catalog record for inspection only and does not start playback or change display assignments.',
    ),
  );
}

