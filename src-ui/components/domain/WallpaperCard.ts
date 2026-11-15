/**
 * WallpaperCard Domain Component (`Step 21 KW-UI-002` & `Step 27 §7, §9`).
 *
 * Architectural Guarantees:
 * - Uses stable `entry.id` (`WallpaperId`) as the rendering key and selection identity.
 * - Renders real thumbnail references via sandboxed asset URL when present, or a deliberate
 *   neutral placeholder when absent or when image loading fails.
 * - Represents `AvailabilityState` truthfully using icon + text + semantic tone (never color alone).
 * - Never displays a false "Playing" badge or triggers playback/desktop attachment on selection.
 * - Never exposes raw host filesystem paths (`C:\...` or `\\server\...`).
 */

import {
  isKnownWallpaperType,
  resolveSafeThumbnailUrl,
  type AvailabilityState,
  type CatalogEntrySummary,
  type WallpaperId,
} from '../../types/catalog.ts';
import { Badge, type BadgeTone } from '../primitives/controls.ts';
import { h, type UiNode } from '../primitives/uiNode.ts';

export interface AvailabilityPresentation {
  label: string;
  iconSymbol: string;
  tone: BadgeTone;
  detailText: string;
  isPlayableCandidate: boolean;
}

export function describeAvailability(availability: AvailabilityState): AvailabilityPresentation {
  switch (availability.status) {
    case 'available':
      return {
        label: 'Available',
        iconSymbol: '●',
        tone: 'available',
        detailText: 'Source asset verified present and readable on disk.',
        isPlayableCandidate: true,
      };
    case 'missing':
      return {
        label: 'File Missing',
        iconSymbol: '▲',
        tone: 'warning',
        detailText: 'Referenced media file could not be found at its registered location.',
        isPlayableCandidate: false,
      };
    case 'inaccessible':
      return {
        label: 'Inaccessible',
        iconSymbol: '▲',
        tone: 'warning',
        detailText: availability.reason || 'Source file is locked or lacks read permission.',
        isPlayableCandidate: false,
      };
    case 'invalid_asset':
      return {
        label: 'Invalid Asset',
        iconSymbol: '✕',
        tone: 'error',
        detailText: availability.reason || 'Source path failed asset validation checks.',
        isPlayableCandidate: false,
      };
    case 'unchecked':
    default:
      return {
        label: 'Unverified',
        iconSymbol: '○',
        tone: 'neutral',
        detailText: 'Catalog record loaded; file availability has not been verified in this session.',
        isPlayableCandidate: false,
      };
  }
}

export function describeWallpaperType(rawType: string): {
  label: string;
  iconSymbol: string;
  tone: BadgeTone;
  isSupported: boolean;
} {
  if (!isKnownWallpaperType(rawType)) {
    return {
      label: 'Unsupported Type',
      iconSymbol: '?',
      tone: 'warning',
      isSupported: false,
    };
  }
  if (rawType === 'video') {
    return {
      label: 'Video',
      iconSymbol: '▶',
      tone: 'cyan',
      isSupported: true,
    };
  }
  return {
    label: 'Web',
    iconSymbol: '◆',
    tone: 'cyan',
    isSupported: true,
  };
}

export function describeSourceKind(sourceKind: CatalogEntrySummary['source_kind']): string {
  return sourceKind === 'external_direct_file' ? 'Local Reference' : 'Managed Package';
}

export interface WallpaperCardProps {
  entry: CatalogEntrySummary;
  isSelected: boolean;
  isFocusRestored?: boolean;
  thumbnailFailed: boolean;
  onSelect: (id: WallpaperId) => void;
  onThumbnailError: (id: WallpaperId) => void;
}

export function WallpaperCard(props: WallpaperCardProps): UiNode {
  const { entry, isSelected, isFocusRestored, thumbnailFailed } = props;
  const typeInfo = describeWallpaperType(entry.wallpaper_type);
  const availInfo = describeAvailability(entry.availability);
  const sourceLabel = describeSourceKind(entry.source_kind);
  const safeThumbnailUrl = !thumbnailFailed ? resolveSafeThumbnailUrl(entry) : null;

  const visibleTags = entry.tags.slice(0, 3);
  const extraTagCount = Math.max(0, entry.tags.length - visibleTags.length);
  const conciseDescription =
    typeof entry.description === 'string' && entry.description.trim().length > 0
      ? entry.description.trim()
      : null;

  const accessibleCardSummary = `${entry.title}, ${typeInfo.label} wallpaper, status ${availInfo.label}${
    isSelected ? ', selected for inspection' : ''
  }`;

  return h(
    'article',
    {
      key: entry.id,
      attrs: {
        'data-testid': `wallpaper-card-${entry.id}`,
        'data-wallpaper-id': entry.id,
        'data-selected': isSelected ? 'true' : 'false',
        'data-availability': entry.availability.status,
        'data-wallpaper-type': typeInfo.isSupported ? entry.wallpaper_type : 'unsupported',
        class: 'kw-wallpaper-card',
        'aria-label': `${entry.title} catalog card`,
      },
    },
    h(
      'button',
      {
        attrs: {
          type: 'button',
          'data-testid': `wallpaper-card-select-${entry.id}`,
          'data-focus-restored': isFocusRestored ? 'true' : undefined,
          class: 'kw-wallpaper-card-trigger kw-focus-ring',
          'aria-pressed': isSelected ? 'true' : 'false',
          'aria-label': accessibleCardSummary,
        },
        onClick: () => props.onSelect(entry.id),
        onKeyDown: (e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault?.();
            props.onSelect(entry.id);
          }
        },
      },
      // 16:9 Preview Area (Real thumbnail or deliberate neutral placeholder)
      h(
        'div',
        {
          attrs: {
            'data-testid': `wallpaper-card-preview-${entry.id}`,
            class: 'kw-card-preview',
          },
        },
        safeThumbnailUrl
          ? h('img', {
              attrs: {
                src: safeThumbnailUrl,
                alt: `Preview thumbnail for ${entry.title}`,
                loading: 'lazy',
                decoding: 'async',
                width: 320,
                height: 180,
                'data-testid': `wallpaper-card-img-${entry.id}`,
                class: 'kw-card-preview-img',
              },
              onImageError: () => props.onThumbnailError(entry.id),
            })
          : h(
              'div',
              {
                attrs: {
                  'data-testid': `wallpaper-card-placeholder-${entry.id}`,
                  role: 'img',
                  'aria-label': `No preview image available for ${entry.title}`,
                  class: 'kw-card-preview-placeholder',
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
                'No Preview Available',
              ),
            ),
        // Top-right badge overlay: Wallpaper Type + Availability
        h(
          'div',
          { attrs: { class: 'kw-card-badges' } },
          Badge({
            testId: `wallpaper-type-badge-${entry.id}`,
            label: typeInfo.label,
            iconSymbol: typeInfo.iconSymbol,
            tone: typeInfo.tone,
            ariaLabel: `Wallpaper type: ${typeInfo.label}`,
          }),
          Badge({
            testId: `wallpaper-availability-badge-${entry.id}`,
            label: availInfo.label,
            iconSymbol: availInfo.iconSymbol,
            tone: availInfo.tone,
            ariaLabel: `Asset status: ${availInfo.label}`,
            title: availInfo.detailText,
          }),
        ),
      ),
      // Card Body: Title, Author/Source, Optional Description, and Tags
      h(
        'div',
        { attrs: { class: 'kw-card-body' } },
        h(
          'h3',
          {
            attrs: {
              'data-testid': `wallpaper-card-title-${entry.id}`,
              class: 'kw-card-title',
              title: entry.title,
            },
          },
          entry.title,
        ),
        h(
          'div',
          { attrs: { class: 'kw-card-meta-row' } },
          h(
            'span',
            {
              attrs: {
                'data-testid': `wallpaper-card-author-${entry.id}`,
                class: 'kw-card-author',
              },
            },
            `By ${entry.author}`,
          ),
          h('span', { attrs: { 'aria-hidden': 'true', class: 'kw-meta-sep' } }, '•'),
          h(
            'span',
            {
              attrs: {
                'data-testid': `wallpaper-card-source-${entry.id}`,
                class: 'kw-card-source',
              },
            },
            sourceLabel,
          ),
        ),
        conciseDescription
          ? h(
              'p',
              {
                attrs: {
                  'data-testid': `wallpaper-card-description-${entry.id}`,
                  class: 'kw-card-description',
                },
              },
              conciseDescription,
            )
          : null,
        visibleTags.length > 0
          ? h(
              'div',
              {
                attrs: {
                  'data-testid': `wallpaper-card-tags-${entry.id}`,
                  class: 'kw-card-tags',
                },
              },
              ...visibleTags.map((tag) =>
                h('span', { key: tag, attrs: { class: 'kw-card-tag' } }, tag),
              ),
              extraTagCount > 0
                ? h(
                    'span',
                    { attrs: { class: 'kw-card-tag-overflow' } },
                    `+${extraTagCount}`,
                  )
                : null,
            )
          : null,
      ),
    ),
  );
}

