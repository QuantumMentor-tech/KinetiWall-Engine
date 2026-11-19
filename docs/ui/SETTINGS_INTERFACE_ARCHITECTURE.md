# Settings Interface Integration, Preference Controls & Save-State UX

## Document ID: KW-UI-061 | Step 61 of 90
**Subsystem**: Frontend User Interface & Shell Integration (`src-ui`)  
**Backend Spec Ref**: KW-DAT-004 (`config.json` Schema V1) & KW-DAT-007 (Atomic Storage & Backup)  
**Status**: Implemented & Verified  

---

### 1. Architectural Scope & Mandate

This subsystem establishes the primary user settings interface for the KinetiWall desktop application. It connects genuine, supported user preferences directly to the centralized configuration repository (`KinetiWallConfig` Schema V1) implemented in Step 60, without duplicate state ownership, misleading save indicators, unsupported options, or broken navigation.

```
+-----------------------------------------------------------------------------------------+
|                                      AppShell                                           |
|                           [Dashboard | Library | Displays | Settings]                   |
+--------------------------------------------+--------------------------------------------+
                                             | (Route: 'settings')
                                             v
+-----------------------------------------------------------------------------------------+
|                                     SettingsView                                        |
|  [Tabs: General | Playback & Policy | Rendering | Audio | Diagnostics & About]          |
|                                                                                         |
|  +--------------------+  +--------------------+  +--------------------+  +-----------+  |
|  |  PreferenceToggle  |  |  PreferenceSelect  |  |PreferenceSegmented |  | Slider/Num|  |
|  +--------------------+  +--------------------+  +--------------------+  +-----------+  |
|                                                                                         |
|  +-----------------------------------------------------------------------------------+  |
|  | SaveStatusBar: [Unsaved Changes (X) / Saved] [Discard Changes] [Save Changes]     |  |
|  +-----------------------------------------------------------------------------------+  |
+--------------------------------------------+--------------------------------------------+
                                             | Reactive Draft / Persist
                                             v
+-----------------------------------------------------------------------------------------+
|                                     SettingsStore                                       |
|  - persistedConfig: KinetiWallConfigDto   |  - isDirty / dirtyFields: string[]          |
|  - draftConfig: KinetiWallConfigDto       |  - validationErrors: Record<string, string>  |
|  - saveStatus: idle|saving|saved|error    |  - saveChanges() / discardChanges()         |
+--------------------------------------------+--------------------------------------------+
                                             | Typed IPC Bridge
                                             v
+-----------------------------------------------------------------------------------------+
|                         SettingsIpcClient & FileSettingsRepository                      |
|                  (Atomic write to config.json.tmp -> rename -> config.json.bak)         |
+-----------------------------------------------------------------------------------------+
```

---

### 2. Supported Preference-Control Mapping (KW-DAT-004 Schema V1)

Every control in the settings interface maps directly to an authoritative configuration property in `KinetiWallConfig`:

| Section | Preference Field | UI Component | Allowed Options / Bounds | Default Value |
| :--- | :--- | :--- | :--- | :--- |
| **General** | `general.autostart` | `PreferenceToggle` | `true \| false` | `false` |
| | `general.start_minimized` | `PreferenceToggle` | `true \| false` | `true` |
| | `general.minimize_to_tray` | `PreferenceToggle` | `true \| false` | `true` |
| | `general.theme` | `PreferenceSegmentedGroup`| `'system' \| 'dark' \| 'light'` | `'system'` |
| | `general.language` | `PreferenceSelect` | `'en-US' \| 'de-DE' \| 'fr-FR' \| 'ja-JP' \| 'zh-CN'` | `'en-US'` |
| **Playback & Policy** | `performance.on_fullscreen_app` | `PreferenceSegmentedGroup`| `'pause' \| 'mute' \| 'keep_playing'` | `'pause'` |
| | `performance.on_maximized_window`| `PreferenceSegmentedGroup`| `'keep_playing' \| 'mute' \| 'pause'` | `'keep_playing'` |
| | `performance.on_battery_power` | `PreferenceSegmentedGroup`| `'throttle_30fps' \| 'pause' \| 'keep_playing'` | `'throttle_30fps'` |
| | `performance.on_remote_desktop` | `PreferenceSegmentedGroup`| `'pause' \| 'keep_playing'` | `'pause'` |
| | `performance.pause_on_workstation_lock` | `PreferenceToggle` | `true \| false` | `true` |
| | `performance.enable_display_power_pause` | `PreferenceToggle` | `true \| false` | `true` |
| | `performance.enable_inactivity_pause` | `PreferenceToggle` | `true \| false` | `true` |
| | `performance.inactivity_threshold_millis` | `PreferenceSlider` | `5,000` to `86,400,000` ms (1–60 min slider) | `300,000` ms (5 min) |
| | `performance.target_framerate_ac` | `PreferenceSlider` | `15` to `240` FPS (step: 5) | `60` FPS |
| | `performance.target_framerate_battery` | `PreferenceSlider` | `15` to `60` FPS (step: 5) | `30` FPS |
| | `performance.enable_adaptive_quality` | `PreferenceToggle` | `true \| false` | `true` |
| | `performance.enable_resource_budget` | `PreferenceToggle` | `true \| false` | `true` |
| **Rendering** | `rendering.hardware_acceleration` | `PreferenceToggle` | `true \| false` | `true` |
| | `rendering.vsync` | `PreferenceToggle` | `true \| false` | `true` |
| | `rendering.stretch_mode_default` | `PreferenceSegmentedGroup`| `'fill' \| 'fit' \| 'stretch' \| 'center'` | `'fill'` |
| **Audio** | `audio.master_volume` | `PreferenceSlider` | `0.0` to `1.0` (0%–100% slider, step: 1%) | `0.5` (50%) |
| | `audio.master_mute` | `PreferenceToggle` | `true \| false` | `false` |
| | `audio.mute_when_other_app_plays_audio` | `PreferenceToggle` | `true \| false` | `true` |
| **Diagnostics** | `diagnostics.log_level` | `PreferenceSelect` | `'error' \| 'warn' \| 'info' \| 'debug' \| 'trace'` | `'info'` |
| | `diagnostics.max_log_retention_days` | `PreferenceSlider` | `1` to `30` days (step: 1) | `7` days |
| | `diagnostics.enable_file_logging` | `PreferenceToggle` | `true \| false` | `true` |
| | `diagnostics.enable_diagnostics_overlay` | `PreferenceToggle` | `true \| false` | `false` |

---

### 3. Core Architectural Guarantees

1. **Separation of Draft and Persisted State**:
   - The authoritative configuration system remains the owner of committed preferences.
   - The UI never mutates `persistedConfig` directly. Edits occur in `draftConfig`.
   - Modifying a field tracks granular dirty states (`dirtyFields: string[]`). A glowing cyan dot indicator appears adjacent to modified controls.
   - When a draft value is reverted to match the persisted value, the field is automatically pruned from `dirtyFields`.

2. **Truthful Save-State UX (`SaveStatusBar`)**:
   - **Clean / Saved**: Displays "Preferences saved and up to date" with the last saved timestamp. Save and Discard buttons are disabled.
   - **Unsaved Draft**: Displays "X unsaved preference(s) in draft", activates "Discard Changes" and "Save Changes" buttons.
   - **Saving in Progress**: Displays a progress spinner and "Saving preferences to disk...". Buttons are disabled to prevent race conditions.
   - **Save Failure**: If atomic write fails, the UI surfaces an inline error banner explaining the storage error, retains the draft without data loss, and leaves the last committed state untouched.

3. **Client-Side Validation & Safeguards**:
   - `validateSettings(config)` runs before persistence requests.
   - Strict range enforcement (AC FPS 15–240, Battery FPS 15–60, Volume 0–100%, Retention 1–30 days, non-empty language).
   - If validation errors exist, the "Save Changes" button is disabled and an inline error message appears directly underneath the invalid control.

4. **Reset-to-Default Operations**:
   - **Section Reset**: Each preference card section provides a "Reset Section" button that resets only that section's draft fields to factory defaults without touching other sections.
   - **Full Factory Reset**: A top-level "Reset All to Defaults" button restores all preferences across all sections atomically.

5. **Accessibility & Interaction Quality (WCAG 2.1 AA)**:
   - `PreferenceToggle`: Exposes `role="switch"`, `aria-checked="true|false"`, and keyboard activation via Space/Enter.
   - `PreferenceSegmentedGroup`: Exposes `role="radiogroup"` with `role="radio"` and `aria-checked`.
   - `PreferenceSlider`: Exposes `aria-valuemin`, `aria-valuemax`, `aria-valuenow`, and `aria-valuetext` with live readout badges.
   - `kw-focus-ring`: Uniform high-contrast focus rings for keyboard navigation.

---

### 4. Verification & Gate Evidence

| Verification Target | Command | Result |
| :--- | :--- | :---: |
| **Settings UI Test Suite** | `node --test src-ui/tests/settingsUi.test.ts` | **14 / 14 PASS** |
| **Full UI Test Suite** | `npm test` | **61 / 61 PASS (7 suites)** |
| **SSR / Build Verification** | `node src-ui/scripts/verifyBuild.ts` | **PASS** |
| **Policy Verification Gate** | `npm run verify:gate` | **PASS** |
| **Rust Workspace Tests** | `cargo test --workspace` | **341 / 341 PASS** |

