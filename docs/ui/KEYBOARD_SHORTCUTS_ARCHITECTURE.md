# KinetiWall Keyboard Shortcut Architecture & Command Registry (Step 64)

## 1. Executive Summary & Design Mandate

Step 64 establishes an authoritative, conflict-safe, accessible keyboard shortcut architecture for KinetiWall. It enables desktop users and assistive technology operators to discover and execute supported application commands swiftly and reliably, without compromising text entry in form fields, interfering with native Windows editing shortcuts, bypassing playback policy arbitration, or introducing hazardous global hotkey hooks.

### Core Architectural Principles
1. **Centralized Command Registry (`CommandRegistry`)**:
   - Single point of truth for application commands, categories, scopes, default key combinations, and availability conditions.
   - Strictly validates command definitions and detects collisions in overlapping scopes at registration time.
2. **Authoritative Action Path Routing**:
   - Keyboard shortcuts NEVER bypass established application abstractions, policy arbitrations, or state managers.
   - Shortcut actions route directly to the identical methods invoked by UI buttons (e.g. `RouterStore.navigate()`, `CatalogStore.loadCatalog()`, `DisplayStore.refreshTopology()`, `SettingsStore.saveChanges()`).
3. **Strict Text-Entry Protection**:
   - Global shortcuts are strictly inhibited when focus is within `<input>`, `<textarea>`, `<select>`, or contenteditable elements unless the command explicitly opts in (`allowInInputs: true`, e.g. `Escape`).
   - Native OS editing combinations (`Ctrl+A`, `Ctrl+C`, `Ctrl+V`, `Ctrl+X`, `Ctrl+Z`, `Ctrl+Y`, arrows, backspace, delete) remain 100% unintercepted.
4. **Modal Dialog Boundary Containment**:
   - Active modal dialogs (`ConfirmationDialog`, `WallpaperPickerModal`, `KeyboardShortcutsModal`) suppress underlying global navigation shortcuts.
   - `Escape` dismisses the topmost active dialog; `Tab` / `Shift+Tab` cycles within the modal boundary via `trapFocus`.
5. **Key Repeat Suppression**:
   - Held-down keys do not trigger repeated executions of high-impact or destructive actions (such as navigation, refresh, or settings save).
6. **Discoverability & Screen Reader Accessibility**:
   - Visual shortcut hints (`Alt+1`..`Alt+4`) are displayed on navigation buttons.
   - A dedicated `KeyboardShortcutsModal` reference sheet is accessible via `?`, `F1`, or the navigation rail footer button.
   - State transitions and actions are announced politely to screen readers via `ScreenReaderAnnouncer`.

---

## 2. Authoritative Supported Commands & Default Shortcut Inventory

| Command ID | Label | Default Key | Scope | Allow In Inputs | Category | Established Action |
|:---|:---|:---|:---|:---:|:---|:---|
| `nav.dashboard` | Go to Dashboard | `Alt+1` | Global | ❌ | Navigation | `RouterStore.navigate('dashboard')` |
| `nav.library` | Go to Wallpaper Library | `Alt+2` | Global | ❌ | Navigation | `RouterStore.navigate('library')` |
| `nav.displays` | Go to Displays | `Alt+3` | Global | ❌ | Navigation | `RouterStore.navigate('displays')` |
| `nav.settings` | Go to Settings | `Alt+4` | Global | ❌ | Navigation | `RouterStore.navigate('settings')` |
| `library.search_focus` | Focus Library Search | `Ctrl+F` | Global | ❌ | Library | Navigates to library & focuses `#library-search-field` |
| `library.refresh` | Refresh Wallpaper Library | `F5` | Library | ❌ | Library | `CatalogStore.loadCatalog()` |
| `displays.refresh` | Refresh Display Topology | `Ctrl+Shift+R` | Displays | ❌ | Displays | `DisplayStore.refreshTopology()` |
| `settings.save` | Save Settings Changes | `Ctrl+S` | Settings | ❌ | Settings | `SettingsStore.saveChanges()` (when dirty) |
| `settings.discard` | Discard Settings Draft | `Ctrl+Shift+Z` | Settings | ❌ | Settings | `SettingsStore.discardChanges()` (when dirty) |
| `help.shortcuts` | Show Keyboard Shortcuts | `?` | Global | ❌ | General | Opens `KeyboardShortcutsModal` |
| `help.shortcuts_f1` | Show Help & Shortcuts | `F1` | Global | ❌ | General | Opens `KeyboardShortcutsModal` |
| `dialog.close` | Close Dialog / Panel | `Escape` | Modal | ✅ | General | Dismisses active modal dialog |

---

## 3. Data Model & Canonical Matching (`src-ui/types/shortcuts.ts`)

### 3.1 Data Types

```typescript
export interface ShortcutKeyCombo {
  key: string;       // Normalized key name: '1', 'f', 'r', 's', '?', 'Escape', 'F1'
  code?: string;     // Physical code fallback: 'Digit1', 'KeyF', 'KeyR'
  ctrl?: boolean;
  alt?: boolean;
  shift?: boolean;
  meta?: boolean;
}

export interface ApplicationCommand {
  id: string;
  label: string;
  description: string;
  category: CommandCategory;
  defaultShortcut: ShortcutKeyCombo;
  shortcut?: ShortcutKeyCombo;
  scope?: CommandScope;
  allowInInputs?: boolean;
  allowRepeat?: boolean;
  action: () => void | Promise<void>;
  isEnabled?: () => boolean;
}
```

### 3.2 Canonical Normalization & Conflict Resolution

Key combinations are hashed into canonical uppercase strings (`[CTRL+][ALT+][SHIFT+][META+]<KEY>`):
```typescript
normalizeShortcutKeyCombo({ key: '1', code: 'Digit1', alt: true }) // => "ALT+1"
normalizeShortcutKeyCombo({ key: 'r', ctrl: true, shift: true })   // => "CTRL+SHIFT+R"
```
When registering a command, `CommandRegistry` tests for collisions against existing commands in overlapping scopes:
- Two global commands cannot share the same key combination.
- A global command and a view-scoped command cannot share the same key combination.
- Two commands in mutually exclusive view scopes (e.g., `library` vs `settings`) MAY share a combination if desired.

---

## 4. Dispatch Flow & Protection Guardrails

When a DOM `keydown` event occurs:

```
[DOM keydown event]
         │
         ▼
[isInputElement(target)?] ──Yes──► [cmd.allowInInputs == true?] ──No──► [Pass through to browser / input]
         │ No                                      │ Yes
         ▼                                         ▼
[isModalOpen == true?] ──Yes──► [cmd.scope == 'modal'?] ──No──► [Suppress global shortcut]
         │ No                                      │ Yes
         ▼                                         ▼
[event.repeat == true?] ──Yes──► [cmd.allowRepeat == true?] ──No──► [Ignore held-down key repeat]
         │ No                                      │ Yes
         ▼                                         ▼
[Scope check: activeRoute matches cmd.scope?] ──No──► [Skip command]
         │ Yes
         ▼
[cmd.isEnabled()? == true?] ──No──► [Skip command]
         │ Yes
         ▼
[event.preventDefault()]
[Execute cmd.action()]
```

---

## 5. Discoverability & User Experience

1. **Inline Navigation Rail Badges**: Each primary route button in `AppShell` displays its accelerator (`Alt+1`, `Alt+2`, `Alt+3`, `Alt+4`) in high-contrast text and announces it in `aria-label`.
2. **Keyboard Shortcuts Modal**:
   - Categorized reference table grouping navigation, library, displays, settings, and general commands.
   - Semantic `<kbd class="kw-kbd">` styling matching the Neon Glass design system.
   - Triggerable via `?` (when outside inputs), `F1`, or the footer button in the navigation rail.
   - Accessible modal semantics: `role="dialog"`, `aria-modal="true"`, `trapFocus`, `Escape` to close.

---

## 6. Verification Matrix

| Verification Target | Command / Test | Outcome |
|:---|:---|:---|
| UI Build & Contract Verification | `node src-ui/scripts/verifyBuild.ts` | OK (Pass) |
| Unit Tests (Shortcuts & Commands) | `node --test src-ui/tests/shortcuts.test.ts` | 9/9 passed |
| Full UI Regression Suite | `npm test` | 83/83 passed |
| Verification Gate (Build, UI, Rust Policy) | `npm run verify:gate` | OK (Pass) |
| Workspace Rust Integration Tests | `cargo test --workspace` | 341/341 passed |

