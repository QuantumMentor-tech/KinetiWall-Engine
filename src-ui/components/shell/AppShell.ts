/**
 * Root Application Shell (`Step 22 KW-SHL-001` & `Step 27 §5.1`).
 *
 * Provides the persistent Neon Glass navigation rail, active-route indication (`aria-current="page"`),
 * disabled-state guardrails for unimplemented routes (`DEC-SHL-04`), top-level error boundary
 * containment (`Principle J`), and the main content viewport.
 */

import { AlertBanner, Button } from '../primitives/controls.ts';
import { h, type UiNode } from '../primitives/uiNode.ts';
import { NAVIGATION_ROUTES, type RouteId } from '../../routes.ts';
import { DisplayIpcClient, createMockDisplayIpcTransport } from '../../services/displayIpcClient.ts';
import { SettingsIpcClient, createMockSettingsIpcTransport } from '../../services/settingsIpcClient.ts';
import { CommandRegistry } from '../../services/commandRegistry.ts';
import type { CatalogStore } from '../../stores/catalogStore.ts';
import { DisplayStore } from '../../stores/displayStore.ts';
import type { RouterStore } from '../../stores/routerStore.ts';
import { SettingsStore } from '../../stores/settingsStore.ts';
import { ThemeStore } from '../../stores/themeStore.ts';
import { AccessibilityStore } from '../../stores/accessibilityStore.ts';
import { NotificationStore } from '../../stores/notificationStore.ts';
import { ReadinessEvaluator } from '../../services/readinessEvaluator.ts';
import { HelpModal } from '../domain/HelpModal.ts';
import type { HelpCategory, SafeActionId } from '../../types/help.ts';
import { announce } from '../../utils/announcer.ts';
import { KeyboardShortcutsModal } from '../domain/KeyboardShortcutsModal.ts';
import { OnboardingModal, ONBOARDING_STAGES } from '../domain/OnboardingModal.ts';
import { ToastContainer } from '../domain/ToastContainer.ts';
import { DashboardView } from '../../views/DashboardView.ts';
import { DisplaysView } from '../../views/DisplaysView.ts';
import { LibraryView } from '../../views/LibraryView.ts';
import { SettingsView } from '../../views/SettingsView.ts';

export interface AppShellProps {
  routerStore: RouterStore;
  catalogStore: CatalogStore;
  displayStore?: DisplayStore;
  settingsStore?: SettingsStore;
  themeStore?: ThemeStore;
  accessibilityStore?: AccessibilityStore;
  commandRegistry?: CommandRegistry;
  notificationStore?: NotificationStore;
  readinessEvaluator?: ReadinessEvaluator;
}

export class AppShellController {
  private readonly routerStore: RouterStore;
  private readonly catalogStore: CatalogStore;
  private readonly displayStore: DisplayStore;
  private readonly settingsStore: SettingsStore;
  private readonly themeStore: ThemeStore;
  private readonly accessibilityStore: AccessibilityStore;
  private readonly commandRegistry: CommandRegistry;
  private readonly notificationStore: NotificationStore;
  private readonly readinessEvaluator: ReadinessEvaluator;
  private isShortcutsModalOpen = false;
  private isOnboardingModalOpen = false;
  private isHelpModalOpen = false;
  private selectedHelpTopicId = 'help-getting-started';
  private helpSearchQuery = '';
  private helpCategoryFilter: HelpCategory | 'all' = 'all';
  private onboardingStageIndex = 0;
  private isReadinessLoading = false;
  private onboardingPersistenceError: string | null = null;
  private hasEvaluatedInitialOnboarding = false;
  private viewError: string | null = null;

  constructor(props: AppShellProps) {
    this.routerStore = props.routerStore;
    this.catalogStore = props.catalogStore;
    this.displayStore =
      props.displayStore ?? new DisplayStore(new DisplayIpcClient(createMockDisplayIpcTransport()));
    this.settingsStore =
      props.settingsStore ?? new SettingsStore(new SettingsIpcClient(createMockSettingsIpcTransport()));
    this.themeStore = props.themeStore ?? new ThemeStore();
    this.accessibilityStore = props.accessibilityStore ?? new AccessibilityStore();
    this.notificationStore = props.notificationStore ?? new NotificationStore();
    this.readinessEvaluator =
      props.readinessEvaluator ??
      new ReadinessEvaluator({
        settingsStore: this.settingsStore,
        catalogStore: this.catalogStore,
        displayStore: this.displayStore,
      });
    this.commandRegistry =
      props.commandRegistry ??
      new CommandRegistry(() => ({
        activeRoute: this.routerStore.getState().activeRoute,
        isModalOpen:
          this.isShortcutsModalOpen || this.isOnboardingModalOpen || this.isHelpModalOpen,
      }));

    this.registerDefaultCommands();

    // Attach event listener in DOM environments
    if (typeof window !== 'undefined' && typeof window.addEventListener === 'function') {
      this.commandRegistry.attach(window);
    }

    // Synchronize initial preferences if settings already loaded
    const initialSettings = this.settingsStore.getState().persistedConfig;
    if (initialSettings) {
      this.themeStore.syncWithSettings(initialSettings);
      this.accessibilityStore.syncWithSettings(initialSettings);
      this.notificationStore.syncWithSettings(initialSettings);
      if (initialSettings.general.onboarding?.status === 'not_started') {
        this.hasEvaluatedInitialOnboarding = true;
        this.isOnboardingModalOpen = true;
      }
    }

    // Apply initial DOM attributes
    this.themeStore.applyToDom();
    this.accessibilityStore.applyToDom();

    // Subscribe to settings changes to synchronize theme, accessibility, and notifications
    this.settingsStore.subscribe((settingsState) => {
      if (settingsState.persistedConfig) {
        this.themeStore.syncWithSettings(settingsState.persistedConfig);
        this.accessibilityStore.syncWithSettings(settingsState.persistedConfig);
        this.notificationStore.syncWithSettings(settingsState.persistedConfig);
        if (
          settingsState.persistedConfig.general.onboarding?.status === 'not_started' &&
          !this.hasEvaluatedInitialOnboarding
        ) {
          this.hasEvaluatedInitialOnboarding = true;
          this.isOnboardingModalOpen = true;
        }
      }
      if (settingsState.draftConfig) {
        if (settingsState.draftConfig.general.theme) {
          this.themeStore.setPreference(settingsState.draftConfig.general.theme);
        }
        if (settingsState.draftConfig.general.reduced_motion) {
          this.accessibilityStore.setPreference(settingsState.draftConfig.general.reduced_motion);
        }
        if (settingsState.draftConfig.general.notifications) {
          this.notificationStore.setPreference(settingsState.draftConfig.general.notifications);
        }
      }
    });

    // If launched directly on the library route, trigger initial catalog load once if still idle
    if (
      this.routerStore.getState().activeRoute === 'library' &&
      this.catalogStore.getState().status === 'idle'
    ) {
      void this.catalogStore.loadCatalog();
    }

    // If launched directly on the displays route, trigger initial display topology discovery
    if (
      this.routerStore.getState().activeRoute === 'displays' &&
      !this.displayStore.getState().snapshot
    ) {
      void this.displayStore.refreshTopology();
    }

    // If launched directly on the settings route, trigger initial settings load
    if (
      this.routerStore.getState().activeRoute === 'settings' &&
      !this.settingsStore.getState().persistedConfig
    ) {
      void this.settingsStore.loadConfig();
    }

    // Automatically ensure store data is loaded when navigating
    this.routerStore.subscribe((routerState) => {
      if (
        routerState.activeRoute === 'library' &&
        this.catalogStore.getState().status === 'idle'
      ) {
        void this.catalogStore.loadCatalog();
      } else if (
        routerState.activeRoute === 'displays' &&
        !this.displayStore.getState().snapshot
      ) {
        void this.displayStore.refreshTopology();
      } else if (
        routerState.activeRoute === 'settings' &&
        !this.settingsStore.getState().persistedConfig
      ) {
        void this.settingsStore.loadConfig();
      }
    });
  }

  navigate(routeId: RouteId): boolean {
    this.viewError = null;
    return this.routerStore.navigate(routeId);
  }

  getThemeStore(): ThemeStore {
    return this.themeStore;
  }

  getAccessibilityStore(): AccessibilityStore {
    return this.accessibilityStore;
  }

  getCommandRegistry(): CommandRegistry {
    return this.commandRegistry;
  }

  getNotificationStore(): NotificationStore {
    return this.notificationStore;
  }

  getReadinessEvaluator(): ReadinessEvaluator {
    return this.readinessEvaluator;
  }

  openOnboardingModal(stageIndex = 0): void {
    this.onboardingStageIndex = Math.max(0, Math.min(stageIndex, ONBOARDING_STAGES.length - 1));
    this.isOnboardingModalOpen = true;
    this.onboardingPersistenceError = null;
    void this.readinessEvaluator.checkReadiness(false);
  }

  closeOnboardingModal(): void {
    this.isOnboardingModalOpen = false;
  }

  isOnboardingOpen(): boolean {
    return this.isOnboardingModalOpen;
  }

  handleOnboardingStageChange(index: number): void {
    const clamped = Math.max(0, Math.min(index, ONBOARDING_STAGES.length - 1));
    this.onboardingStageIndex = clamped;
    const stageId = ONBOARDING_STAGES[clamped]?.id ?? 'welcome';
    const config = this.settingsStore.getState().draftConfig;
    if (
      config &&
      config.general.onboarding.status !== 'completed' &&
      config.general.onboarding.status !== 'dismissed'
    ) {
      this.settingsStore.updateDraftField('general', 'onboarding', {
        ...config.general.onboarding,
        status: 'in_progress',
        last_completed_step: stageId,
      });
    }
  }

  async handleOnboardingRefreshReadiness(): Promise<void> {
    this.isReadinessLoading = true;
    try {
      await this.readinessEvaluator.checkReadiness(true);
      announce('System capability readiness check completed.');
    } finally {
      this.isReadinessLoading = false;
    }
  }

  async handleOnboardingComplete(): Promise<boolean> {
    this.onboardingPersistenceError = null;
    const currentOnboarding = this.settingsStore.getState().draftConfig?.general.onboarding;
    this.settingsStore.updateDraftField('general', 'onboarding', {
      version: currentOnboarding?.version ?? 1,
      status: 'completed',
      last_completed_step: 'completion',
      completed_at_millis: Date.now(),
    });
    const ok = await this.settingsStore.saveChanges();
    if (ok) {
      this.isOnboardingModalOpen = false;
      this.notificationStore.success('Setup completed! Welcome to KinetiWall.', {
        title: 'Setup Complete',
        reasonCode: 'onboarding.completed',
      });
      announce('First-run setup completed. Welcome to KinetiWall.');
      return true;
    } else {
      this.onboardingPersistenceError = 'Failed to persist setup completion to disk. Preferences will be retried.';
      this.notificationStore.error('Could not save onboarding completion to disk.', {
        title: 'Save Failed',
        reasonCode: 'onboarding.save_failed',
      });
      return false;
    }
  }

  async handleOnboardingDismiss(): Promise<boolean> {
    this.onboardingPersistenceError = null;
    const currentOnboarding = this.settingsStore.getState().draftConfig?.general.onboarding;
    const stageId = ONBOARDING_STAGES[this.onboardingStageIndex]?.id ?? 'welcome';
    this.settingsStore.updateDraftField('general', 'onboarding', {
      version: currentOnboarding?.version ?? 1,
      status: 'dismissed',
      last_completed_step: stageId,
      completed_at_millis: null,
    });
    const ok = await this.settingsStore.saveChanges();
    this.isOnboardingModalOpen = false;
    if (ok) {
      this.notificationStore.info('Onboarding guide dismissed. Reopen anytime in Settings.', {
        title: 'Guide Dismissed',
        reasonCode: 'onboarding.dismissed',
      });
      announce('Onboarding guide dismissed. You can reopen it anytime in Settings.');
      return true;
    } else {
      this.onboardingPersistenceError = 'Failed to persist dismissal to disk.';
      return false;
    }
  }

  openShortcutsModal(): void {
    this.isShortcutsModalOpen = true;
  }

  closeShortcutsModal(): void {
    this.isShortcutsModalOpen = false;
  }

  isShortcutsOpen(): boolean {
    return this.isShortcutsModalOpen;
  }

  openHelpModal(topicId?: string): void {
    if (topicId) {
      this.selectedHelpTopicId = topicId;
    }
    this.isHelpModalOpen = true;
  }

  closeHelpModal(): void {
    this.isHelpModalOpen = false;
  }

  isHelpOpen(): boolean {
    return this.isHelpModalOpen;
  }

  executeSafeAction(actionId: SafeActionId): void {
    this.closeHelpModal();

    switch (actionId) {
      case 'nav.dashboard':
        this.navigate('dashboard');
        break;
      case 'nav.library':
        this.navigate('library');
        break;
      case 'nav.displays':
        this.navigate('displays');
        break;
      case 'nav.settings':
        this.navigate('settings');
        break;
      case 'library.refresh':
        void this.catalogStore.loadCatalog();
        announce('Refreshing wallpaper catalog');
        break;
      case 'displays.refresh':
        void this.displayStore.refreshTopology();
        announce('Refreshing display topology');
        break;
      case 'onboarding.reopen':
        this.openOnboardingModal(0);
        break;
      case 'readiness.recheck':
        this.navigate('settings');
        this.openOnboardingModal(1);
        break;
      case 'help.shortcuts':
        this.openShortcutsModal();
        break;
      default:
        break;
    }
  }

  dispose(): void {
    this.commandRegistry.detach();
    this.notificationStore.dispose();
  }

  private registerDefaultCommands(): void {
    // 1. Navigation Commands
    this.commandRegistry.registerCommand({
      id: 'nav.dashboard',
      label: 'Go to Dashboard',
      description: 'Navigate to Engine Overview and status dashboard',
      category: 'navigation',
      defaultShortcut: { key: '1', code: 'Digit1', alt: true },
      action: () => {
        this.navigate('dashboard');
        announce('Navigated to Dashboard');
      },
    });

    this.commandRegistry.registerCommand({
      id: 'nav.library',
      label: 'Go to Wallpaper Library',
      description: 'Browse registered wallpapers and inspect asset availability',
      category: 'navigation',
      defaultShortcut: { key: '2', code: 'Digit2', alt: true },
      action: () => {
        this.navigate('library');
        announce('Navigated to Wallpaper Library');
      },
    });

    this.commandRegistry.registerCommand({
      id: 'nav.displays',
      label: 'Go to Displays',
      description: 'Manage multi-monitor layouts and wallpaper assignments',
      category: 'navigation',
      defaultShortcut: { key: '3', code: 'Digit3', alt: true },
      action: () => {
        this.navigate('displays');
        announce('Navigated to Displays & Layout');
      },
    });

    this.commandRegistry.registerCommand({
      id: 'nav.settings',
      label: 'Go to Settings',
      description: 'Configure engine behavior, power policies, and preferences',
      category: 'navigation',
      defaultShortcut: { key: '4', code: 'Digit4', alt: true },
      action: () => {
        this.navigate('settings');
        announce('Navigated to Application Settings');
      },
    });

    // 2. Library & Search Commands
    this.commandRegistry.registerCommand({
      id: 'library.search_focus',
      label: 'Focus Library Search',
      description: 'Focus the search query input in wallpaper library',
      category: 'library',
      defaultShortcut: { key: 'f', code: 'KeyF', ctrl: true },
      action: () => {
        if (this.routerStore.getState().activeRoute !== 'library') {
          this.navigate('library');
        }
        if (typeof document !== 'undefined') {
          const input = (document.getElementById('library-search-field') ??
            document.getElementById('library-search-input')) as HTMLInputElement | null;
          input?.focus();
          input?.select();
        }
      },
    });

    this.commandRegistry.registerCommand({
      id: 'library.refresh',
      label: 'Refresh Wallpaper Library',
      description: 'Reload local catalog records and re-verify asset availability',
      category: 'library',
      scope: 'library',
      defaultShortcut: { key: 'F5' },
      action: () => {
        void this.catalogStore.loadCatalog();
        announce('Refreshing wallpaper catalog');
      },
    });

    // 3. Displays Commands
    this.commandRegistry.registerCommand({
      id: 'displays.refresh',
      label: 'Refresh Display Topology',
      description: 'Re-discover connected monitors and update layout bounds',
      category: 'displays',
      scope: 'displays',
      defaultShortcut: { key: 'r', code: 'KeyR', ctrl: true, shift: true },
      action: () => {
        void this.displayStore.refreshTopology();
        announce('Refreshing display topology');
      },
    });

    // 4. Settings Commands
    this.commandRegistry.registerCommand({
      id: 'settings.save',
      label: 'Save Settings Changes',
      description: 'Persist modified preference draft to disk',
      category: 'settings',
      scope: 'settings',
      defaultShortcut: { key: 's', code: 'KeyS', ctrl: true },
      isEnabled: () => this.settingsStore.getState().isDirty,
      action: async () => {
        const ok = await this.settingsStore.saveChanges();
        if (ok) {
          this.notificationStore.success('Application preferences saved to disk.', {
            title: 'Settings Saved',
            reasonCode: 'settings.saved',
          });
        } else {
          this.notificationStore.error('Failed to save settings. Please inspect validation errors.', {
            title: 'Save Failed',
            reasonCode: 'settings.save_failed',
          });
        }
      },
    });

    this.commandRegistry.registerCommand({
      id: 'settings.discard',
      label: 'Discard Settings Draft',
      description: 'Revert uncommitted preference changes to last saved state',
      category: 'settings',
      scope: 'settings',
      defaultShortcut: { key: 'z', code: 'KeyZ', ctrl: true, shift: true },
      isEnabled: () => this.settingsStore.getState().isDirty,
      action: () => {
        this.settingsStore.discardChanges();
        this.notificationStore.info('Unsaved preference changes discarded.', {
          title: 'Changes Discarded',
          reasonCode: 'settings.discarded',
        });
      },
    });

    // 5. General & Help Commands
    this.commandRegistry.registerCommand({
      id: 'help.shortcuts',
      label: 'Show Keyboard Shortcuts',
      description: 'Open the keyboard shortcuts reference dialog',
      category: 'general',
      defaultShortcut: { key: '?' },
      action: () => {
        this.openShortcutsModal();
      },
    });

    this.commandRegistry.registerCommand({
      id: 'help.main',
      label: 'Open Help & Troubleshooting',
      description: 'Open the in-app help catalogue and troubleshooting guide',
      category: 'general',
      defaultShortcut: { key: 'F1' },
      action: () => {
        this.openHelpModal();
      },
    });

    this.commandRegistry.registerCommand({
      id: 'help.onboarding',
      label: 'Show Guided Onboarding',
      description: 'Open the first-run guided tour and system readiness inspector',
      category: 'general',
      defaultShortcut: { key: 'h', code: 'KeyH', ctrl: true, shift: true },
      action: () => {
        this.openOnboardingModal(0);
      },
    });

    this.commandRegistry.registerCommand({
      id: 'dialog.close',
      label: 'Close Dialog / Panel',
      description: 'Dismiss the active modal dialog or inspector panel',
      category: 'general',
      scope: 'modal',
      defaultShortcut: { key: 'Escape' },
      allowInInputs: true,
      action: () => {
        if (this.isHelpModalOpen) {
          this.closeHelpModal();
        } else if (this.isShortcutsModalOpen) {
          this.closeShortcutsModal();
        } else if (this.isOnboardingModalOpen) {
          void this.handleOnboardingDismiss();
        }
      },
    });
  }

  render(): UiNode {
    const routerState = this.routerStore.getState();
    const catalogState = this.catalogStore.getState();

    let mainViewNode: UiNode;
    try {
      if (this.viewError) {
        throw new Error(this.viewError);
      }
      if (routerState.activeRoute === 'library') {
        mainViewNode = LibraryView({
          store: this.catalogStore,
          state: catalogState,
          viewportWidthPx: routerState.viewportWidthPx,
        });
      } else if (routerState.activeRoute === 'displays') {
        mainViewNode = DisplaysView({
          displayStore: this.displayStore,
          catalogStore: this.catalogStore,
          onOpenHelp: (topicId) => this.openHelpModal(topicId),
        });
      } else if (routerState.activeRoute === 'settings') {
        mainViewNode = SettingsView({
          settingsStore: this.settingsStore,
          onReopenOnboarding: () => this.openOnboardingModal(0),
          onOpenHelp: (topicId) => this.openHelpModal(topicId),
        });
      } else {
        mainViewNode = DashboardView({
          routerStore: this.routerStore,
          catalogState,
          onAddWallpaper: () => {
            this.routerStore.navigate('library');
            void this.catalogStore.triggerRegistration();
          },
        });
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      mainViewNode = AlertBanner({
        testId: 'shell-error-boundary',
        tone: 'error',
        title: 'View Rendering Error',
        message: msg,
        actionButton: Button({
          testId: 'shell-error-reset-btn',
          label: 'Reset View',
          variant: 'primary',
          size: 'sm',
          onClick: () => {
            this.viewError = null;
            this.routerStore.navigate('dashboard');
          },
        }),
      });
    }

    const themeState = this.themeStore.getState();
    const a11yState = this.accessibilityStore.getState();

    return h(
      'div',
      {
        attrs: {
          'data-testid': 'kinetiwall-app-shell',
          'data-active-route': routerState.activeRoute,
          'data-viewport-width': String(routerState.viewportWidthPx),
          'data-theme': themeState.effectiveTheme,
          'data-theme-preference': themeState.preference,
          'data-reduced-motion': a11yState.effectiveReducedMotion ? 'true' : 'false',
          'data-motion-preference': a11yState.reducedMotionPreference,
          class: 'kw-shell',
        },
      },
      // Left Navigation Rail
      h(
        'nav',
        {
          attrs: {
            role: 'navigation',
            'aria-label': 'Primary Navigation',
            'data-testid': 'shell-nav-rail',
            class: 'kw-nav-rail',
          },
        },
        h(
          'div',
          { attrs: { class: 'kw-brand-header', 'data-testid': 'shell-brand-header' } },
          h('span', { attrs: { class: 'kw-brand-mark', 'aria-hidden': 'true' } }, '◆'),
          h('span', { attrs: { class: 'kw-brand-name' } }, 'KinetiWall'),
        ),
        h(
          'ul',
          { attrs: { class: 'kw-nav-list', role: 'list' } },
          ...NAVIGATION_ROUTES.map((route, idx) => {
            const isActive = routerState.activeRoute === route.id;
            const isDisabled = !route.isAvailable;
            const shortcutHint = `Alt+${idx + 1}`;
            return h(
              'li',
              { key: route.id, attrs: { class: 'kw-nav-item' } },
              h(
                'button',
                {
                  attrs: {
                    type: 'button',
                    'data-testid': `nav-item-${route.id}`,
                    'data-route-id': route.id,
                    'data-active': isActive ? 'true' : 'false',
                    class: `kw-nav-btn kw-focus-ring${isActive ? ' kw-nav-btn-active' : ''}${
                      isDisabled ? ' kw-nav-btn-disabled' : ''
                    }`,
                    disabled: isDisabled,
                    'aria-disabled': isDisabled ? 'true' : undefined,
                    'aria-current': isActive ? 'page' : undefined,
                    'aria-label': isDisabled
                      ? `${route.label} (Unavailable: ${route.unavailableReason})`
                      : `${route.label} (${shortcutHint})`,
                    title: isDisabled ? route.unavailableReason : `${route.shortDescription} (${shortcutHint})`,
                  },
                  onClick: isDisabled
                    ? undefined
                    : () => {
                        this.navigate(route.id);
                      },
                  onKeyDown: isDisabled
                    ? undefined
                    : (e) => {
                        if (e.key === 'Enter' || e.key === ' ') {
                          e.preventDefault?.();
                          this.navigate(route.id);
                        }
                      },
                },
                h('span', { attrs: { class: 'kw-nav-label' } }, route.label),
                h('span', { attrs: { class: 'kw-nav-shortcut-hint', 'aria-hidden': 'true' } }, shortcutHint),
              ),
            );
          }),
        ),
        // Help & Troubleshooting Button in Nav Footer
        h(
          'button',
          {
            attrs: {
              type: 'button',
              'data-testid': 'nav-help-btn',
              class: 'kw-shortcuts-footer-btn kw-focus-ring',
              'aria-label': 'Help & Troubleshooting (press F1)',
              title: 'Help & Troubleshooting (press F1)',
            },
            onClick: () => {
              this.openHelpModal();
            },
          },
          h('span', { attrs: { 'aria-hidden': 'true' } }, '❓'),
          h('span', {}, 'Help'),
          h('span', { attrs: { class: 'kw-nav-shortcut-hint', 'aria-hidden': 'true' } }, 'F1'),
        ),
        // Shortcuts Reference Button in Nav Footer
        h(
          'button',
          {
            attrs: {
              type: 'button',
              'data-testid': 'nav-shortcuts-btn',
              class: 'kw-shortcuts-footer-btn kw-focus-ring',
              'aria-label': 'Keyboard Shortcuts (press ?)',
              title: 'Keyboard Shortcuts (press ?)',
            },
            onClick: () => {
              this.openShortcutsModal();
            },
          },
          h('span', { attrs: { 'aria-hidden': 'true' } }, '⌨'),
          h('span', {}, 'Shortcuts'),
          h('span', { attrs: { class: 'kw-nav-shortcut-hint', 'aria-hidden': 'true' } }, '?'),
        ),
      ),
      // Main Content Viewport
      h(
        'main',
        {
          attrs: {
            role: 'main',
            'data-testid': 'shell-main-viewport',
            class: 'kw-main-viewport',
          },
        },
        mainViewNode,
      ),
      // Help & Troubleshooting Dialog Modal (Step 67)
      HelpModal({
        isOpen: this.isHelpModalOpen,
        selectedTopicId: this.selectedHelpTopicId,
        searchQuery: this.helpSearchQuery,
        categoryFilter: this.helpCategoryFilter,
        onClose: () => this.closeHelpModal(),
        onSelectTopic: (topicId) => {
          this.selectedHelpTopicId = topicId;
        },
        onSearchChange: (q) => {
          this.helpSearchQuery = q;
        },
        onCategoryChange: (cat) => {
          this.helpCategoryFilter = cat;
        },
        onExecuteAction: (actionId) => this.executeSafeAction(actionId),
      }),
      // Keyboard Shortcuts Dialog Modal
      KeyboardShortcutsModal({
        isOpen: this.isShortcutsModalOpen,
        registry: this.commandRegistry,
        onClose: () => {
          this.closeShortcutsModal();
        },
      }),
      // Guided Onboarding Dialog Modal (Step 66)
      OnboardingModal({
        isOpen: this.isOnboardingModalOpen,
        currentStageIndex: this.onboardingStageIndex,
        report: this.readinessEvaluator.getLatestReport(),
        isReadinessLoading: this.isReadinessLoading,
        persistenceError: this.onboardingPersistenceError,
        onStageChange: (idx) => this.handleOnboardingStageChange(idx),
        onRefreshReadiness: () => void this.handleOnboardingRefreshReadiness(),
        onComplete: () => void this.handleOnboardingComplete(),
        onDismiss: () => void this.handleOnboardingDismiss(),
        onOpenLibrary: () => {
          void this.handleOnboardingComplete();
          this.navigate('library');
        },
      }),
      // In-App Notifications Toast Container
      ToastContainer({
        notifications: this.notificationStore.getState(),
        onDismiss: (id) => this.notificationStore.dismiss(id),
      }),
    );
  }
}
