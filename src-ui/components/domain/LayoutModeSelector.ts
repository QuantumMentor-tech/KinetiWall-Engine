/**
 * Multi-Monitor Presentation Layout Mode Selector Component (Step 46).
 * Presents Independent, Span, and Clone layout options with capability warnings,
 * single-display safeguards, and honest playback synchronization descriptions.
 */

import { Badge } from '../primitives/controls.ts';
import { h, type UiNode } from '../primitives/uiNode.ts';
import type { MultiMonitorLayoutMode } from '../../types/display.ts';

export interface LayoutModeSelectorProps {
  currentMode: MultiMonitorLayoutMode;
  activeMode?: MultiMonitorLayoutMode | null;
  displayCount: number;
  disabled?: boolean;
  onSelectMode: (mode: MultiMonitorLayoutMode) => void;
}

export function LayoutModeSelector(props: LayoutModeSelectorProps): UiNode {
  const { currentMode, activeMode, displayCount, disabled, onSelectMode } = props;
  const isMultiDisplay = displayCount > 1;

  const modes: Array<{
    id: MultiMonitorLayoutMode;
    label: string;
    description: string;
    requiresMulti: boolean;
    syncNote?: string;
  }> = [
    {
      id: 'independent',
      label: 'Independent',
      description: 'Assign and render individual wallpapers on each monitor independently.',
      requiresMulti: false,
    },
    {
      id: 'span',
      label: 'Span Virtual Desktop',
      description: 'Stretch a unified wide wallpaper across all monitors in virtual desktop geometry.',
      requiresMulti: true,
    },
    {
      id: 'clone',
      label: 'Clone / Duplicate',
      description: 'Mirror the primary wallpaper across all monitors with best-effort software sync.',
      requiresMulti: true,
      syncNote: 'Best-effort software synchronization (hardware genlock not claimed).',
    },
  ];

  return h(
    'section',
    {
      attrs: {
        'data-testid': 'layout-mode-selector',
        class: 'kw-layout-mode-selector',
        role: 'region',
        'aria-label': 'Multi-Monitor Presentation Layout',
      },
    },
    h(
      'div',
      { attrs: { class: 'kw-layout-selector-header' } },
      h('h3', { attrs: { class: 'kw-section-title' } }, 'Presentation Layout Mode'),
      activeMode && activeMode !== currentMode
        ? Badge({
            testId: 'layout-pending-apply-badge',
            label: 'Layout change pending apply',
            iconSymbol: '!',
            tone: 'warning',
          })
        : null,
    ),
    h(
      'div',
      {
        attrs: {
          class: 'kw-layout-mode-pills',
          role: 'radiogroup',
          'aria-label': 'Select presentation layout mode',
        },
      },
      ...modes.map((mode) => {
        const isSelected = currentMode === mode.id;
        const isModeDisabled = Boolean(disabled || (mode.requiresMulti && !isMultiDisplay));

        let disabledReason: string | undefined;
        if (mode.requiresMulti && !isMultiDisplay) {
          disabledReason = 'Requires 2 or more connected displays';
        }

        return h(
          'div',
          {
            key: mode.id,
            attrs: {
              'data-testid': `layout-mode-option-${mode.id}`,
              'data-selected': isSelected ? 'true' : 'false',
              'data-disabled': isModeDisabled ? 'true' : 'false',
              class: `kw-layout-mode-pill${isSelected ? ' kw-layout-mode-pill-selected' : ''}${
                isModeDisabled ? ' kw-layout-mode-pill-disabled' : ''
              }`,
              role: 'radio',
              'aria-checked': isSelected ? 'true' : 'false',
              'aria-disabled': isModeDisabled ? 'true' : undefined,
              tabindex: isModeDisabled ? '-1' : isSelected ? '0' : '-1',
              title: disabledReason ?? mode.description,
            },
            onClick: isModeDisabled ? undefined : () => onSelectMode(mode.id),
            onKeyDown: isModeDisabled
              ? undefined
              : (e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault?.();
                    onSelectMode(mode.id);
                  }
                },
          },
          h(
            'div',
            { attrs: { class: 'kw-pill-main-row' } },
            h('span', { attrs: { class: 'kw-pill-title' } }, mode.label),
            isSelected
              ? h('span', { attrs: { class: 'kw-pill-check', 'aria-hidden': 'true' } }, '✓')
              : null,
          ),
          h('p', { attrs: { class: 'kw-pill-desc' } }, mode.description),
          mode.syncNote
            ? h('span', { attrs: { class: 'kw-pill-sync-note' } }, `ℹ ${mode.syncNote}`)
            : null,
          disabledReason
            ? h('span', { attrs: { class: 'kw-pill-unsupported' } }, `⚠ ${disabledReason}`)
            : null,
        );
      }),
    ),
  );
}

