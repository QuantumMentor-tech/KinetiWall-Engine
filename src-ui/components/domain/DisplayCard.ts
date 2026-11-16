/**
 * Detailed Display Card Component (Step 46).
 * Presents individual monitor hardware info, resolution, assignment status,
 * truthful playback session state, and assignment management actions.
 */

import { Badge, Button } from '../primitives/controls.ts';
import { h, type UiNode } from '../primitives/uiNode.ts';
import type {
  DisplayDescriptorDto,
  DisplaySessionStatusDto,
  EffectivePlaybackDecisionDto,
  MultiMonitorLayoutMode,
  ResolvedDisplayAssignmentDto,
} from '../../types/display.ts';

export interface DisplayCardProps {
  display: DisplayDescriptorDto;
  assignment?: ResolvedDisplayAssignmentDto;
  session?: DisplaySessionStatusDto;
  policyDecision?: EffectivePlaybackDecisionDto;
  isSelected: boolean;
  layoutMode: MultiMonitorLayoutMode;
  disabled?: boolean;
  onSelect: () => void;
  onChangeWallpaper: () => void;
  onClearAssignment: () => void;
}

export function DisplayCard(props: DisplayCardProps): UiNode {
  const {
    display,
    assignment,
    session,
    policyDecision,
    isSelected,
    layoutMode,
    disabled,
    onSelect,
    onChangeWallpaper,
    onClearAssignment,
  } = props;

  const friendlyName = display.friendlyName || display.deviceName || 'Display';
  const hasWallpaper = Boolean(assignment?.wallpaperId);
  const wallpaperTitle = assignment?.effectiveTitle || (hasWallpaper ? 'Assigned Wallpaper' : 'No wallpaper assigned');
  const isExplicit = assignment?.assignmentKind === 'explicit';

  // Session state mapping
  const sessionState = session?.state ?? 'uninitialized';
  const isSessionActive = sessionState === 'active';

  let sessionTone: 'available' | 'cyan' | 'warning' | 'error' | 'neutral' = 'neutral';
  let sessionLabel = 'No Session';
  let sessionSymbol = '○';

  if (sessionState === 'active') {
    sessionTone = 'available';
    sessionLabel = 'Live Wallpaper Active';
    sessionSymbol = '●';
  } else if (sessionState === 'starting') {
    sessionTone = 'cyan';
    sessionLabel = 'Starting Playback';
    sessionSymbol = '⏳';
  } else if (sessionState === 'static_fallback') {
    sessionTone = 'cyan';
    sessionLabel = session?.fallbackSource ? `Fallback: ${session.fallbackSource}` : 'Static Fallback';
    sessionSymbol = '🖼';
  } else if (sessionState === 'suspended') {
    sessionTone = 'warning';
    sessionLabel = 'Playback Paused';
    sessionSymbol = '⏸';
  } else if (sessionState === 'failed') {
    sessionTone = 'error';
    sessionLabel = session?.failureReason ? `Failed: ${session.failureReason}` : 'Playback Failed';
    sessionSymbol = '✕';
  } else if (sessionState === 'detached') {
    sessionTone = 'warning';
    sessionLabel = 'Host Detached';
    sessionSymbol = '⚠';
  } else if (sessionState === 'stopped') {
    sessionTone = 'neutral';
    sessionLabel = 'Playback Stopped';
    sessionSymbol = '■';
  }

  return h(
    'article',
    {
      attrs: {
        'data-testid': `display-card-${display.id}`,
        'data-display-id': display.id,
        'data-selected': isSelected ? 'true' : 'false',
        'data-primary': display.isPrimary ? 'true' : 'false',
        class: `kw-display-card${isSelected ? ' kw-display-card-selected' : ''}`,
      },
    },
    // Top Bar: Name, Badges
    h(
      'div',
      { attrs: { class: 'kw-display-card-header' } },
      h(
        'div',
        { attrs: { class: 'kw-display-card-title-group' } },
        h('h4', { attrs: { class: 'kw-display-card-title' } }, friendlyName),
        h(
          'span',
          { attrs: { class: 'kw-display-card-device', title: display.hardwareId ?? display.deviceName } },
          display.deviceName,
        ),
      ),
      h(
        'div',
        { attrs: { class: 'kw-display-card-badges' } },
        display.isPrimary
          ? Badge({
              testId: `display-primary-badge-${display.id}`,
              label: 'Primary',
              iconSymbol: '★',
              tone: 'cyan',
            })
          : null,
        Badge({
          testId: `display-session-badge-${display.id}`,
          label: sessionLabel,
          iconSymbol: sessionSymbol,
          tone: sessionTone,
        }),
        session?.effectiveQualityLevel && session.effectiveQualityLevel !== 'normal'
          ? Badge({
              testId: `display-adaptive-badge-${display.id}`,
              label: `Adaptive (${session.effectiveFps ?? 30} FPS)`,
              iconSymbol: '⚡',
              tone: 'warning',
            })
          : null,
      ),
    ),
    // Specs Grid: Geometry, Scale, Coordinates
    h(
      'div',
      { attrs: { class: 'kw-display-card-specs' } },
      h(
        'div',
        { attrs: { class: 'kw-spec-item' } },
        h('span', { attrs: { class: 'kw-spec-label' } }, 'Resolution'),
        h('span', { attrs: { class: 'kw-spec-value' } }, `${display.geometry.width} × ${display.geometry.height}`),
      ),
      h(
        'div',
        { attrs: { class: 'kw-spec-item' } },
        h('span', { attrs: { class: 'kw-spec-label' } }, 'Position'),
        h(
          'span',
          { attrs: { class: 'kw-spec-value' } },
          `(${display.geometry.left}, ${display.geometry.top})`,
        ),
      ),
      h(
        'div',
        { attrs: { class: 'kw-spec-item' } },
        h('span', { attrs: { class: 'kw-spec-label' } }, 'DPI Scale'),
        h(
          'span',
          { attrs: { class: 'kw-spec-value' } },
          `${display.geometry.scaleFactorPercent ?? 100}%`,
        ),
      ),
    ),
    // Assignment section
    h(
      'div',
      { attrs: { class: 'kw-display-card-assignment' } },
      h(
        'div',
        { attrs: { class: 'kw-assignment-info' } },
        h('span', { attrs: { class: 'kw-assignment-heading' } }, 'Assigned Wallpaper:'),
        h(
          'span',
          {
            attrs: {
              class: `kw-assignment-title${hasWallpaper ? '' : ' kw-assignment-empty'}`,
              'data-testid': `display-wallpaper-title-${display.id}`,
            },
          },
          wallpaperTitle,
        ),
        hasWallpaper
          ? h(
              'span',
              { attrs: { class: 'kw-assignment-meta' } },
              isExplicit ? 'Explicit assignment' : 'Global fallback',
            )
          : null,
      ),
      // Action buttons
      h(
        'div',
        { attrs: { class: 'kw-display-card-actions' } },
        Button({
          testId: `display-change-wallpaper-btn-${display.id}`,
          label: hasWallpaper ? 'Change Wallpaper...' : 'Assign Wallpaper...',
          variant: 'primary',
          size: 'sm',
          disabled,
          onClick: onChangeWallpaper,
        }),
        isExplicit
          ? Button({
              testId: `display-clear-assignment-btn-${display.id}`,
              label: 'Clear',
              variant: 'ghost',
              size: 'sm',
              disabled,
              onClick: onClearAssignment,
            })
          : null,
      ),
    ),
    // Step 57: Policy Decision Diagnostics & Explainability
    policyDecision
      ? h(
          'div',
          {
            attrs: {
              'data-testid': `display-policy-diagnostics-${display.id}`,
              style:
                'margin-top: 10px; padding: 8px 12px; border-radius: 6px; background: rgba(0, 240, 255, 0.03); border: 1px solid rgba(0, 240, 255, 0.1); font-size: 0.75rem; color: #8A8F98; display: flex; flex-direction: column; gap: 4px;',
            },
          },
          h(
            'div',
            { style: 'display: flex; align-items: center; justify-content: space-between;' },
            h(
              'span',
              { style: 'font-weight: 600; color: #00F0FF; display: flex; align-items: center; gap: 4px;' },
              '🔍 Policy Decision:',
            ),
            h(
              'span',
              {
                attrs: { 'data-testid': `policy-reason-code-${display.id}` },
                style: 'font-family: monospace; font-size: 0.7rem; color: #5A6072;',
              },
              policyDecision.primaryReasonCode,
            ),
          ),
          h(
            'p',
            {
              attrs: { 'data-testid': `policy-explanation-text-${display.id}` },
              style: 'margin: 0; color: #D1D5DB; line-height: 1.3;',
            },
            policyDecision.primaryExplanation,
          ),
          policyDecision.isStateDivergent
            ? h(
                'div',
                {
                  attrs: { 'data-testid': `policy-divergence-note-${display.id}` },
                  style: 'font-size: 0.7rem; color: #FFB020; margin-top: 2px;',
                },
                `⚠️ Reconciling transition: desired=${policyDecision.desiredState}, backend=${policyDecision.actualState}`,
              )
            : null,
        )
      : null,
  );
}

