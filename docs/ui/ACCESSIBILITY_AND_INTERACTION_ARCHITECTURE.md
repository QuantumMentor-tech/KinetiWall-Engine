# KinetiWall Accessibility Preferences, Reduced Motion & Inclusive Interaction Architecture (Step 63)

## 1. Executive Summary & Design Mandate

Step 63 establishes an authoritative, privacy-preserving accessibility and inclusive interaction system for KinetiWall. It enables users to configure motion intensity and interact predictably with the application through keyboards, screen readers, and assistive technologies without sacrificing the sleek Neon Glass design language or disrupting active wallpaper playback sessions.

### Core Architectural Principles
1. **Three-Tier Motion Control**: Provides `'system' | 'always_reduce' | 'no_preference'` options persistent within `KinetiWallConfig` Schema V1 (KW-DAT-004).
2. **Deterministic Motion Resolution**:
   - `'always_reduce'`: Forces collapsed animations, transitions to 0ms, and pauses decorative loops across all application UI surfaces.
   - `'no_preference'`: Enables full expressive Neon Glass motion transitions even if the OS requests reduced motion.
   - `'system'`: Synchronizes dynamically with Windows / OS `prefers-reduced-motion: reduce` media queries with zero memory leaks.
3. **Scoped CSS Motion Architecture**: Replaces indiscriminate global CSS overrides with scoped design tokens (`--kw-transition-fast`, `--kw-transition-normal`) and explicit `[data-reduced-motion="true|false"]` selectors. Functional status badges and indicators remain fully legible and statically visible.
4. **Assistive Technology Live Announcements**: A lightweight, polite live region (`aria-live="polite"`, `role="status"`, `.kw-sr-only`) announces view transitions, preference saves, and error states without noisy duplicate stutter.
5. **Robust Keyboard Navigation & Focus Management**: Implements W3C WAI-ARIA Radio Group pattern with roving `tabindex`, keyboard-activatable `switch` toggles, focus restoration stack for transient dialogs, and modal focus trapping.
6. **Strict Playback Isolation Guarantee**: Accessibility preferences, DOM modifications, and keyboard listeners are strictly confined to the application shell and settings UI. No styling, scripts, or hooks are injected into player windows or wallpaper processes.

---

## 2. Authoritative Data Model & Persistence

### 2.1 Rust Desktop Configuration (`crates/kinetiwall-desktop/src/settings.rs`)

```rust
/// Accessible reduced motion preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReducedMotionPreference {
    #[default]
    System,
    AlwaysReduce,
    NoPreference,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneralConfig {
    pub autostart: bool,
    pub start_minimized: bool,
    pub minimize_to_tray: bool,
    pub theme: ThemePreference,
    #[serde(default)]
    pub reduced_motion: ReducedMotionPreference,
    pub language: String,
}
```

- Tagged with `#[serde(default)]` to guarantee backward compatibility with existing configuration files lacking the field.
- Serialized to snake_case (`system`, `always_reduce`, `no_preference`).

### 2.2 TypeScript Frontend DTO (`src-ui/types/settings.ts`)

```typescript
export type ReducedMotionPreferenceDto = 'system' | 'always_reduce' | 'no_preference';

export interface GeneralConfigDto {
  autostart: boolean;
  start_minimized: boolean;
  minimize_to_tray: boolean;
  theme: ThemePreferenceDto;
  reduced_motion: ReducedMotionPreferenceDto;
  language: string;
}
```

---

## 3. Motion Resolution & Reactive State Management

### 3.1 Decision Logic (`src-ui/types/accessibility.ts`)

```typescript
export function resolveEffectiveReducedMotion(
  preference: ReducedMotionPreferenceDto,
  isSystemReducedMotion: boolean,
): boolean {
  switch (preference) {
    case 'always_reduce':
      return true;
    case 'no_preference':
      return false;
    case 'system':
    default:
      return isSystemReducedMotion;
  }
}
```

| User Preference | OS `prefers-reduced-motion` | Effective Reduced Motion | Application UI Motion Behavior |
|:---|:---|:---|:---|
| `system` | `false` | `false` | Full animations (120ms/180ms transitions, subtle pulses) |
| `system` | `true` | `true` | 0ms transitions, decorative animations paused |
| `always_reduce` | `false` | `true` | 0ms transitions, decorative animations paused |
| `always_reduce` | `true` | `true` | 0ms transitions, decorative animations paused |
| `no_preference` | `false` | `false` | Full animations |
| `no_preference` | `true` | `false` | Full animations (explicit user override of OS) |

### 3.2 Operating System Motion Detection (`src-ui/services/systemMotionDetector.ts`)

- `BrowserSystemMotionDetector` binds to `window.matchMedia('(prefers-reduced-motion: reduce)')`.
- Supports modern `addEventListener('change', ...)` as well as legacy `addListener` for backward compatibility in older WebView2 runtimes.
- Returns clean disposal closures to prevent memory leaks.
- Includes `MockSystemMotionDetector` for headless unit testing.

### 3.3 AccessibilityStore (`src-ui/stores/accessibilityStore.ts`)

Maintains the authoritative state:
```typescript
export interface AccessibilityState {
  reducedMotionPreference: ReducedMotionPreferenceDto;
  isSystemReducedMotion: boolean;
  effectiveReducedMotion: boolean;
  isSystemSupported: boolean;
}
```

Synchronizes bidirectionally with `SettingsStore` and drives `data-reduced-motion` and `data-motion-preference` DOM attributes on `document.documentElement` and `AppShell`.

---

## 4. Scoped CSS Motion Strategy (`src-ui/styles/theme.css`)

Rather than destroying layout rendering with indiscriminate `*` rules, the scoped CSS strategy adjusts design tokens and targets decorative loops:

```css
:root[data-reduced-motion="true"],
[data-reduced-motion="true"] {
  --kw-transition-fast: 0ms !important;
  --kw-transition-normal: 0ms !important;
}

[data-reduced-motion="true"] *,
[data-reduced-motion="true"] *::before,
[data-reduced-motion="true"] *::after {
  animation-duration: 0.001ms !important;
  animation-iteration-count: 1 !important;
  transition-duration: 0.001ms !important;
  scroll-behavior: auto !important;
}

[data-reduced-motion="true"] .kw-dirty-indicator {
  animation: none !important;
  opacity: 1 !important;
}
```

When users specify `no_preference` (`[data-reduced-motion="false"]`), the CSS rules ensure `@media (prefers-reduced-motion: reduce)` does not override user intent:

```css
@media (prefers-reduced-motion: reduce) {
  :root:not([data-reduced-motion="false"]) {
    --kw-transition-fast: 0ms !important;
    --kw-transition-normal: 0ms !important;
  }
}
```

---

## 5. Assistive Technology Live Announcements (`src-ui/utils/announcer.ts`)

The `ScreenReaderAnnouncer` manages polite and assertive live regions:
- Renders visually hidden regions (`.kw-sr-only`) with `aria-live="polite"` or `aria-live="assertive"` and `aria-atomic="true"`.
- Features built-in 400ms deduplication debouncing to prevent repetitive audio jitter when buttons are rapidly clicked.
- Integrated into `SettingsView` for:
  - Tab navigation changes ("Playback & Policy settings tab selected.")
  - Successful settings persistence ("Settings saved successfully.")
  - Save failure alerts ("Failed to save settings. Please inspect validation errors.")
  - Discarding draft changes ("Unsaved changes discarded.")

---

## 6. Keyboard Navigation & Focus Trapping (`src-ui/utils/focusManager.ts`)

- **Roving Tabindex in Radio Groups**: `PreferenceSegmentedGroup` assigns `tabindex="0"` to the selected option and `tabindex="-1"` to unselected options. Supports `ArrowRight` / `ArrowDown` (next option) and `ArrowLeft` / `ArrowUp` (previous option) cycling.
- **Switch Toggles**: `PreferenceToggle` adheres to W3C Switch design pattern, toggling on `Space` or `Enter` keypresses with `role="switch"` and `aria-checked`.
- **High-Contrast Focus Rings**: All interactive controls (`button`, `input`, `select`, `[role="switch"]`, `[role="tab"]`, `[role="radio"]`) receive a high-contrast focus ring (`outline: 2px solid var(--kw-border-focus); outline-offset: 2px;`).
- **Modal Focus Trapping & Restoration**: `trapFocus` intercepts `Tab` and `Shift+Tab` within modal dialogs (`ConfirmationDialog`), preventing keyboard focus from escaping into background content. `FocusRestorationStack` restores focus to triggering elements when dialogs close.

---

## 7. Verification & Compliance Matrix

| Verification Target | Gate / Command | Outcome |
|:---|:---|:---|
| Rust Backend Settings & Migration | `cargo test -p kinetiwall-desktop --test settings_tests` | 13/13 passing |
| UI Build & Contract Verification | `node src-ui/scripts/verifyBuild.ts` | Passing |
| Full UI Accessibility & Interaction Suite | `npm test` (`src-ui/tests/accessibility.test.ts`) | 74/74 passing |
| Full Policy & UI Verification Gate | `npm run verify:gate` | Passing |
| Full Rust Workspace Unit & Integration Tests | `cargo test --workspace` | 341/341 passing |

