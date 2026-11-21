# First-Run Experience, Guided Onboarding & Capability Readiness Architecture

## Specification & Contract Reference (Step 66)

This document specifies the first-run onboarding lifecycle, capability readiness modeling, safe persistence, and accessible guidance architecture for KinetiWall Engine.

---

## 1. Architectural Principles

1. **Explain Existing Capabilities Honestly**: The onboarding tour introduces existing capabilities (wallpaper library, registration/import, multi-monitor layouts, power arbitration, keyboard shortcuts) without fabricating functionality or advertising unsupported features.
2. **Never Force Setup**: Users must never be required to complete optional tutorials, import assets, or configure monitors merely to reach the primary interface. The "Skip Guide" control is prominently available on all stages.
3. **Truthful Capability Readiness**: Capability checks evaluate real subsystem status (`SettingsStore`, `CatalogStore`, `DisplayStore`, playback backends) rather than assuming readiness. Unknown or unverified capabilities are never collapsed into "Ready".
4. **Isolate Optional Dependencies**: An unavailable optional feature (e.g., missing Edge WebView2 for web wallpapers or missing video codecs) must never mark the core application unusable. Essential features (configuration, catalog, desktop integration) are decoupled from optional enhancements.
5. **Non-Invasive, Read-Only Evaluation**: Readiness checks never mutate Windows desktop-host structures, terminate processes, download unverified dependencies, or modify registry keys.
6. **Authoritative Persistence**: Onboarding completion and dismissal are tracked strictly via the centralized `GeneralConfig` schema (`crates/kinetiwall-desktop/src/settings.rs` and `src-ui/types/settings.ts`).

---

## 2. Onboarding Lifecycle & State Model

The onboarding state is stored in `GeneralConfig.onboarding` (`config.json` Schema V1):

```typescript
export type OnboardingStatusDto = 'not_started' | 'in_progress' | 'completed' | 'dismissed';

export interface OnboardingRecordDto {
  version: number;
  status: OnboardingStatusDto;
  last_completed_step: string | null;
  completed_at_millis: number | null;
}
```

### Lifecycle Semantics

| Status | Definition | Initial Launch Behavior |
| :--- | :--- | :--- |
| `not_started` | Fresh installation or uninitialized profile without prior record. | Automatically opens the `OnboardingModal` on initial launch. |
| `in_progress` | User advanced past stage 0 but has not yet finished or dismissed. | Opens modal restoring the last recorded stage index. |
| `completed` | User explicitly finished setup or clicked "Open Wallpaper Library". | Modal does not open on startup; access remains available in Settings. |
| `dismissed` | User closed or clicked "Skip Guide". | Modal does not open on startup; access remains available in Settings. |

### Reopening Guidance

Users can revisit the guided tour and system readiness inspector at any time:
- **Application Settings**: In General Preferences, clicking the **"Launch Guided Tour"** button under *Guided Onboarding & System Readiness*.
- **Global Command / Shortcut**: Pressing `Ctrl+Shift+H` executes the registered `help.onboarding` command.

---

## 3. Truthful Capability Readiness Model

Readiness is evaluated across 6 key subsystems, distinguishing between **Essential** (blockers) and **Optional** (warnings):

```
┌────────────────────────────────────────────────────────┐
│              ReadinessEvaluator Service                │
└──────────────────────────┬─────────────────────────────┘
                           │
    ┌──────────────────────┼──────────────────────┐
    ▼                      ▼                      ▼
┌──────────────────┐ ┌──────────────────┐ ┌──────────────────┐
│  Configuration   │ │ Wallpaper Catalog│ │Desktop Integration│
│  (Essential)     │ │  (Essential)     │ │  (Essential)     │
└──────────────────┘ └──────────────────┘ └──────────────────┘
    ┌──────────────────────┼──────────────────────┐
    ▼                      ▼                      ▼
┌──────────────────┐ ┌──────────────────┐ ┌──────────────────┐
│   Video Player   │ │  Web Engine      │ │ Audio Reactivity │
│   (Optional)     │ │  (Optional)      │ │   (Optional)     │
└──────────────────┘ └──────────────────┘ └──────────────────┘
```

### Capability Details

| Subsystem | Requirement | Source of Truth | Readiness State Criteria |
| :--- | :--- | :--- | :--- |
| **`configuration`** | Essential | `SettingsStore` | `ready`: Config loaded & validated against Schema V1.<br>`initializing`: Load in progress.<br>`action_required`: Parse or validation failure. |
| **`wallpaper_catalog`** | Essential | `CatalogStore` | `ready`: Catalog database active and integrity verified.<br>`initializing`: Initial catalog read.<br>`limited`: Read error; safe fallback active. |
| **`desktop_integration`** | Essential | `DisplayStore` | `ready`: Monitors enumerated; WorkerW desktop topology active.<br>`initializing`: Topology discovery in-flight.<br>`limited`: No active monitors detected. |
| **`video_player`** | Optional | Playback Engine | `ready`: DirectShow & Media Foundation codecs available.<br>`limited`: Restricted codecs; static/web wallpapers supported. |
| **`web_engine`** | Optional | WebView2 Runtime | `ready`: Microsoft Edge WebView2 runtime confirmed.<br>`limited`: WebView2 absent; web wallpapers disabled. |
| **`audio_reactivity`** | Optional | WASAPI Capture | `ready`: Active default loopback audio device detected.<br>`limited`: Audio capture disabled or muted. |

### Anti-Stampede & Safe Retry

Rapidly clicking **"Re-check Readiness"** does not launch redundant concurrent evaluation threads. An in-flight check promise is shared across concurrent callers, preventing performance degradation and event loops.

---

## 4. Guided Tour Stages

The `OnboardingModal` presents a 4-step sequential flow:

1. **Stage 1 (`welcome`)**:
   - Welcome banner detailing local-first architecture (zero telemetry, zero cloud tracking).
   - Core value cards: Resource-aware policies, multi-monitor presentation, video & web support.
2. **Stage 2 (`readiness`)**:
   - Live capability cards showing status badges (✓ Ready, ⚠ Limited, ⏳ Initializing).
   - Detailed status messages with actionable tips for remediation where applicable.
   - Dedicated "Re-check Readiness" action with loading spinner.
3. **Stage 3 (`workflows`)**:
   - Step 1: Browse Wallpaper Library (`Alt+2`) and inspect metadata.
   - Step 2: Import content safely via "Add Wallpaper" without unsolicited disk scanning.
   - Step 3: Assign per-monitor layouts (Independent, Span, Clone) via Displays (`Alt+3`).
4. **Stage 4 (`completion`)**:
   - Confirmation hero badge ("You're Ready to Explore KinetiWall").
   - Discovery tips: Keyboard shortcuts reference (`?` / `F1`) and Settings (`Alt+4`).
   - Primary action: "Open Wallpaper Library" (completes onboarding and navigates to library).
   - Secondary action: "Finish Setup" (completes onboarding and returns to current view).

---

## 5. Accessibility & UX Guarantees

- **Modal Semantics**: `role="dialog"`, `aria-modal="true"`, `aria-labelledby="onboarding-modal-title"`.
- **Focus Management**: Focus is strictly trapped within the dialog using `trapFocus()`. Pressing `Escape` invokes safe dismissal.
- **Progress Tracking**: Accessible step indicator bar (`role="progressbar"`, `aria-valuenow`, `aria-valuemin="1"`, `aria-valuemax="4"`).
- **Reduced Motion**: All modal transitions respect `--kw-transition-fast` and the `data-reduced-motion` attribute.
- **Save Failures**: In the rare event that settings persistence fails, an inline warning banner is displayed and the draft is preserved without misleading the user.

---

## 6. Verification & Automated Test Commands

### Running UI & Contract Tests
```powershell
npm test
node src-ui/scripts/verifyBuild.ts
```

### Running Native Settings & Migration Tests
```powershell
cargo test -p kinetiwall-desktop --test settings_tests
```

### Running the Full Regression Gate
```powershell
npm run verify:gate
cargo test --workspace
```

