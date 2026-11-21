/**
 * First-Run Experience & Guided Onboarding Modal (Step 66).
 *
 * Accessible multi-stage onboarding tour providing:
 * - Engine introduction & local-first guarantees.
 * - Truthful capability readiness evaluation with safe re-check action.
 * - Workflow guidance for catalog browsing, asset imports, and multi-monitor layouts.
 * - Clear, non-blocking skip, back, next, and finish actions.
 */

import { AlertBanner, Badge, Button, Spinner } from '../primitives/controls.ts';
import { h, type UiNode } from '../primitives/uiNode.ts';
import type { AggregateReadinessReport } from '../../types/readiness.ts';
import { getReadinessTone, getReadinessLabel, getReadinessSymbol } from '../../types/readiness.ts';
import { trapFocus } from '../../utils/focusManager.ts';

export interface OnboardingModalProps {
  isOpen: boolean;
  currentStageIndex: number;
  report: AggregateReadinessReport;
  isReadinessLoading?: boolean;
  persistenceError?: string | null;
  onStageChange: (index: number) => void;
  onRefreshReadiness: () => void;
  onComplete: () => void;
  onDismiss: () => void;
  onOpenLibrary: () => void;
}

export const ONBOARDING_STAGES = [
  { id: 'welcome', title: 'Welcome to KinetiWall', label: 'Welcome' },
  { id: 'readiness', title: 'System & Capability Readiness', label: 'Readiness' },
  { id: 'workflows', title: 'Essential Workflows', label: 'Workflows' },
  { id: 'completion', title: "You're All Set!", label: 'Finish' },
] as const;

export function OnboardingModal(props: OnboardingModalProps): UiNode | null {
  const {
    isOpen,
    currentStageIndex,
    report,
    isReadinessLoading = false,
    persistenceError = null,
    onStageChange,
    onRefreshReadiness,
    onComplete,
    onDismiss,
    onOpenLibrary,
  } = props;

  if (!isOpen) {
    return null;
  }

  const stage = ONBOARDING_STAGES[currentStageIndex] ?? ONBOARDING_STAGES[0];
  const totalStages = ONBOARDING_STAGES.length;
  const isFirstStage = currentStageIndex === 0;
  const isLastStage = currentStageIndex === totalStages - 1;

  // Render content depending on active stage
  let stageContent: UiNode;

  if (currentStageIndex === 0) {
    // -------------------------------------------------------------------------
    // Stage 1: Welcome & Overview
    // -------------------------------------------------------------------------
    stageContent = h(
      'div',
      { attrs: { class: 'kw-onboarding-stage-content', 'data-testid': 'onboarding-stage-welcome' } },
      h(
        'div',
        { attrs: { class: 'kw-onboarding-hero' } },
        h('span', { attrs: { class: 'kw-onboarding-hero-icon', 'aria-hidden': 'true' } }, '◆'),
        h(
          'p',
          { attrs: { class: 'kw-onboarding-hero-text' } },
          'KinetiWall is a high-performance, local-first live wallpaper engine for Windows. It embeds smooth, hardware-accelerated wallpapers directly into your desktop background while honoring battery, fullscreen games, and display power policies.',
        ),
      ),
      h(
        'div',
        { attrs: { class: 'kw-onboarding-feature-grid' } },
        h(
          'div',
          { attrs: { class: 'kw-onboarding-feature-card' } },
          h('span', { attrs: { class: 'kw-onboarding-feature-icon', 'aria-hidden': 'true' } }, '🔒'),
          h('h4', { attrs: { class: 'kw-onboarding-feature-title' } }, '100% Local & Private'),
          h(
            'p',
            { attrs: { class: 'kw-onboarding-feature-copy' } },
            'Zero cloud accounts, zero telemetry, and zero remote analytics. All wallpaper assets, thumbnails, and preferences stay on your PC.',
          ),
        ),
        h(
          'div',
          { attrs: { class: 'kw-onboarding-feature-card' } },
          h('span', { attrs: { class: 'kw-onboarding-feature-icon', 'aria-hidden': 'true' } }, '⚡'),
          h('h4', { attrs: { class: 'kw-onboarding-feature-title' } }, 'Resource-Aware Policies'),
          h(
            'p',
            { attrs: { class: 'kw-onboarding-feature-copy' } },
            'Automatically pauses or throttles playback during fullscreen games, battery power, workstation lock, and display sleep.',
          ),
        ),
        h(
          'div',
          { attrs: { class: 'kw-onboarding-feature-card' } },
          h('span', { attrs: { class: 'kw-onboarding-feature-icon', 'aria-hidden': 'true' } }, '🖥'),
          h('h4', { attrs: { class: 'kw-onboarding-feature-title' } }, 'Multi-Monitor Ready'),
          h(
            'p',
            { attrs: { class: 'kw-onboarding-feature-copy' } },
            'Assign different wallpapers per display, span ultra-wide panoramic scenes, or clone identical playback across monitors.',
          ),
        ),
        h(
          'div',
          { attrs: { class: 'kw-onboarding-feature-card' } },
          h('span', { attrs: { class: 'kw-onboarding-feature-icon', 'aria-hidden': 'true' } }, '🎬'),
          h('h4', { attrs: { class: 'kw-onboarding-feature-title' } }, 'Video & Interactive Web'),
          h(
            'p',
            { attrs: { class: 'kw-onboarding-feature-copy' } },
            'Supports video loops (.mp4, .webm) and interactive HTML5/Canvas packages with optional audio reactivity.',
          ),
        ),
      ),
    );
  } else if (currentStageIndex === 1) {
    // -------------------------------------------------------------------------
    // Stage 2: Truthful System Capability Readiness
    // -------------------------------------------------------------------------
    stageContent = h(
      'div',
      { attrs: { class: 'kw-onboarding-stage-content', 'data-testid': 'onboarding-stage-readiness' } },
      h(
        'div',
        { attrs: { class: 'kw-onboarding-readiness-header' } },
        h(
          'div',
          { attrs: { class: 'kw-onboarding-readiness-intro' } },
          h(
            'p',
            { attrs: { class: 'kw-modal-subtitle' } },
            'KinetiWall performs non-invasive checks to verify that essential services and multimedia backends are ready.',
          ),
        ),
        Button({
          testId: 'onboarding-recheck-readiness-btn',
          label: isReadinessLoading ? 'Checking...' : 'Re-check Readiness',
          variant: 'secondary',
          size: 'sm',
          isLoading: isReadinessLoading,
          onClick: onRefreshReadiness,
        }),
      ),
      // Readiness items list
      h(
        'div',
        {
          attrs: {
            class: 'kw-readiness-list',
            'data-testid': 'onboarding-readiness-list',
            role: 'list',
          },
        },
        ...report.capabilities.map((cap) => {
          const tone = getReadinessTone(cap.state);
          const label = getReadinessLabel(cap.state);
          const symbol = getReadinessSymbol(cap.state);
          const isEssential = cap.requirement === 'essential';

          return h(
            'div',
            {
              key: cap.id,
              attrs: {
                class: `kw-readiness-item kw-readiness-item-${cap.state}`,
                'data-testid': `readiness-item-${cap.id}`,
                role: 'listitem',
              },
            },
            h(
              'div',
              { attrs: { class: 'kw-readiness-item-main' } },
              h(
                'div',
                { attrs: { class: 'kw-readiness-item-title-row' } },
                h('span', { attrs: { class: 'kw-readiness-item-label' } }, cap.label),
                isEssential
                  ? h(
                      'span',
                      { attrs: { class: 'kw-badge kw-badge-subtle', title: 'Required for core operation' } },
                      'Essential',
                    )
                  : h(
                      'span',
                      { attrs: { class: 'kw-badge kw-badge-subtle', title: 'Optional multimedia enhancement' } },
                      'Optional',
                    ),
              ),
              h('p', { attrs: { class: 'kw-readiness-item-detail' } }, cap.detail),
              cap.remediation
                ? h(
                    'p',
                    { attrs: { class: 'kw-readiness-item-remediation' } },
                    `Tip: ${cap.remediation}`,
                  )
                : null,
            ),
            h(
              'div',
              { attrs: { class: 'kw-readiness-item-status' } },
              Badge({
                testId: `readiness-badge-${cap.id}`,
                label,
                iconSymbol: symbol,
                tone: tone === 'success' ? 'available' : tone === 'warning' ? 'warning' : 'neutral',
              }),
            ),
          );
        }),
      ),
      // Overall status summary
      h(
        'div',
        { attrs: { class: 'kw-readiness-summary-card' } },
        h(
          'p',
          { attrs: { class: 'kw-readiness-summary-text' } },
          report.isOperational
            ? report.hasWarnings
              ? 'Engine is operational. Some optional backends are restricted, but wallpaper playback is available.'
              : 'All essential systems and optional multimedia backends are ready for playback.'
            : 'One or more essential prerequisites need your attention before wallpapers can attach.',
        ),
      ),
    );
  } else if (currentStageIndex === 2) {
    // -------------------------------------------------------------------------
    // Stage 3: Essential Workflows
    // -------------------------------------------------------------------------
    stageContent = h(
      'div',
      { attrs: { class: 'kw-onboarding-stage-content', 'data-testid': 'onboarding-stage-workflows' } },
      h(
        'div',
        { attrs: { class: 'kw-workflow-steps' } },
        h(
          'div',
          { attrs: { class: 'kw-workflow-card' } },
          h('div', { attrs: { class: 'kw-workflow-num' } }, '1'),
          h(
            'div',
            { attrs: { class: 'kw-workflow-body' } },
            h('h4', { attrs: { class: 'kw-workflow-title' } }, 'Browse Wallpaper Library (Alt+2)'),
            h(
              'p',
              { attrs: { class: 'kw-workflow-copy' } },
              'View your local wallpaper library in a responsive grid. Inspect asset metadata, verify disk availability, and search by title, author, or tags.',
            ),
          ),
        ),
        h(
          'div',
          { attrs: { class: 'kw-workflow-card' } },
          h('div', { attrs: { class: 'kw-workflow-num' } }, '2'),
          h(
            'div',
            { attrs: { class: 'kw-workflow-body' } },
            h('h4', { attrs: { class: 'kw-workflow-title' } }, 'Import Supported Content'),
            h(
              'p',
              { attrs: { class: 'kw-workflow-copy' } },
              'Click "Add Wallpaper" to register video files (.mp4, .webm) or web wallpaper folders. KinetiWall never scans your disk automatically without consent.',
            ),
          ),
        ),
        h(
          'div',
          { attrs: { class: 'kw-workflow-card' } },
          h('div', { attrs: { class: 'kw-workflow-num' } }, '3'),
          h(
            'div',
            { attrs: { class: 'kw-workflow-body' } },
            h('h4', { attrs: { class: 'kw-workflow-title' } }, 'Assign Displays & Layout (Alt+3)'),
            h(
              'p',
              { attrs: { class: 'kw-workflow-copy' } },
              'Open the Displays view to see connected monitors, choose your presentation mode (Independent, Span, or Clone), and apply your chosen wallpaper.',
            ),
          ),
        ),
      ),
    );
  } else {
    // -------------------------------------------------------------------------
    // Stage 4: Ready to Launch
    // -------------------------------------------------------------------------
    stageContent = h(
      'div',
      { attrs: { class: 'kw-onboarding-stage-content', 'data-testid': 'onboarding-stage-completion' } },
      h(
        'div',
        { attrs: { class: 'kw-onboarding-completion-hero' } },
        h('span', { attrs: { class: 'kw-onboarding-completion-icon', 'aria-hidden': 'true' } }, '🚀'),
        h(
          'h3',
          { attrs: { class: 'kw-onboarding-completion-title' } },
          "You're Ready to Explore KinetiWall",
        ),
        h(
          'p',
          { attrs: { class: 'kw-onboarding-completion-text' } },
          'First-run setup is complete. You can jump directly into your Wallpaper Library to select a wallpaper, or finish setup to explore the dashboard.',
        ),
      ),
      h(
        'div',
        { attrs: { class: 'kw-onboarding-tips-list' } },
        h(
          'div',
          { attrs: { class: 'kw-onboarding-tip-item' } },
          h('strong', {}, 'Tip: '),
          h('span', {}, 'Press '),
          h('kbd', { attrs: { class: 'kw-kbd' } }, '?'),
          h('span', {}, ' or '),
          h('kbd', { attrs: { class: 'kw-kbd' } }, 'F1'),
          h('span', {}, ' at any time to open the Keyboard Shortcuts reference dialog.'),
        ),
        h(
          'div',
          { attrs: { class: 'kw-onboarding-tip-item' } },
          h('strong', {}, 'Tip: '),
          h(
            'span',
            {},
            'You can revisit this guided tour or re-check system readiness anytime from Application Settings (Alt+4).',
          ),
        ),
      ),
    );
  }

  return h(
    'div',
    {
      attrs: {
        'data-testid': 'onboarding-modal-backdrop',
        class: 'kw-modal-backdrop',
        role: 'presentation',
      },
      onClick: (e) => {
        if ((e.target as any)?.classList?.contains('kw-modal-backdrop')) {
          onDismiss();
        }
      },
    },
    h(
      'div',
      {
        attrs: {
          'data-testid': 'onboarding-modal-dialog',
          class: 'kw-modal-dialog kw-onboarding-modal',
          role: 'dialog',
          'aria-modal': 'true',
          'aria-labelledby': 'onboarding-modal-title',
          'aria-describedby': 'onboarding-modal-subtitle',
        },
        onKeyDown: (e) => {
          if (e.key === 'Escape') {
            e.preventDefault?.();
            onDismiss();
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
          h(
            'div',
            { attrs: { class: 'kw-onboarding-step-badge' } },
            `Step ${currentStageIndex + 1} of ${totalStages}: ${stage.label}`,
          ),
          h('h3', { attrs: { id: 'onboarding-modal-title', class: 'kw-modal-title' } }, stage.title),
        ),
        Button({
          testId: 'onboarding-modal-skip-btn',
          label: 'Skip Guide',
          ariaLabel: 'Skip onboarding guide and go to dashboard',
          variant: 'ghost',
          size: 'sm',
          onClick: onDismiss,
        }),
      ),
      // Step indicator bar
      h(
        'div',
        {
          attrs: {
            class: 'kw-onboarding-progress-bar',
            role: 'progressbar',
            'aria-valuemin': '1',
            'aria-valuemax': String(totalStages),
            'aria-valuenow': String(currentStageIndex + 1),
            'aria-label': `Onboarding progress: step ${currentStageIndex + 1} of ${totalStages}`,
          },
        },
        ...ONBOARDING_STAGES.map((s, idx) =>
          h('div', {
            key: s.id,
            attrs: {
              class: `kw-onboarding-progress-segment${
                idx === currentStageIndex
                  ? ' kw-progress-segment-active'
                  : idx < currentStageIndex
                  ? ' kw-progress-segment-completed'
                  : ''
              }`,
              title: `${s.label} (${idx + 1}/${totalStages})`,
            },
          }),
        ),
      ),
      // Optional Persistence Error Banner
      persistenceError
        ? AlertBanner({
            testId: 'onboarding-persistence-error-banner',
            tone: 'warning',
            title: 'Settings Notice',
            message: persistenceError,
          })
        : null,
      // Main Body
      h('div', { attrs: { class: 'kw-onboarding-body' } }, stageContent),
      // Footer Controls
      h(
        'div',
        { attrs: { class: 'kw-modal-footer kw-onboarding-footer' } },
        h(
          'div',
          { attrs: { class: 'kw-onboarding-footer-left' } },
          !isFirstStage
            ? Button({
                testId: 'onboarding-prev-btn',
                label: 'Previous',
                variant: 'secondary',
                size: 'sm',
                onClick: () => onStageChange(currentStageIndex - 1),
              })
            : null,
        ),
        h(
          'div',
          { attrs: { class: 'kw-onboarding-footer-right' } },
          !isLastStage
            ? Button({
                testId: 'onboarding-next-btn',
                label: currentStageIndex === 0 ? 'Next: Check Readiness' : 'Next',
                variant: 'primary',
                size: 'sm',
                onClick: () => onStageChange(currentStageIndex + 1),
              })
            : null,
          isLastStage
            ? Button({
                testId: 'onboarding-open-library-btn',
                label: 'Open Wallpaper Library',
                variant: 'primary',
                size: 'sm',
                onClick: onOpenLibrary,
              })
            : null,
          isLastStage
            ? Button({
                testId: 'onboarding-finish-btn',
                label: 'Finish Setup',
                variant: 'secondary',
                size: 'sm',
                onClick: onComplete,
              })
            : null,
        ),
      ),
    ),
  );
}

