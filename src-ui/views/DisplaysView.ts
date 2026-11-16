/**
 * Multi-Monitor Displays & Layout Management View (Step 46).
 * Integrates display discovery, visual arrangement, per-display assignment cards,
 * presentation layout modes, and accessible wallpaper assignment modal.
 */

import { AlertBanner, Badge, Button, Spinner } from '../components/primitives/controls.ts';
import { h, type UiNode } from '../components/primitives/uiNode.ts';
import { DisplayArrangementView } from '../components/domain/DisplayArrangementView.ts';
import { DisplayCard } from '../components/domain/DisplayCard.ts';
import { LayoutModeSelector } from '../components/domain/LayoutModeSelector.ts';
import { WallpaperPickerModal } from '../components/domain/WallpaperPickerModal.ts';
import type { CatalogStore } from '../stores/catalogStore.ts';
import type { DisplayStore } from '../stores/displayStore.ts';
import type { DisplayId } from '../types/display.ts';

export interface DisplaysViewProps {
  displayStore: DisplayStore;
  catalogStore: CatalogStore;
}

export function DisplaysView(props: DisplaysViewProps): UiNode {
  const { displayStore, catalogStore } = props;
  const state = displayStore.getState();
  const catalogState = catalogStore.getState();

  const displays = state.snapshot?.displays ?? [];
  const displayCount = displays.length;
  const targetDisplay = displays.find((d) => d.id === state.pickerTargetDisplayId) ?? null;

  // Header badges
  const activeSessionsCount = state.sessions.filter((s) => s.state === 'active').length;

  return h(
    'section',
    {
      attrs: {
        'data-testid': 'displays-view',
        'aria-labelledby': 'displays-page-heading',
        class: 'kw-displays-view',
      },
    },
    // Page Header
    h(
      'header',
      { attrs: { class: 'kw-page-header' } },
      h(
        'div',
        { attrs: { class: 'kw-page-header-titles' } },
        h(
          'h1',
          {
            attrs: {
              id: 'displays-page-heading',
              'data-testid': 'displays-heading',
              class: 'kw-page-title',
            },
          },
          'Displays & Layout',
        ),
        h(
          'p',
          { attrs: { class: 'kw-page-subtitle' } },
          'Configure multi-monitor presentation, wallpaper assignments, and display topology',
        ),
      ),
      h(
        'div',
        { attrs: { class: 'kw-page-header-actions' } },
        Badge({
          testId: 'displays-topology-badge',
          label: `${displayCount} ${displayCount === 1 ? 'Display' : 'Displays'} Detected`,
          iconSymbol: '🖥',
          tone: displayCount > 0 ? 'cyan' : 'warning',
        }),
        activeSessionsCount > 0
          ? Badge({
              testId: 'displays-active-sessions-badge',
              label: `${activeSessionsCount} Live Wallpaper${activeSessionsCount > 1 ? 's' : ''}`,
              iconSymbol: '●',
              tone: 'available',
            })
          : null,
        Button({
          testId: 'displays-refresh-btn',
          label: 'Refresh',
          variant: 'secondary',
          size: 'sm',
          isLoading: state.isLoading,
          onClick: () => {
            void displayStore.refreshTopology();
          },
        }),
        Button({
          testId: 'displays-apply-layout-btn',
          label: 'Apply Layout',
          variant: 'primary',
          size: 'sm',
          isLoading: state.isApplying,
          onClick: () => {
            void displayStore.applyLayout();
          },
        }),
      ),
    ),

    // Error Banner if present
    state.error
      ? AlertBanner({
          testId: 'displays-error-banner',
          tone: 'error',
          title: 'Display Subsystem Notice',
          message: state.error,
        })
      : null,

    // Loading State
    state.isLoading && displayCount === 0
      ? h(
          'div',
          { attrs: { class: 'kw-displays-loading', 'data-testid': 'displays-loading-state' } },
          Spinner({ size: 'lg', label: 'Detecting connected displays...' }),
        )
      : null,

    // Main Content
    h(
      'div',
      { attrs: { class: 'kw-displays-content' } },
      // 1. Presentation Mode Selector
      LayoutModeSelector({
        currentMode: state.savedMode,
        activeMode: state.activeMode,
        displayCount,
        disabled: state.isSaving || state.isApplying,
        onSelectMode: (mode) => {
          void displayStore.setLayoutMode(mode);
        },
      }),

      // 2. Visual Arrangement Canvas
      h(
        'section',
        {
          attrs: {
            class: 'kw-arrangement-section',
            'aria-label': 'Visual Display Arrangement',
          },
        },
        h('h3', { attrs: { class: 'kw-section-title' } }, 'Display Arrangement'),
        DisplayArrangementView({
          snapshot: state.snapshot,
          selectedDisplayId: state.selectedDisplayId,
          resolvedAssignments: state.resolvedAssignments,
          sessions: state.sessions,
          onSelectDisplay: (id: DisplayId) => {
            displayStore.selectDisplay(id);
          },
        }),
      ),

      // 3. Individual Display Cards Grid
      h(
        'section',
        {
          attrs: {
            class: 'kw-display-cards-section',
            'aria-label': 'Display List and Assignments',
          },
        },
        h('h3', { attrs: { class: 'kw-section-title' } }, 'Connected Monitors'),
        h(
          'div',
          {
            attrs: {
              class: 'kw-display-cards-grid',
              'data-testid': 'display-cards-grid',
            },
          },
          ...displays.map((display) => {
            const assignment = state.resolvedAssignments.find((a) => a.displayId === display.id);
            const session = state.sessions.find((s) => s.displayId === display.id);
            const policyDecision = state.policyDecisions[display.id];
            const isSelected = display.id === state.selectedDisplayId;

            return DisplayCard({
              display,
              assignment,
              session,
              policyDecision,
              isSelected,
              layoutMode: state.savedMode,
              disabled: state.isSaving || state.isApplying,
              onSelect: () => {
                displayStore.selectDisplay(display.id);
              },
              onChangeWallpaper: () => {
                displayStore.openWallpaperPicker(display.id);
              },
              onClearAssignment: () => {
                void displayStore.removeAssignment(display.id);
              },
            });
          }),
        ),
      ),

      // 4. Smart Playback & Fullscreen Pause Section
      h(
        'section',
        {
          attrs: {
            class: 'kw-policy-settings-section',
            'aria-label': 'Smart Playback Policy',
            'data-testid': 'smart-playback-policy-section',
          },
        },
        h('h3', { attrs: { class: 'kw-section-title' } }, 'Smart Playback Policy'),
        h(
          'div',
          {
            attrs: {
              class: 'kw-policy-card',
              style: 'padding: 16px; border-radius: 8px; background: rgba(18, 20, 31, 0.6); border: 1px solid rgba(255, 255, 255, 0.08); margin-top: 16px;',
            },
          },
          h(
            'div',
            {
              attrs: {
                style: 'display: flex; justify-content: space-between; align-items: center; margin-bottom: 12px;',
              },
            },
            h(
              'div',
              {},
              h('div', { attrs: { style: 'font-weight: 600; color: #FFFFFF;' } }, 'Smart Pause on Fullscreen Apps'),
              h(
                'div',
                { attrs: { style: 'font-size: 0.82rem; color: #8A8F98; margin-top: 2px;' } },
                'Automatically suspends live wallpaper rendering when a game or full-screen app is active to save GPU/CPU resources.',
              ),
            ),
            h(
              'button',
              {
                attrs: {
                  type: 'button',
                  role: 'switch',
                  'aria-checked': String(state.fullscreenPolicy.enabled),
                  'data-testid': 'toggle-smart-pause',
                  class: `kw-toggle-switch ${state.fullscreenPolicy.enabled ? 'kw-toggle-switch--active' : ''}`,
                  style: `width: 44px; height: 24px; border-radius: 12px; border: none; cursor: pointer; background: ${state.fullscreenPolicy.enabled ? '#00F0FF' : 'rgba(255, 255, 255, 0.2)'}; position: relative; transition: background 0.2s;`,
                },
                listeners: {
                  click: () => {
                    void displayStore.toggleSmartPauseFullscreen();
                  },
                },
              },
              h('span', {
                attrs: {
                  style: `position: absolute; top: 2px; ${state.fullscreenPolicy.enabled ? 'right: 2px;' : 'left: 2px;'} width: 20px; height: 20px; border-radius: 50%; background: #0A0B10; transition: all 0.2s;`,
                },
              }),
            ),
          ),

          h(
            'div',
            {
              attrs: {
                style: 'font-size: 0.75rem; color: #5A6072; display: flex; align-items: center; gap: 6px;',
              },
            },
            h('span', {}, '🔒 Privacy Notice: KinetiWall inspects only window boundaries and excluded shell classes. No window titles, typed text, or process logs are ever recorded.'),
          ),
        ),

        // Power-State Awareness & Battery Policy Card
        h(
          'div',
          {
            attrs: {
              class: 'kw-policy-card kw-power-policy-card',
              'data-testid': 'power-policy-card',
              style: 'padding: 16px; border-radius: 8px; background: rgba(18, 20, 31, 0.6); border: 1px solid rgba(255, 255, 255, 0.08); margin-top: 16px;',
            },
          },
          // Header with live power status badge
          h(
            'div',
            {
              attrs: {
                style: 'display: flex; justify-content: space-between; align-items: center; margin-bottom: 12px;',
              },
            },
            h(
              'div',
              {},
              h('div', { attrs: { style: 'font-weight: 600; color: #FFFFFF;' } }, 'Power-State & Battery-Sensitive Playback'),
              h(
                'div',
                { attrs: { style: 'font-size: 0.82rem; color: #8A8F98; margin-top: 2px;' } },
                'Adapts live wallpaper rendering when running on battery or when Windows Battery Saver is active.',
              ),
            ),
            h(
              'div',
              {
                attrs: {
                  'data-testid': 'power-status-badge',
                  style: 'padding: 4px 10px; border-radius: 12px; font-size: 0.78rem; font-weight: 600; background: rgba(0, 240, 255, 0.12); color: #00F0FF; border: 1px solid rgba(0, 240, 255, 0.3); display: flex; align-items: center; gap: 6px;',
                },
              },
              state.powerSnapshot.batteryAvailability === 'no_battery'
                ? '⚡ AC Power (Desktop)'
                : state.powerSnapshot.powerSource === 'battery'
                ? `🔋 Battery ${state.powerSnapshot.batteryPercent ?? '--'}%${state.powerSnapshot.batterySaver === 'enabled' ? ' (Saver Active)' : ''}`
                : `⚡ AC Charging ${state.powerSnapshot.batteryPercent ?? '--'}%`,
            ),
          ),

          // Policy Mode Selector: Off | Balanced | Save Battery
          h(
            'div',
            {
              attrs: {
                style: 'display: flex; gap: 8px; margin-top: 12px; margin-bottom: 12px;',
                'data-testid': 'power-mode-selector',
              },
            },
            ...(['off', 'balanced', 'save_battery'] as const).map((mode) => {
              const isSelected = state.powerSettings.mode === mode;
              const label = mode === 'off' ? 'Off (Standard)' : mode === 'balanced' ? 'Balanced' : 'Save Battery';
              return h(
                'button',
                {
                  attrs: {
                    type: 'button',
                    'data-testid': `power-mode-${mode}`,
                    style: `flex: 1; padding: 8px 12px; border-radius: 6px; font-size: 0.82rem; font-weight: 500; cursor: pointer; border: 1px solid ${isSelected ? '#00F0FF' : 'rgba(255, 255, 255, 0.1)'}; background: ${isSelected ? 'rgba(0, 240, 255, 0.15)' : 'rgba(255, 255, 255, 0.03)'}; color: ${isSelected ? '#00F0FF' : '#8A8F98'}; transition: all 0.2s;`,
                  },
                  listeners: {
                    click: () => {
                      void displayStore.setPowerPolicyMode(mode);
                    },
                  },
                },
                label,
              );
            }),
          ),

          // Trigger description based on current mode
          h(
            'div',
            {
              attrs: {
                style: 'font-size: 0.8rem; color: #8A8F98; line-height: 1.4; margin-bottom: 8px;',
              },
            },
            state.powerSettings.mode === 'off'
              ? 'Normal wallpaper playback continues uninterrupted under all power conditions.'
              : state.powerSettings.mode === 'balanced'
              ? 'Pauses wallpaper rendering when battery charge falls below 20% or Windows Battery Saver is engaged.'
              : 'Automatically pauses wallpaper rendering whenever host device is operating on DC battery power.',
          ),

          // Static Wallpaper Fallback status indicator
          h(
            'div',
            {
              attrs: {
                'data-testid': 'static-fallback-info',
                style: 'margin-top: 10px; margin-bottom: 12px; padding: 8px 12px; border-radius: 6px; background: rgba(0, 240, 255, 0.05); border: 1px solid rgba(0, 240, 255, 0.15); font-size: 0.8rem; color: #8A8F98; display: flex; align-items: center; justify-content: space-between;',
              },
            },
            h(
              'span',
              {},
              '🖼 Static Wallpaper Fallback: Retains poster or captured frame during power-saving pauses',
            ),
            h(
              'button',
              {
                attrs: {
                  type: 'button',
                  'data-testid': 'toggle-static-fallback-button',
                  style: `padding: 3px 8px; border-radius: 4px; font-size: 0.75rem; font-weight: 600; cursor: pointer; border: 1px solid ${state.powerSettings.enableStaticFallback ? '#00E5A3' : '#8A8F98'}; background: ${state.powerSettings.enableStaticFallback ? 'rgba(0, 229, 163, 0.15)' : 'rgba(255, 255, 255, 0.05)'}; color: ${state.powerSettings.enableStaticFallback ? '#00E5A3' : '#8A8F98'};`,
                },
                listeners: {
                  click: () => {
                    void displayStore.toggleStaticFallback();
                  },
                },
              },
              state.powerSettings.enableStaticFallback ? 'Enabled' : 'Disabled',
            ),
          ),

          // Adaptive Playback Quality & Performance Budgets
          h(
            'div',
            {
              attrs: {
                'data-testid': 'adaptive-quality-info',
                style: 'margin-bottom: 12px; padding: 8px 12px; border-radius: 6px; background: rgba(0, 240, 255, 0.05); border: 1px solid rgba(0, 240, 255, 0.15); font-size: 0.8rem; color: #8A8F98; display: flex; align-items: center; justify-content: space-between;',
              },
            },
            h(
              'span',
              {},
              `⚡ Adaptive Quality Optimization: Dynamically throttles render cadence under sustained resource pressure (Floor: ${state.adaptiveSettings.targetFpsFloor} FPS)`,
            ),
            h(
              'button',
              {
                attrs: {
                  type: 'button',
                  'data-testid': 'toggle-adaptive-quality-button',
                  style: `padding: 3px 8px; border-radius: 4px; font-size: 0.75rem; font-weight: 600; cursor: pointer; border: 1px solid ${state.adaptiveSettings.enabled ? '#00E5A3' : '#8A8F98'}; background: ${state.adaptiveSettings.enabled ? 'rgba(0, 229, 163, 0.15)' : 'rgba(255, 255, 255, 0.05)'}; color: ${state.adaptiveSettings.enabled ? '#00E5A3' : '#8A8F98'};`,
                },
                listeners: {
                  click: () => {
                    void displayStore.toggleAdaptiveQuality();
                  },
                },
              },
              state.adaptiveSettings.enabled ? 'Enabled' : 'Disabled (Default)',
            ),
          ),

          // Dynamic Resource Budgeting & Optimization Arbitration (Step 52)
          h(
            'div',
            {
              attrs: {
                'data-testid': 'resource-budget-info',
                style: 'margin-bottom: 12px; padding: 8px 12px; border-radius: 6px; background: rgba(0, 240, 255, 0.05); border: 1px solid rgba(0, 240, 255, 0.15); font-size: 0.8rem; color: #8A8F98; display: flex; align-items: center; justify-content: space-between;',
              },
            },
            h(
              'span',
              {},
              `📊 Resource Budget Arbiter: Coordinated multi-session load balancing (Target: ${state.resourceBudget.targetAggregateCpuPercent}% CPU, Status: ${state.budgetState})`,
            ),
            h(
              'button',
              {
                attrs: {
                  type: 'button',
                  'data-testid': 'toggle-resource-budget-button',
                  style: `padding: 3px 8px; border-radius: 4px; font-size: 0.75rem; font-weight: 600; cursor: pointer; border: 1px solid ${state.resourceBudget.enabled ? '#00E5A3' : '#8A8F98'}; background: ${state.resourceBudget.enabled ? 'rgba(0, 229, 163, 0.15)' : 'rgba(255, 255, 255, 0.05)'}; color: ${state.resourceBudget.enabled ? '#00E5A3' : '#8A8F98'};`,
                },
                listeners: {
                  click: () => {
                    void displayStore.toggleResourceBudget();
                  },
                },
              },
              state.resourceBudget.enabled ? 'Enabled' : 'Disabled (Default)',
            ),
          ),

          // Windows Session Lifecycle & Workstation Lock Awareness (Step 53)
          h(
            'div',
            {
              attrs: {
                'data-testid': 'session-policy-info',
                style: 'margin-bottom: 12px; padding: 8px 12px; border-radius: 6px; background: rgba(0, 240, 255, 0.05); border: 1px solid rgba(0, 240, 255, 0.15); font-size: 0.8rem; color: #8A8F98; display: flex; align-items: center; justify-content: space-between;',
              },
            },
            h(
              'span',
              {},
              `🔒 Session Lifecycle Awareness: Automatically pauses playback when workstation is locked (Lock: ${state.sessionSnapshot.lockState}, Session: ${state.sessionSnapshot.connectionState})`,
            ),
            h(
              'button',
              {
                attrs: {
                  type: 'button',
                  'data-testid': 'toggle-lock-pause-button',
                  style: `padding: 3px 8px; border-radius: 4px; font-size: 0.75rem; font-weight: 600; cursor: pointer; border: 1px solid ${state.sessionSettings.enabled ? '#00E5A3' : '#8A8F98'}; background: ${state.sessionSettings.enabled ? 'rgba(0, 229, 163, 0.15)' : 'rgba(255, 255, 255, 0.05)'}; color: ${state.sessionSettings.enabled ? '#00E5A3' : '#8A8F98'};`,
                },
                listeners: {
                  click: () => {
                    void displayStore.toggleLockPause();
                  },
                },
              },
              state.sessionSettings.enabled ? 'Enabled' : 'Disabled (Default)',
            ),
          ),

          // Windows Display-Power Awareness & Screen-Off Suspension (Step 54)
          h(
            'div',
            {
              attrs: {
                'data-testid': 'display-power-policy-info',
                style: 'margin-bottom: 12px; padding: 8px 12px; border-radius: 6px; background: rgba(0, 240, 255, 0.05); border: 1px solid rgba(0, 240, 255, 0.15); font-size: 0.8rem; color: #8A8F98; display: flex; align-items: center; justify-content: space-between;',
              },
            },
            h(
              'span',
              {},
              `🖥️ Display-Power Awareness: Suspends rendering when display powers off (Display: ${state.displayPowerSnapshot.state}, Scope: ${state.displayPowerSnapshot.scope})`,
            ),
            h(
              'button',
              {
                attrs: {
                  type: 'button',
                  'data-testid': 'toggle-display-power-button',
                  style: `padding: 3px 8px; border-radius: 4px; font-size: 0.75rem; font-weight: 600; cursor: pointer; border: 1px solid ${state.displayPowerSettings.enabled ? '#00E5A3' : '#8A8F98'}; background: ${state.displayPowerSettings.enabled ? 'rgba(0, 229, 163, 0.15)' : 'rgba(255, 255, 255, 0.05)'}; color: ${state.displayPowerSettings.enabled ? '#00E5A3' : '#8A8F98'};`,
                },
                listeners: {
                  click: () => {
                    void displayStore.toggleDisplayPowerPause();
                  },
                },
              },
              state.displayPowerSettings.enabled ? 'Enabled' : 'Disabled (Default)',
            ),
          ),

          // Windows User-Inactivity Awareness & Idle Suspension (Step 55)
          h(
            'div',
            {
              attrs: {
                'data-testid': 'inactivity-policy-info',
                style: 'margin-bottom: 12px; padding: 8px 12px; border-radius: 6px; background: rgba(0, 240, 255, 0.05); border: 1px solid rgba(0, 240, 255, 0.15); font-size: 0.8rem; color: #8A8F98; display: flex; align-items: center; justify-content: space-between;',
              },
            },
            h(
              'span',
              {},
              `⏳ User-Inactivity Awareness: Pauses playback on session idle (State: ${state.inactivitySnapshot.state}, Threshold: ${Math.round(state.inactivitySettings.idleThresholdMillis / 60000)}m)`,
            ),
            h(
              'button',
              {
                attrs: {
                  type: 'button',
                  'data-testid': 'toggle-inactivity-button',
                  style: `padding: 3px 8px; border-radius: 4px; font-size: 0.75rem; font-weight: 600; cursor: pointer; border: 1px solid ${state.inactivitySettings.enabled ? '#00E5A3' : '#8A8F98'}; background: ${state.inactivitySettings.enabled ? 'rgba(0, 229, 163, 0.15)' : 'rgba(255, 255, 255, 0.05)'}; color: ${state.inactivitySettings.enabled ? '#00E5A3' : '#8A8F98'};`,
                },
                listeners: {
                  click: () => {
                    void displayStore.toggleInactivityPause();
                  },
                },
              },
              state.inactivitySettings.enabled ? 'Enabled' : 'Disabled (Default)',
            ),
          ),

          h(
            'div',
            {
              attrs: {
                style: 'font-size: 0.75rem; color: #5A6072; display: flex; align-items: center; gap: 6px;',
              },
            },
            h('span', {}, '🔒 Privacy & Safety: Native Win32 power status is queried locally. No device telemetry, battery logs, or power-plan modifications are ever made.'),
          ),
        ),
      ),
    ),

    // Modal: Wallpaper Picker
    WallpaperPickerModal({
      isOpen: state.isPickerOpen,
      targetDisplay,
      wallpapers: catalogState.entries,
      isAssigning: state.isSaving,
      onSelectWallpaper: (wallpaperId) => {
        if (state.pickerTargetDisplayId) {
          void displayStore.assignWallpaper(state.pickerTargetDisplayId, wallpaperId);
        }
      },
      onClose: () => {
        displayStore.closeWallpaperPicker();
      },
    }),
  );
}

