/**
 * Wallpaper Picker Dialog Modal Component (Step 46).
 * Accessible dialog allowing selection of registered catalog wallpapers
 * for assignment to a targeted display.
 */

import { Badge, Button } from '../primitives/controls.ts';
import { h, type UiNode } from '../primitives/uiNode.ts';
import type { CatalogEntrySummary } from '../../types/catalog.ts';
import type { DisplayDescriptorDto } from '../../types/display.ts';
import { trapFocus } from '../../utils/focusManager.ts';

export interface WallpaperPickerModalProps {
  isOpen: boolean;
  targetDisplay: DisplayDescriptorDto | null;
  wallpapers: CatalogEntrySummary[];
  isAssigning: boolean;
  onSelectWallpaper: (wallpaperId: string) => void;
  onClose: () => void;
}

export function WallpaperPickerModal(props: WallpaperPickerModalProps): UiNode | null {
  const { isOpen, targetDisplay, wallpapers, isAssigning, onSelectWallpaper, onClose } = props;

  if (!isOpen || !targetDisplay) {
    return null;
  }

  const targetName = targetDisplay.friendlyName || targetDisplay.deviceName || 'Display';

  return h(
    'div',
    {
      attrs: {
        'data-testid': 'wallpaper-picker-backdrop',
        class: 'kw-modal-backdrop',
        role: 'presentation',
      },
      onClick: (e) => {
        // Close if backdrop clicked directly
        if ((e.target as any)?.classList?.contains('kw-modal-backdrop')) {
          onClose();
        }
      },
    },
    h(
      'div',
      {
        attrs: {
          'data-testid': 'wallpaper-picker-dialog',
          class: 'kw-modal-dialog kw-picker-modal',
          role: 'dialog',
          'aria-modal': 'true',
          'aria-labelledby': 'wallpaper-picker-title',
        },
        onKeyDown: (e) => {
          if (e.key === 'Escape' && !isAssigning) {
            e.preventDefault?.();
            onClose();
          } else if (e.key === 'Tab' && e.currentTarget instanceof HTMLElement) {
            trapFocus(e.currentTarget, e);
          }
        },
      },
      // Modal Header
      h(
        'div',
        { attrs: { class: 'kw-modal-header' } },
        h(
          'div',
          { attrs: { class: 'kw-modal-header-titles' } },
          h('h3', { attrs: { id: 'wallpaper-picker-title', class: 'kw-modal-title' } }, 'Assign Wallpaper'),
          h(
            'p',
            { attrs: { class: 'kw-modal-subtitle' } },
            `Choose a wallpaper for ${targetName}${targetDisplay.isPrimary ? ' (Primary Display)' : ''}`,
          ),
        ),
        Button({
          testId: 'wallpaper-picker-close-btn',
          label: '✕',
          ariaLabel: 'Close dialog',
          variant: 'ghost',
          size: 'sm',
          onClick: onClose,
        }),
      ),
      // Wallpapers List / Grid
      h(
        'div',
        {
          attrs: {
            'data-testid': 'wallpaper-picker-list',
            class: 'kw-picker-list',
            role: 'list',
          },
        },
        ...(wallpapers.length === 0
          ? [
              h(
                'div',
                { attrs: { class: 'kw-picker-empty', 'data-testid': 'wallpaper-picker-empty' } },
                h('p', null, 'No wallpapers found in library.'),
              ),
            ]
          : wallpapers.map((wallpaper) => {
              const isAvailable = wallpaper.availability.status === 'available';
              const typeLabel = wallpaper.wallpaper_type === 'video' ? 'Video' : 'Web';

              return h(
                'div',
                {
                  key: wallpaper.id,
                  attrs: {
                    'data-testid': `picker-item-${wallpaper.id}`,
                    class: `kw-picker-item${!isAvailable ? ' kw-picker-item-unavailable' : ''}`,
                    role: 'listitem',
                  },
                },
                h(
                  'div',
                  { attrs: { class: 'kw-picker-item-info' } },
                  h('span', { attrs: { class: 'kw-picker-item-title' } }, wallpaper.title),
                  h(
                    'div',
                    { attrs: { class: 'kw-picker-item-meta' } },
                    Badge({
                      label: typeLabel,
                      iconSymbol: '▶',
                      tone: 'cyan',
                    }),
                    !isAvailable
                      ? Badge({
                          label: 'Unavailable',
                          iconSymbol: '⚠',
                          tone: 'warning',
                        })
                      : null,
                    h('span', { attrs: { class: 'kw-picker-item-author' } }, `by ${wallpaper.author}`),
                  ),
                ),
                Button({
                  testId: `picker-select-btn-${wallpaper.id}`,
                  label: 'Assign',
                  variant: 'primary',
                  size: 'sm',
                  disabled: !isAvailable || isAssigning,
                  onClick: () => onSelectWallpaper(wallpaper.id),
                }),
              );
            })),
      ),
      // Modal Footer
      h(
        'div',
        { attrs: { class: 'kw-modal-footer' } },
        Button({
          testId: 'wallpaper-picker-cancel-btn',
          label: 'Cancel',
          variant: 'secondary',
          size: 'md',
          disabled: isAssigning,
          onClick: onClose,
        }),
      ),
    ),
  );
}
