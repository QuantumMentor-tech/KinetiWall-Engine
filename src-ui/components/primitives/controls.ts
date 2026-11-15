/**
 * Reusable UI Primitives (`Step 21 KW-UI-002`):
 * Button, Badge, Spinner, AlertBanner, and EmptyStatePanel.
 */

import { h, type UiNode } from './uiNode.ts';

export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger' | 'neon';
export type ButtonSize = 'sm' | 'md' | 'lg';

export interface ButtonProps {
  id?: string;
  testId?: string;
  label: string;
  ariaLabel?: string;
  variant?: ButtonVariant;
  size?: ButtonSize;
  isLoading?: boolean;
  disabled?: boolean;
  title?: string;
  onClick?: () => void;
}

export function Button(props: ButtonProps): UiNode {
  const variant = props.variant ?? 'secondary';
  const size = props.size ?? 'md';
  const isDisabled = Boolean(props.disabled || props.isLoading);

  return h(
    'button',
    {
      attrs: {
        id: props.id,
        type: 'button',
        'data-testid': props.testId,
        'data-variant': variant,
        'data-size': size,
        'data-loading': props.isLoading ? 'true' : 'false',
        class: `kw-btn kw-btn-${variant} kw-btn-${size} kw-focus-ring`,
        disabled: isDisabled,
        'aria-disabled': isDisabled ? 'true' : undefined,
        'aria-busy': props.isLoading ? 'true' : undefined,
        'aria-label': props.ariaLabel ?? props.label,
        title: props.title,
      },
      onClick: isDisabled ? undefined : props.onClick,
      onKeyDown: isDisabled
        ? undefined
        : (e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault?.();
              props.onClick?.();
            }
          },
    },
    props.isLoading ? Spinner({ size: 'sm', label: `${props.label} in progress` }) : null,
    h('span', { attrs: { class: 'kw-btn-label' } }, props.label),
  );
}

export type BadgeTone = 'cyan' | 'available' | 'warning' | 'error' | 'neutral';

export interface BadgeProps {
  testId?: string;
  label: string;
  iconSymbol: string;
  tone: BadgeTone;
  ariaLabel?: string;
  title?: string;
}

/**
 * Status/Metadata Badge that pairs an explicit icon symbol with readable text
 * so color is never the sole indicator of state (Step 21 §9 & Step 27 §12).
 */
export function Badge(props: BadgeProps): UiNode {
  return h(
    'span',
    {
      attrs: {
        'data-testid': props.testId,
        'data-tone': props.tone,
        class: `kw-badge kw-badge-${props.tone}`,
        'aria-label': props.ariaLabel ?? props.label,
        title: props.title,
      },
    },
    h('span', { attrs: { 'aria-hidden': 'true', class: 'kw-badge-icon' } }, props.iconSymbol),
    h('span', { attrs: { class: 'kw-badge-text' } }, props.label),
  );
}

export function Spinner(props: { size?: 'sm' | 'md' | 'lg'; label?: string; testId?: string }): UiNode {
  const label = props.label ?? 'Loading';
  return h(
    'span',
    {
      attrs: {
        role: 'status',
        'aria-live': 'polite',
        'aria-label': label,
        'data-testid': props.testId ?? 'kw-spinner',
        class: `kw-spinner kw-spinner-${props.size ?? 'md'}`,
      },
    },
    h('span', { attrs: { class: 'kw-sr-only' } }, label),
  );
}

export interface AlertBannerProps {
  testId: string;
  tone: 'warning' | 'error' | 'info' | 'success';
  title: string;
  message: string;
  actionButton?: UiNode;
  dismissButton?: UiNode;
}

export function AlertBanner(props: AlertBannerProps): UiNode {
  const role = props.tone === 'error' || props.tone === 'warning' ? 'alert' : 'status';
  return h(
    'div',
    {
      attrs: {
        role,
        'data-testid': props.testId,
        'data-tone': props.tone,
        class: `kw-alert-banner kw-alert-${props.tone}`,
      },
    },
    h(
      'div',
      { attrs: { class: 'kw-alert-content' } },
      h('strong', { attrs: { class: 'kw-alert-title' } }, props.title),
      h('p', { attrs: { class: 'kw-alert-message' } }, props.message),
    ),
    props.actionButton || props.dismissButton
      ? h(
          'div',
          { attrs: { class: 'kw-alert-actions' } },
          props.actionButton ?? null,
          props.dismissButton ?? null,
        )
      : null,
  );
}

export interface EmptyStatePanelProps {
  testId: string;
  heading: string;
  description: string;
  primaryAction?: UiNode;
}

export function EmptyStatePanel(props: EmptyStatePanelProps): UiNode {
  return h(
    'section',
    {
      attrs: {
        'data-testid': props.testId,
        'aria-label': props.heading,
        class: 'kw-empty-state-panel',
      },
    },
    h('div', { attrs: { 'aria-hidden': 'true', class: 'kw-empty-state-glyph' } }, '[+]'),
    h('h2', { attrs: { class: 'kw-empty-state-heading' } }, props.heading),
    h('p', { attrs: { class: 'kw-empty-state-description' } }, props.description),
    props.primaryAction
      ? h('div', { attrs: { class: 'kw-empty-state-actions' } }, props.primaryAction)
      : null,
  );
}

export interface SearchInputProps {
  id: string;
  testId: string;
  clearButtonTestId: string;
  value: string;
  placeholder: string;
  ariaLabel: string;
  disabled?: boolean;
  onChange: (value: string) => void;
  onClear: () => void;
}

export function SearchInput(props: SearchInputProps): UiNode {
  const hasValue = props.value.length > 0;
  return h(
    'div',
    {
      attrs: {
        role: 'search',
        'data-testid': `${props.testId}-container`,
        class: 'kw-search-field',
      },
    },
    h(
      'label',
      {
        attrs: {
          for: props.id,
          class: 'kw-sr-only',
        },
      },
      props.ariaLabel,
    ),
    h('span', { attrs: { 'aria-hidden': 'true', class: 'kw-search-icon' } }, '⌕'),
    h('input', {
      attrs: {
        id: props.id,
        type: 'search',
        'data-testid': props.testId,
        value: props.value,
        placeholder: props.placeholder,
        'aria-label': props.ariaLabel,
        disabled: Boolean(props.disabled),
        class: 'kw-search-input kw-focus-ring',
      },
      onInput: props.disabled ? undefined : (val) => props.onChange(val),
      onChange: props.disabled ? undefined : (val) => props.onChange(val),
      onKeyDown: props.disabled
        ? undefined
        : (e) => {
            if (e.key === 'Escape' && hasValue) {
              e.preventDefault?.();
              props.onClear();
            }
          },
    }),
    hasValue
      ? Button({
          testId: props.clearButtonTestId,
          label: 'Clear',
          ariaLabel: 'Clear search query',
          variant: 'ghost',
          size: 'sm',
          disabled: props.disabled,
          onClick: props.onClear,
        })
      : null,
  );
}

export interface FilterOption<T extends string> {
  value: T;
  label: string;
  testId: string;
}

export interface SegmentedFilterGroupProps<T extends string> {
  testId: string;
  groupLabel: string;
  selectedValue: T;
  options: readonly FilterOption<T>[];
  disabled?: boolean;
  onSelect: (value: T) => void;
}

export function SegmentedFilterGroup<T extends string>(
  props: SegmentedFilterGroupProps<T>,
): UiNode {
  return h(
    'div',
    {
      attrs: {
        role: 'group',
        'aria-label': props.groupLabel,
        'data-testid': props.testId,
        'data-selected-value': props.selectedValue,
        class: 'kw-filter-group',
      },
    },
    ...props.options.map((opt) => {
      const isSelected = props.selectedValue === opt.value;
      const isDisabled = Boolean(props.disabled);
      return h(
        'button',
        {
          key: opt.value,
          attrs: {
            type: 'button',
            'data-testid': opt.testId,
            'data-filter-value': opt.value,
            'data-selected': isSelected ? 'true' : 'false',
            'aria-pressed': isSelected ? 'true' : 'false',
            'aria-label': `${props.groupLabel}: ${opt.label}`,
            disabled: isDisabled,
            class: `kw-filter-btn kw-focus-ring${isSelected ? ' kw-filter-btn-selected' : ''}`,
          },
          onClick: isDisabled ? undefined : () => props.onSelect(opt.value),
          onKeyDown: isDisabled
            ? undefined
            : (e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault?.();
                  props.onSelect(opt.value);
                }
              },
        },
        opt.label,
      );
    }),
  );
}

export interface SortOption<T extends string> {
  value: T;
  label: string;
}

export interface SortSelectProps<T extends string> {
  id: string;
  testId: string;
  ariaLabel: string;
  selectedValue: T;
  options: readonly SortOption<T>[];
  disabled?: boolean;
  onChange: (value: T) => void;
}

export function SortSelect<T extends string>(props: SortSelectProps<T>): UiNode {
  return h(
    'div',
    {
      attrs: {
        'data-testid': `${props.testId}-wrapper`,
        class: 'kw-sort-wrapper',
      },
    },
    h(
      'label',
      {
        attrs: {
          for: props.id,
          class: 'kw-sort-label',
        },
      },
      'Sort by',
    ),
    h(
      'select',
      {
        attrs: {
          id: props.id,
          'data-testid': props.testId,
          value: props.selectedValue,
          'aria-label': props.ariaLabel,
          disabled: Boolean(props.disabled),
          class: 'kw-sort-select kw-focus-ring',
        },
        onChange: props.disabled ? undefined : (val) => props.onChange(val as T),
      },
      ...props.options.map((opt) =>
        h(
          'option',
          {
            key: opt.value,
            attrs: {
              value: opt.value,
              selected: props.selectedValue === opt.value ? true : undefined,
            },
          },
          opt.label,
        ),
      ),
    ),
  );
}

export interface ConfirmationDialogProps {
  testId: string;
  title: string;
  description: string;
  safetyNotice: string;
  confirmLabel: string;
  cancelLabel?: string;
  isConfirmLoading?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

export function ConfirmationDialog(props: ConfirmationDialogProps): UiNode {
  const titleId = `${props.testId}-title`;
  const descId = `${props.testId}-desc`;
  return h(
    'div',
    {
      attrs: {
        role: 'alertdialog',
        'aria-modal': 'true',
        'aria-labelledby': titleId,
        'aria-describedby': descId,
        'data-testid': props.testId,
        class: 'kw-confirm-dialog',
      },
      onKeyDown: (e) => {
        if (e.key === 'Escape' && !props.isConfirmLoading) {
          e.preventDefault?.();
          props.onCancel();
        }
      },
    },
    h(
      'h3',
      {
        attrs: {
          id: titleId,
          'data-testid': `${props.testId}-title`,
          class: 'kw-confirm-dialog-title',
        },
      },
      props.title,
    ),
    h(
      'p',
      {
        attrs: {
          id: descId,
          'data-testid': `${props.testId}-description`,
          class: 'kw-confirm-dialog-description',
        },
      },
      props.description,
    ),
    h(
      'p',
      {
        attrs: {
          'data-testid': `${props.testId}-safety-notice`,
          class: 'kw-confirm-dialog-safety',
        },
      },
      props.safetyNotice,
    ),
    h(
      'div',
      {
        attrs: {
          class: 'kw-confirm-dialog-actions',
        },
      },
      Button({
        testId: `${props.testId}-cancel-btn`,
        label: props.cancelLabel ?? 'Cancel',
        variant: 'secondary',
        size: 'sm',
        disabled: props.isConfirmLoading,
        onClick: props.onCancel,
      }),
      Button({
        testId: `${props.testId}-confirm-btn`,
        label: props.confirmLabel,
        variant: 'danger',
        size: 'sm',
        isLoading: props.isConfirmLoading,
        onClick: props.onConfirm,
      }),
    ),
  );
}



