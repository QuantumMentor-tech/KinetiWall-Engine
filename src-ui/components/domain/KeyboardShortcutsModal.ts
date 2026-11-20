/**
 * Keyboard Shortcuts Reference Dialog (Step 64).
 * Accessible, discoverable cheat sheet showing registered commands and their key combos.
 */

import { Button } from '../primitives/controls.ts';
import { h, type UiNode } from '../primitives/uiNode.ts';
import type { CommandRegistry } from '../../services/commandRegistry.ts';
import {
  formatShortcutKeyCombo,
  type CommandCategory,
} from '../../types/shortcuts.ts';
import { trapFocus } from '../../utils/focusManager.ts';

export interface KeyboardShortcutsModalProps {
  isOpen: boolean;
  registry: CommandRegistry;
  onClose: () => void;
}

const CATEGORY_TITLES: Record<CommandCategory, string> = {
  navigation: 'Navigation',
  library: 'Wallpaper Library & Search',
  displays: 'Displays & Playback',
  settings: 'Settings & Preferences',
  general: 'General & Dialogs',
};

export function KeyboardShortcutsModal(props: KeyboardShortcutsModalProps): UiNode | null {
  const { isOpen, registry, onClose } = props;

  if (!isOpen) {
    return null;
  }

  const allCommands = registry.getAllCommands();
  const categories: CommandCategory[] = [
    'navigation',
    'library',
    'displays',
    'settings',
    'general',
  ];

  return h(
    'div',
    {
      attrs: {
        'data-testid': 'shortcuts-modal-backdrop',
        class: 'kw-modal-backdrop',
        role: 'presentation',
      },
      onClick: (e) => {
        if ((e.target as any)?.classList?.contains('kw-modal-backdrop')) {
          onClose();
        }
      },
    },
    h(
      'div',
      {
        attrs: {
          'data-testid': 'shortcuts-modal-dialog',
          class: 'kw-modal-dialog kw-shortcuts-modal',
          role: 'dialog',
          'aria-modal': 'true',
          'aria-labelledby': 'shortcuts-modal-title',
          'aria-describedby': 'shortcuts-modal-desc',
        },
        onKeyDown: (e) => {
          if (e.key === 'Escape') {
            e.preventDefault?.();
            onClose();
          } else if (e.key === 'Tab' && e.currentTarget instanceof HTMLElement) {
            trapFocus(e.currentTarget, e);
          }
        },
      },
      // Header
      h(
        'div',
        { attrs: { class: 'kw-modal-header' } },
        h(
          'div',
          { attrs: { class: 'kw-modal-header-titles' } },
          h('h3', { attrs: { id: 'shortcuts-modal-title', class: 'kw-modal-title' } }, 'Keyboard Shortcuts'),
          h(
            'p',
            { attrs: { id: 'shortcuts-modal-desc', class: 'kw-modal-subtitle' } },
            'Standard keyboard shortcuts for fast, accessible desktop navigation',
          ),
        ),
        Button({
          testId: 'shortcuts-modal-close-btn',
          label: '✕',
          ariaLabel: 'Close keyboard shortcuts dialog',
          variant: 'ghost',
          size: 'sm',
          onClick: onClose,
        }),
      ),
      // Body: Categorized Commands List
      h(
        'div',
        {
          attrs: {
            'data-testid': 'shortcuts-list-container',
            class: 'kw-shortcuts-list-container',
          },
        },
        ...categories.map((category) => {
          const commands = allCommands.filter((cmd) => cmd.category === category);
          if (commands.length === 0) return null;

          return h(
            'section',
            {
              key: category,
              attrs: {
                class: 'kw-shortcuts-section',
                'data-testid': `shortcuts-section-${category}`,
              },
            },
            h('h4', { attrs: { class: 'kw-shortcuts-section-title' } }, CATEGORY_TITLES[category]),
            h(
              'div',
              { attrs: { class: 'kw-shortcuts-table', role: 'table', 'aria-label': CATEGORY_TITLES[category] } },
              ...commands.map((cmd) => {
                const combo = cmd.shortcut ?? cmd.defaultShortcut;
                const formatted = formatShortcutKeyCombo(combo);
                const keys = formatted.split('+');

                return h(
                  'div',
                  {
                    key: cmd.id,
                    attrs: {
                      class: 'kw-shortcut-row',
                      role: 'row',
                      'data-testid': `shortcut-row-${cmd.id}`,
                    },
                  },
                  h(
                    'div',
                    { attrs: { class: 'kw-shortcut-info', role: 'cell' } },
                    h('span', { attrs: { class: 'kw-shortcut-label' } }, cmd.label),
                    h('span', { attrs: { class: 'kw-shortcut-desc' } }, cmd.description),
                  ),
                  h(
                    'div',
                    { attrs: { class: 'kw-shortcut-keys', role: 'cell' } },
                    ...keys.map((k, idx) =>
                      h(
                        'span',
                        { key: `${k}-${idx}`, attrs: { class: 'kw-kbd-group' } },
                        h('kbd', { attrs: { class: 'kw-kbd' } }, k),
                        idx < keys.length - 1 ? h('span', { attrs: { class: 'kw-kbd-plus' } }, '+') : null,
                      ),
                    ),
                  ),
                );
              }),
            ),
          );
        }),
      ),
      // Footer
      h(
        'div',
        { attrs: { class: 'kw-modal-footer' } },
        Button({
          testId: 'shortcuts-modal-done-btn',
          label: 'Got it',
          variant: 'primary',
          size: 'md',
          onClick: onClose,
        }),
      ),
    ),
  );
}
