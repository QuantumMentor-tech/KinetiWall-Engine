/**
 * Centralized Command Registry & Shortcut Dispatch Layer (Step 64).
 * Provides safe, conflict-detected, scope-aware keyboard shortcut routing.
 */

import {
  isInputElement,
  matchesKeyEvent,
  normalizeShortcutKeyCombo,
  type ApplicationCommand,
  type CommandCategory,
  type CommandScope,
  type ShortcutDispatchContext,
  type ShortcutKeyCombo,
} from '../types/shortcuts.ts';

export class CommandRegistry {
  private readonly commands = new Map<string, ApplicationCommand>();
  private attachedTarget: (Window | Document | HTMLElement) | null = null;
  private keydownHandler: ((e: Event) => void) | null = null;
  private activeContextSupplier?: () => ShortcutDispatchContext;

  constructor(activeContextSupplier?: () => ShortcutDispatchContext) {
    this.activeContextSupplier = activeContextSupplier;
  }

  /**
   * Registers a supported application command.
   * Throws if validation fails or an unresolvable shortcut conflict is detected.
   * Returns a cleanup unregister function.
   */
  public registerCommand(command: ApplicationCommand): () => void {
    const validation = this.validateCommand(command);
    if (!validation.valid) {
      throw new Error(`[CommandRegistry] Invalid command definition: ${validation.error}`);
    }

    const activeCombo = command.shortcut ?? command.defaultShortcut;
    const conflict = this.findConflictingCommand(activeCombo, command.scope ?? 'global', command.id);
    if (conflict) {
      throw new Error(
        `[CommandRegistry] Shortcut collision: Command '${command.id}' (${command.label}) conflicts with '${conflict.id}' (${conflict.label}) on shortcut '${normalizeShortcutKeyCombo(activeCombo)}'`,
      );
    }

    this.commands.set(command.id, {
      ...command,
      scope: command.scope ?? 'global',
      allowInInputs: Boolean(command.allowInInputs),
      allowRepeat: Boolean(command.allowRepeat),
    });

    return () => {
      this.unregisterCommand(command.id);
    };
  }

  /**
   * Unregisters a command by ID. Returns true if the command existed and was removed.
   */
  public unregisterCommand(commandId: string): boolean {
    return this.commands.delete(commandId);
  }

  public getCommand(commandId: string): ApplicationCommand | undefined {
    return this.commands.get(commandId);
  }

  public getAllCommands(): ApplicationCommand[] {
    return Array.from(this.commands.values());
  }

  public getCommandsByCategory(category: CommandCategory): ApplicationCommand[] {
    return this.getAllCommands().filter((cmd) => cmd.category === category);
  }

  /**
   * Validates structural invariants of a command.
   */
  public validateCommand(command: ApplicationCommand): { valid: boolean; error?: string } {
    if (!command.id || command.id.trim().length === 0) {
      return { valid: false, error: 'Command id cannot be empty' };
    }
    if (!command.label || command.label.trim().length === 0) {
      return { valid: false, error: 'Command label cannot be empty' };
    }
    if (typeof command.action !== 'function') {
      return { valid: false, error: 'Command must provide an action function' };
    }
    const combo = command.shortcut ?? command.defaultShortcut;
    if (!combo || !combo.key || combo.key.trim().length === 0) {
      return { valid: false, error: 'Command shortcut key cannot be empty' };
    }
    return { valid: true };
  }

  /**
   * Searches for another command whose shortcut collides in an overlapping scope.
   */
  public findConflictingCommand(
    combo: ShortcutKeyCombo,
    scope: CommandScope = 'global',
    excludeCommandId?: string,
  ): ApplicationCommand | undefined {
    const targetHash = normalizeShortcutKeyCombo(combo);

    for (const cmd of this.commands.values()) {
      if (excludeCommandId && cmd.id === excludeCommandId) {
        continue;
      }

      const existingCombo = cmd.shortcut ?? cmd.defaultShortcut;
      const existingHash = normalizeShortcutKeyCombo(existingCombo);

      if (targetHash === existingHash) {
        // Conflicting if both are global, or one is global and the other matches scope, or both share scope
        const existingScope = cmd.scope ?? 'global';
        if (
          existingScope === 'global' ||
          scope === 'global' ||
          existingScope === scope
        ) {
          return cmd;
        }
      }
    }

    return undefined;
  }

  /**
   * Evaluates and routes an incoming KeyboardEvent to a matching eligible command.
   * Returns true if a command was found and executed.
   */
  public dispatch(event: KeyboardEvent, context?: ShortcutDispatchContext): boolean {
    const resolvedContext: ShortcutDispatchContext = {
      ...(this.activeContextSupplier?.() ?? {}),
      ...(context ?? {}),
    };

    const target = (resolvedContext.targetElement ?? event.target) as EventTarget | null;
    const isInput = isInputElement(target);
    const isModalOpen = Boolean(resolvedContext.isModalOpen);
    const activeRoute = resolvedContext.activeRoute;

    for (const cmd of this.commands.values()) {
      const combo = cmd.shortcut ?? cmd.defaultShortcut;

      if (!matchesKeyEvent(combo, event)) {
        continue;
      }

      // 1. Text input protection
      if (isInput && !cmd.allowInInputs) {
        continue;
      }

      // 2. Modal dialog boundary protection
      // When a modal dialog is open, global commands are suppressed unless explicitly modal-scoped
      if (isModalOpen && cmd.scope !== 'modal' && !cmd.allowInInputs) {
        continue;
      }

      // 3. Scope restriction (e.g. only on 'library' route)
      if (cmd.scope && cmd.scope !== 'global' && cmd.scope !== 'modal') {
        if (activeRoute && cmd.scope !== activeRoute) {
          continue;
        }
      }

      // 4. Repeat suppression for non-repeatable commands
      if (event.repeat && !cmd.allowRepeat) {
        return false;
      }

      // 5. Availability condition
      if (cmd.isEnabled && !cmd.isEnabled()) {
        continue;
      }

      // Action execution
      try {
        event.preventDefault?.();
        void cmd.action();
        return true;
      } catch (err) {
        console.error(`[CommandRegistry] Execution error in command '${cmd.id}':`, err);
        return false;
      }
    }

    return false;
  }

  /**
   * Attaches the registry's keydown listener to the specified DOM window/target.
   * Returns a cleanup disposal function.
   */
  public attach(target: Window | Document | HTMLElement = typeof window !== 'undefined' ? window : ({} as any)): () => void {
    if (!target || typeof (target as EventTarget).addEventListener !== 'function') {
      return () => {};
    }

    this.detach();
    this.attachedTarget = target;
    this.keydownHandler = (e: Event) => {
      this.dispatch(e as KeyboardEvent);
    };

    target.addEventListener('keydown', this.keydownHandler as EventListener);

    return () => {
      this.detach();
    };
  }

  /**
   * Detaches the keydown event listener.
   */
  public detach(): void {
    if (this.attachedTarget && this.keydownHandler) {
      this.attachedTarget.removeEventListener('keydown', this.keydownHandler as EventListener);
      this.attachedTarget = null;
      this.keydownHandler = null;
    }
  }

  /**
   * Clears all registered commands and detaches listeners.
   */
  public clear(): void {
    this.detach();
    this.commands.clear();
  }
}
