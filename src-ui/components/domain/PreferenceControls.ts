/**
 * Reusable Settings & Preference UI Components (Step 61).
 * Adheres to Neon Glass Design Tokens, accessibility standards (WCAG 2.1 AA),
 * and transparent save-state tracking.
 */

import { AlertBanner, Badge, Button, Spinner } from '../primitives/controls.ts';
import { h, type UiNode } from '../primitives/uiNode.ts';

// -------------------------------------------------------------------------------------------------
// 1. PreferenceSection
// -------------------------------------------------------------------------------------------------

export interface PreferenceSectionProps {
  testId: string;
  title: string;
  description: string;
  icon: string;
  onResetSection?: () => void;
  resetSectionLabel?: string;
  children: Array<UiNode | string | null | undefined | false>;
}

export function PreferenceSection(props: PreferenceSectionProps): UiNode {
  return h(
    'section',
    {
      attrs: {
        'data-testid': props.testId,
        'aria-labelledby': `${props.testId}-heading`,
        class: 'kw-preference-section',
      },
    },
    h(
      'header',
      { attrs: { class: 'kw-preference-section-header' } },
      h(
        'div',
        { attrs: { class: 'kw-preference-section-title-group' } },
        h(
          'span',
          { attrs: { 'aria-hidden': 'true', class: 'kw-preference-section-icon' } },
          props.icon,
        ),
        h(
          'div',
          {},
          h(
            'h2',
            {
              attrs: {
                id: `${props.testId}-heading`,
                class: 'kw-preference-section-title',
              },
            },
            props.title,
          ),
          h('p', { attrs: { class: 'kw-preference-section-desc' } }, props.description),
        ),
      ),
      props.onResetSection
        ? Button({
            testId: `${props.testId}-reset-btn`,
            label: props.resetSectionLabel ?? 'Reset Section',
            variant: 'ghost',
            size: 'sm',
            onClick: props.onResetSection,
          })
        : null,
    ),
    h('div', { attrs: { class: 'kw-preference-section-body' } }, ...props.children),
  );
}

// -------------------------------------------------------------------------------------------------
// 2. PreferenceToggle (Switch)
// -------------------------------------------------------------------------------------------------

export interface PreferenceToggleProps {
  id: string;
  testId: string;
  label: string;
  description: string;
  checked: boolean;
  disabled?: boolean;
  dirty?: boolean;
  errorMessage?: string;
  onChange: (checked: boolean) => void;
}

export function PreferenceToggle(props: PreferenceToggleProps): UiNode {
  const isChecked = Boolean(props.checked);
  const isDisabled = Boolean(props.disabled);

  return h(
    'div',
    {
      attrs: {
        'data-testid': `${props.testId}-row`,
        'data-dirty': props.dirty ? 'true' : 'false',
        class: `kw-preference-row${props.dirty ? ' kw-preference-row-dirty' : ''}`,
      },
    },
    h(
      'div',
      { attrs: { class: 'kw-preference-row-text' } },
      h(
        'label',
        {
          attrs: {
            for: props.id,
            id: `${props.id}-label`,
            class: 'kw-preference-label',
          },
        },
        props.label,
        props.dirty
          ? h(
              'span',
              {
                attrs: {
                  'data-testid': `${props.testId}-dirty-dot`,
                  class: 'kw-dirty-indicator',
                  title: 'Modified draft value (unsaved)',
                },
              },
              '●',
            )
          : null,
      ),
      h('p', { attrs: { class: 'kw-preference-row-desc' } }, props.description),
      props.errorMessage
        ? h('p', { attrs: { class: 'kw-preference-error', role: 'alert' } }, props.errorMessage)
        : null,
    ),
    h(
      'button',
      {
        attrs: {
          id: props.id,
          type: 'button',
          role: 'switch',
          'aria-checked': isChecked ? 'true' : 'false',
          'aria-labelledby': `${props.id}-label`,
          'data-testid': props.testId,
          disabled: isDisabled,
          class: `kw-preference-toggle kw-focus-ring${isChecked ? ' kw-toggle-checked' : ''}${
            isDisabled ? ' kw-toggle-disabled' : ''
          }`,
        },
        onClick: isDisabled ? undefined : () => props.onChange(!isChecked),
        onKeyDown: isDisabled
          ? undefined
          : (e) => {
              if (e.key === ' ' || e.key === 'Enter') {
                e.preventDefault?.();
                props.onChange(!isChecked);
              }
            },
      },
      h('span', { attrs: { class: 'kw-toggle-thumb' } }),
      h(
        'span',
        { attrs: { class: 'kw-sr-only' } },
        isChecked ? 'Enabled' : 'Disabled',
      ),
    ),
  );
}

// -------------------------------------------------------------------------------------------------
// 3. PreferenceSelect (Dropdown)
// -------------------------------------------------------------------------------------------------

export interface PreferenceSelectOption {
  value: string;
  label: string;
}

export interface PreferenceSelectProps {
  id: string;
  testId: string;
  label: string;
  description: string;
  value: string;
  options: readonly PreferenceSelectOption[];
  disabled?: boolean;
  dirty?: boolean;
  errorMessage?: string;
  onChange: (value: string) => void;
}

export function PreferenceSelect(props: PreferenceSelectProps): UiNode {
  const isDisabled = Boolean(props.disabled);

  return h(
    'div',
    {
      attrs: {
        'data-testid': `${props.testId}-row`,
        'data-dirty': props.dirty ? 'true' : 'false',
        class: `kw-preference-row${props.dirty ? ' kw-preference-row-dirty' : ''}`,
      },
    },
    h(
      'div',
      { attrs: { class: 'kw-preference-row-text' } },
      h(
        'label',
        {
          attrs: {
            for: props.id,
            id: `${props.id}-label`,
            class: 'kw-preference-label',
          },
        },
        props.label,
        props.dirty
          ? h(
              'span',
              {
                attrs: {
                  'data-testid': `${props.testId}-dirty-dot`,
                  class: 'kw-dirty-indicator',
                  title: 'Modified draft value (unsaved)',
                },
              },
              '●',
            )
          : null,
      ),
      h('p', { attrs: { class: 'kw-preference-row-desc' } }, props.description),
      props.errorMessage
        ? h('p', { attrs: { class: 'kw-preference-error', role: 'alert' } }, props.errorMessage)
        : null,
    ),
    h(
      'select',
      {
        attrs: {
          id: props.id,
          'data-testid': props.testId,
          'aria-labelledby': `${props.id}-label`,
          disabled: isDisabled,
          class: 'kw-preference-select kw-focus-ring',
          value: props.value,
        },
        onChange: isDisabled ? undefined : (val) => props.onChange(val),
      },
      ...props.options.map((opt) =>
        h(
          'option',
          {
            key: opt.value,
            attrs: {
              value: opt.value,
              selected: props.value === opt.value ? true : undefined,
            },
          },
          opt.label,
        ),
      ),
    ),
  );
}

// -------------------------------------------------------------------------------------------------
// 4. PreferenceSegmentedGroup
// -------------------------------------------------------------------------------------------------

export interface SegmentedGroupOption {
  value: string;
  label: string;
  testId: string;
}

export interface PreferenceSegmentedGroupProps {
  testId: string;
  label: string;
  description: string;
  value: string;
  options: readonly SegmentedGroupOption[];
  disabled?: boolean;
  dirty?: boolean;
  errorMessage?: string;
  onSelect: (value: string) => void;
}

export function PreferenceSegmentedGroup(props: PreferenceSegmentedGroupProps): UiNode {
  const isDisabled = Boolean(props.disabled);

  return h(
    'div',
    {
      attrs: {
        'data-testid': `${props.testId}-row`,
        'data-dirty': props.dirty ? 'true' : 'false',
        class: `kw-preference-row kw-preference-row-vertical${
          props.dirty ? ' kw-preference-row-dirty' : ''
        }`,
      },
    },
    h(
      'div',
      { attrs: { class: 'kw-preference-row-text' } },
      h(
        'span',
        { attrs: { class: 'kw-preference-label' } },
        props.label,
        props.dirty
          ? h(
              'span',
              {
                attrs: {
                  'data-testid': `${props.testId}-dirty-dot`,
                  class: 'kw-dirty-indicator',
                  title: 'Modified draft value (unsaved)',
                },
              },
              '●',
            )
          : null,
      ),
      h('p', { attrs: { class: 'kw-preference-row-desc' } }, props.description),
      props.errorMessage
        ? h('p', { attrs: { class: 'kw-preference-error', role: 'alert' } }, props.errorMessage)
        : null,
    ),
    h(
      'div',
      {
        attrs: {
          role: 'radiogroup',
          'aria-label': props.label,
          'data-testid': props.testId,
          class: 'kw-filter-group',
        },
      },
      ...props.options.map((opt, index) => {
        const isSelected = props.value === opt.value;
        return h(
          'button',
          {
            key: opt.value,
            attrs: {
              type: 'button',
              role: 'radio',
              'aria-checked': isSelected ? 'true' : 'false',
              'data-testid': opt.testId,
              'data-selected': isSelected ? 'true' : 'false',
              disabled: isDisabled,
              tabindex: isSelected ? '0' : '-1',
              class: `kw-filter-btn kw-focus-ring${isSelected ? ' kw-filter-btn-selected' : ''}`,
            },
            onClick: isDisabled ? undefined : () => props.onSelect(opt.value),
            onKeyDown: isDisabled
              ? undefined
              : (e) => {
                  if (e.key === ' ' || e.key === 'Enter') {
                    e.preventDefault?.();
                    props.onSelect(opt.value);
                  } else if (e.key === 'ArrowRight' || e.key === 'ArrowDown') {
                    e.preventDefault?.();
                    const nextIndex = (index + 1) % props.options.length;
                    props.onSelect(props.options[nextIndex].value);
                  } else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') {
                    e.preventDefault?.();
                    const prevIndex = (index - 1 + props.options.length) % props.options.length;
                    props.onSelect(props.options[prevIndex].value);
                  }
                },
          },
          opt.label,
        );
      }),
    ),
  );
}

// -------------------------------------------------------------------------------------------------
// 5. PreferenceSlider (Range / Number)
// -------------------------------------------------------------------------------------------------

export interface PreferenceSliderProps {
  id: string;
  testId: string;
  label: string;
  description: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  unitLabel: string;
  formatDisplayValue?: (val: number) => string;
  disabled?: boolean;
  dirty?: boolean;
  errorMessage?: string;
  onChange: (value: number) => void;
}

export function PreferenceSlider(props: PreferenceSliderProps): UiNode {
  const isDisabled = Boolean(props.disabled);
  const displayVal = props.formatDisplayValue
    ? props.formatDisplayValue(props.value)
    : `${props.value} ${props.unitLabel}`;

  return h(
    'div',
    {
      attrs: {
        'data-testid': `${props.testId}-row`,
        'data-dirty': props.dirty ? 'true' : 'false',
        class: `kw-preference-row kw-preference-row-vertical${
          props.dirty ? ' kw-preference-row-dirty' : ''
        }`,
      },
    },
    h(
      'div',
      { attrs: { class: 'kw-preference-row-header-inline' } },
      h(
        'div',
        { attrs: { class: 'kw-preference-row-text' } },
        h(
          'label',
          {
            attrs: {
              for: props.id,
              id: `${props.id}-label`,
              class: 'kw-preference-label',
            },
          },
          props.label,
          props.dirty
            ? h(
                'span',
                {
                  attrs: {
                    'data-testid': `${props.testId}-dirty-dot`,
                    class: 'kw-dirty-indicator',
                    title: 'Modified draft value (unsaved)',
                  },
                },
                '●',
              )
            : null,
        ),
        h('p', { attrs: { class: 'kw-preference-row-desc' } }, props.description),
      ),
      Badge({
        testId: `${props.testId}-badge`,
        label: displayVal,
        iconSymbol: '⎚',
        tone: props.dirty ? 'cyan' : 'neutral',
      }),
    ),
    props.errorMessage
      ? h('p', { attrs: { class: 'kw-preference-error', role: 'alert' } }, props.errorMessage)
      : null,
    h(
      'div',
      { attrs: { class: 'kw-slider-container' } },
      h('input', {
        attrs: {
          id: props.id,
          type: 'range',
          'data-testid': props.testId,
          'aria-labelledby': `${props.id}-label`,
          'aria-valuemin': props.min,
          'aria-valuemax': props.max,
          'aria-valuenow': props.value,
          'aria-valuetext': displayVal,
          min: props.min,
          max: props.max,
          step: props.step ?? 1,
          value: props.value,
          disabled: isDisabled,
          class: 'kw-preference-slider kw-focus-ring',
        },
        onInput: isDisabled ? undefined : (val) => props.onChange(Number(val)),
        onChange: isDisabled ? undefined : (val) => props.onChange(Number(val)),
      }),
      h(
        'div',
        { attrs: { class: 'kw-slider-scale' } },
        h('span', {}, `${props.min} ${props.unitLabel}`),
        h('span', {}, `${props.max} ${props.unitLabel}`),
      ),
    ),
  );
}

// -------------------------------------------------------------------------------------------------
// 6. SaveStatusBar
// -------------------------------------------------------------------------------------------------

export interface SaveStatusBarProps {
  testId: string;
  isDirty: boolean;
  dirtyCount: number;
  isSaving: boolean;
  saveStatus: 'idle' | 'saving' | 'saved' | 'error';
  saveError: string | null;
  hasValidationErrors: boolean;
  lastSavedTimestampMillis: number | null;
  onSave: () => void;
  onDiscard: () => void;
}

export function SaveStatusBar(props: SaveStatusBarProps): UiNode {
  let statusText = 'Preferences saved and up to date';
  let statusTone: 'available' | 'cyan' | 'warning' | 'error' = 'available';

  if (props.saveStatus === 'saving') {
    statusText = 'Saving preferences to disk...';
    statusTone = 'cyan';
  } else if (props.saveStatus === 'error' || props.saveError) {
    statusText = props.saveError ?? 'Failed to save preferences';
    statusTone = 'error';
  } else if (props.isDirty) {
    statusText = `${props.dirtyCount} unsaved preference${
      props.dirtyCount === 1 ? '' : 's'
    } in draft`;
    statusTone = 'warning';
  }

  return h(
    'div',
    {
      attrs: {
        'data-testid': props.testId,
        'data-dirty': props.isDirty ? 'true' : 'false',
        'data-status': props.saveStatus,
        class: `kw-save-status-bar${props.isDirty ? ' kw-save-status-bar-dirty' : ''}`,
      },
    },
    h(
      'div',
      { attrs: { class: 'kw-save-status-info' } },
      props.isSaving
        ? Spinner({ size: 'sm', label: 'Saving' })
        : Badge({
            testId: `${props.testId}-badge`,
            label: statusText,
            iconSymbol:
              statusTone === 'available'
                ? '✓'
                : statusTone === 'warning'
                ? '●'
                : statusTone === 'error'
                ? '⚠'
                : '⟳',
            tone: statusTone,
          }),
      props.lastSavedTimestampMillis && !props.isDirty && props.saveStatus === 'saved'
        ? h(
            'span',
            { attrs: { class: 'kw-save-status-time' } },
            `Last saved ${new Date(props.lastSavedTimestampMillis).toLocaleTimeString()}`,
          )
        : null,
    ),
    h(
      'div',
      { attrs: { class: 'kw-save-status-actions' } },
      Button({
        testId: `${props.testId}-discard-btn`,
        label: 'Discard Changes',
        variant: 'secondary',
        size: 'sm',
        disabled: !props.isDirty || props.isSaving,
        onClick: props.onDiscard,
      }),
      Button({
        testId: `${props.testId}-save-btn`,
        label: 'Save Changes',
        variant: 'primary',
        size: 'sm',
        isLoading: props.isSaving,
        disabled: !props.isDirty || props.hasValidationErrors || props.isSaving,
        onClick: props.onSave,
      }),
    ),
  );
}

