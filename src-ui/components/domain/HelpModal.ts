/**
 * Help & Troubleshooting Modal Component
 *
 * Provides searchable, categorized in-app guidance, diagnostic resolution,
 * and safe quick actions.
 */

import { h, type UiNode } from '../primitives/uiNode.ts';
import { Button, SearchInput } from '../primitives/controls.ts';
import { trapFocus } from '../../utils/focusManager.ts';
import {
  type HelpCategory,
  type SafeActionId,
  HELP_CATEGORIES,
  searchHelpTopics,
} from '../../types/help.ts';
import { ALL_HELP_TOPICS } from '../../services/helpContent.ts';

export interface HelpModalProps {
  isOpen: boolean;
  selectedTopicId?: string;
  activeTopicId?: string;
  searchQuery: string;
  categoryFilter: HelpCategory | 'all';
  onClose: () => void;
  onSelectTopic: (topicId: string) => void;
  onSearchChange: (query: string) => void;
  onCategoryChange: (category: HelpCategory | 'all') => void;
  onExecuteAction: (actionId: SafeActionId) => void;
}

export function HelpModal(props: HelpModalProps): UiNode | null {
  const {
    isOpen,
    selectedTopicId,
    activeTopicId,
    searchQuery,
    categoryFilter,
    onClose,
    onSelectTopic,
    onSearchChange,
    onCategoryChange,
    onExecuteAction,
  } = props;

  if (!isOpen) return null;

  const currentTopicId = selectedTopicId ?? activeTopicId;

  // Filter topics
  const filteredTopics = searchHelpTopics(
    ALL_HELP_TOPICS,
    searchQuery,
    categoryFilter === 'all' ? undefined : categoryFilter,
  );

  // Active topic resolution
  const activeTopic =
    (currentTopicId ? filteredTopics.find((t) => t.id === currentTopicId) : null) ??
    filteredTopics[0] ??
    ALL_HELP_TOPICS[0];

  // Build detail pane children
  const detailChildren: UiNode[] = [];
  if (activeTopic) {
    const isTroubleshooting = activeTopic.category === 'troubleshooting';
    const catMeta = HELP_CATEGORIES.find((c) => c.id === activeTopic.category);

    detailChildren.push(
      h(
        'div',
        { attrs: { class: 'kw-help-detail-header' } },
        h(
          'span',
          {
            attrs: {
              class: `kw-help-badge kw-help-badge-${activeTopic.category}`,
              'data-testid': 'help-topic-badge',
            },
          },
          `${catMeta ? catMeta.icon + ' ' : ''}${catMeta ? catMeta.label : activeTopic.category}`,
        ),
        h('h2', { attrs: { id: 'help-detail-heading', class: 'kw-topic-heading' } }, activeTopic.title),
        h('p', { attrs: { class: 'kw-topic-summary' } }, activeTopic.summary),
      ),
    );

    // If structured troubleshooting topic
    if (activeTopic.troubleshootingDetail) {
      const ts = activeTopic.troubleshootingDetail;
      detailChildren.push(
        h(
          'div',
          {
            attrs: {
              class: 'kw-troubleshooting-breakdown',
              'data-testid': 'help-troubleshooting-breakdown',
            },
          },
          // 1. What Happened
          h(
            'div',
            { attrs: { class: 'kw-ts-box kw-ts-box-happened' } },
            h('h4', { attrs: { class: 'kw-ts-label' } }, 'What Happened:'),
            h('p', { attrs: { class: 'kw-ts-text' } }, ts.whatHappened),
          ),
          // 2. What Is Known
          h(
            'div',
            { attrs: { class: 'kw-ts-box kw-ts-box-known' } },
            h('h4', { attrs: { class: 'kw-ts-label' } }, 'What Is Known:'),
            h('p', { attrs: { class: 'kw-ts-text' } }, ts.whatIsKnown),
          ),
          // 3. Likely Interpretation
          h(
            'div',
            { attrs: { class: 'kw-ts-box kw-ts-box-interpretation' } },
            h('h4', { attrs: { class: 'kw-ts-label' } }, 'Likely Interpretation:'),
            h('p', { attrs: { class: 'kw-ts-text' } }, ts.likelyInterpretation),
          ),
          // 4. Recommended Next Step
          h(
            'div',
            { attrs: { class: 'kw-ts-box kw-ts-box-step' } },
            h('h4', { attrs: { class: 'kw-ts-label' } }, 'Recommended Next Step:'),
            h('p', { attrs: { class: 'kw-ts-text' } }, ts.recommendedNextStep),
          ),
          // 5. Important Limitations
          h(
            'div',
            { attrs: { class: 'kw-ts-box kw-ts-box-limitations' } },
            h('h4', { attrs: { class: 'kw-ts-label' } }, 'Important Limitations:'),
            h('p', { attrs: { class: 'kw-ts-text' } }, ts.limitations),
          ),
        ),
      );
    } else {
      // General feature guidance
      detailChildren.push(
        h(
          'div',
          { attrs: { class: 'kw-help-detail-body' } },
          ...activeTopic.content.map((paragraph) =>
            h('p', { attrs: { class: 'kw-topic-paragraph' } }, paragraph),
          ),
        ),
      );
    }

    // Step-by-step guidance list if present
    if (activeTopic.steps && activeTopic.steps.length > 0) {
      detailChildren.push(
        h(
          'div',
          { attrs: { class: 'kw-topic-steps-container' } },
          h('h4', { attrs: { class: 'kw-topic-subheading' } }, 'Step-by-Step Instructions'),
          h(
            'ol',
            { attrs: { class: 'kw-topic-steps-list' } },
            ...activeTopic.steps.map((st) => h('li', {}, st)),
          ),
        ),
      );
    }

    // Related Keyboard Shortcuts if present
    if (activeTopic.relatedShortcuts && activeTopic.relatedShortcuts.length > 0) {
      detailChildren.push(
        h(
          'div',
          { attrs: { class: 'kw-topic-shortcuts-container' } },
          h('h4', { attrs: { class: 'kw-topic-subheading' } }, 'Helpful Shortcuts'),
          h(
            'div',
            { attrs: { class: 'kw-topic-shortcuts-list' } },
            ...activeTopic.relatedShortcuts.map((sc) =>
              h(
                'div',
                { attrs: { class: 'kw-topic-shortcut-row' } },
                h('span', { attrs: { class: 'kw-shortcut-desc' } }, sc.description),
                h('kbd', { attrs: { class: 'kw-kbd-key' } }, sc.key),
              ),
            ),
          ),
        ),
      );
    }

    // Recommended Safe Action CTA button
    if (activeTopic.recommendedAction) {
      detailChildren.push(
        h(
          'div',
          { attrs: { class: 'kw-topic-action-bar' } },
          Button({
            testId: 'help-execute-action-btn',
            label: activeTopic.recommendedAction.label,
            variant: 'primary',
            size: 'md',
            onClick: () => {
              if (activeTopic?.recommendedAction) {
                onExecuteAction(activeTopic.recommendedAction.actionId);
              }
            },
          }),
        ),
      );
    }
  } else {
    detailChildren.push(
      h('p', { attrs: { class: 'kw-topic-paragraph' } }, 'Select a topic to read guidance.'),
    );
  }

  // Left column topic items
  const topicListChildren: UiNode[] =
    filteredTopics.length > 0
      ? filteredTopics.map((topic) => {
          const isSelected = activeTopic?.id === topic.id;
          const isTs = topic.category === 'troubleshooting';
          return h(
            'button',
            {
              key: topic.id,
              attrs: {
                type: 'button',
                'data-testid': `help-topic-item-${topic.id}`,
                'data-selected': isSelected ? 'true' : 'false',
                role: 'listitem',
                class: `kw-help-topic-item kw-focus-ring${
                  isSelected ? ' kw-help-topic-item-active' : ''
                }${isTs ? ' kw-help-topic-item-ts' : ''}`,
              },
              onClick: () => onSelectTopic(topic.id),
            },
            h('div', { attrs: { class: 'kw-topic-item-title' } }, topic.title),
            h('div', { attrs: { class: 'kw-topic-item-summary' } }, topic.summary),
          );
        })
      : [
          h(
            'div',
            {
              attrs: {
                class: 'kw-help-empty-search',
                'data-testid': 'help-empty-search',
              },
            },
            h('span', { attrs: { class: 'kw-empty-icon', 'aria-hidden': 'true' } }, '🔍'),
            h('p', {}, `No topics found matching "${searchQuery}".`),
            Button({
              testId: 'help-clear-search-btn',
              label: 'Clear Search',
              variant: 'secondary',
              size: 'sm',
              onClick: () => {
                onSearchChange('');
                onCategoryChange('all');
              },
            }),
          ),
        ];

  return h(
    'div',
    {
      attrs: {
        'data-testid': 'help-modal-backdrop',
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
          'data-testid': 'help-modal-dialog',
          class: 'kw-modal-dialog kw-help-modal',
          role: 'dialog',
          'aria-modal': 'true',
          'aria-labelledby': 'help-modal-title',
          'aria-describedby': 'help-modal-desc',
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
          h('h3', { attrs: { id: 'help-modal-title', class: 'kw-modal-title' } }, 'Help & Troubleshooting'),
          h(
            'p',
            { attrs: { id: 'help-modal-desc', class: 'kw-modal-subtitle' } },
            'Contextual guidance, local feature reference, and diagnostic resolution',
          ),
        ),
        Button({
          testId: 'help-modal-close-btn',
          label: '✕',
          ariaLabel: 'Close Help and Troubleshooting dialog',
          variant: 'ghost',
          size: 'sm',
          onClick: onClose,
        }),
      ),

      // Toolbar: Search + Category Filters
      h(
        'div',
        { attrs: { class: 'kw-help-toolbar' } },
        SearchInput({
          id: 'help-search-field',
          testId: 'help-search-input',
          ariaLabel: 'Search help topics and error codes',
          clearButtonTestId: 'help-clear-search-query-btn',
          value: searchQuery,
          placeholder: 'Search topics, features, or error codes...',
          onChange: onSearchChange,
          onClear: () => onSearchChange(''),
        }),
        h(
          'div',
          {
            attrs: {
              class: 'kw-help-category-pills',
              role: 'tablist',
              'aria-label': 'Help Categories',
            },
          },
          h(
            'button',
            {
              attrs: {
                type: 'button',
                role: 'tab',
                'data-testid': 'help-cat-all',
                'aria-selected': categoryFilter === 'all' ? 'true' : 'false',
                class: `kw-help-cat-pill${categoryFilter === 'all' ? ' kw-cat-pill-active' : ''}`,
              },
              onClick: () => onCategoryChange('all'),
            },
            'All Topics',
          ),
          ...HELP_CATEGORIES.map((cat) => {
            const isActive = categoryFilter === cat.id;
            return h(
              'button',
              {
                key: cat.id,
                attrs: {
                  type: 'button',
                  role: 'tab',
                  'data-testid': `help-cat-${cat.id}`,
                  'aria-selected': isActive ? 'true' : 'false',
                  class: `kw-help-cat-pill${isActive ? ' kw-cat-pill-active' : ''}`,
                },
                onClick: () => onCategoryChange(cat.id),
              },
              `${cat.icon} ${cat.label}`,
            );
          }),
        ),
      ),

      // Main Split View
      h(
        'div',
        { attrs: { class: 'kw-help-split-body' } },
        // Left Column: Topic List
        h(
          'div',
          {
            attrs: {
              class: 'kw-help-topic-list',
              'data-testid': 'help-topic-list',
              role: 'list',
              'aria-label': 'Matching Topics',
            },
          },
          ...topicListChildren,
        ),

        // Right Column: Topic Detail
        h(
          'div',
          {
            attrs: {
              class: 'kw-help-topic-detail',
              'data-testid': 'help-topic-detail',
              tabindex: '0',
              role: 'region',
              'aria-labelledby': 'help-detail-heading',
            },
          },
          ...detailChildren,
        ),
      ),

      // Footer
      h(
        'div',
        { attrs: { class: 'kw-modal-footer' } },
        Button({
          testId: 'help-footer-close-btn',
          label: 'Close Help',
          variant: 'secondary',
          size: 'sm',
          onClick: onClose,
        }),
      ),
    ),
  );
}
