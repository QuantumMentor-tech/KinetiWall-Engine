# In-App Help, Contextual Guidance & Troubleshooting Architecture

## Specification & Contract Reference (Step 67)

This document specifies the in-app help system, contextual assistance triggers, diagnostic reason code mapping, safe action dispatch, and evidence-based troubleshooting architecture for KinetiWall Engine.

---

## 1. Architectural Foundations

1. **Local-First & Offline Resilience**: All help content, troubleshooting guides, and keyword indexes are bundled locally within the application. The system operates 100% offline with zero external network dependencies, telemetry, or remote knowledge base lookups.
2. **Evidence-Based Troubleshooting**: Diagnostic guidance is directly derived from observable system states (policy arbitration decisions, desktop topology classifications, catalog integrity reports, and capability readiness checks). When the root cause cannot be conclusively proven, KinetiWall communicates this uncertainty honestly rather than speculating.
3. **Safe Whitelisted Actions**: Topics may offer recommended next actions (e.g., navigating to Settings, refreshing display topology, or reopening onboarding). These actions dispatch exclusively through a strictly typed, whitelisted action registry (`SafeActionId`). Arbitrary shell execution or privileged system modifications are impossible.
4. **Context Preservation**: Opening help or troubleshooting dialogs never resets user state, pending settings drafts, active search filters, or live wallpaper playback.
5. **Universal Accessibility**: Conforms to WCAG 2.1 AA standards, supporting full keyboard navigation (`F1` for Help, `?` for Shortcuts, `Escape` for dismissal), screen reader live announcements, visible focus rings, and high-contrast themes.

---

## 2. Information Architecture & Category Taxonomy

Help content is organized into 6 user-oriented categories defined in `src-ui/types/help.ts`:

| Category Identifier | Display Label | Scope & User Goal |
| :--- | :--- | :--- |
| `getting_started` | Getting Started | Core mission, local-first guarantees, WorkerW desktop embedding, first-run orientation. |
| `library_and_import` | Library & Import | Managing local catalog records, video/web package imports, metadata inspection, and asset relinking. |
| `playback_and_policies` | Playback & Policies | Battery optimization, fullscreen game pause, inactivity sleep, workstation lock, and audio reactivity. |
| `displays_and_layout` | Displays & Layout | Multi-monitor presentation modes (Independent, Span, Clone) and per-display assignments. |
| `settings_and_preferences` | Settings & Preferences | Startup options, tray minimization, themes (Dark, Light, System Sync), and keyboard shortcuts. |
| `troubleshooting` | Troubleshooting & Diagnostics | 5-part evidence-based breakdowns for real, reproducible operational conditions. |

---

## 3. Help Topic Data Model

Each help topic conforms to the `HelpTopic` interface (`src-ui/types/help.ts`):

```typescript
export interface HelpTopic {
  id: string;                               // Stable identifier (e.g. 'ts-playback-paused')
  title: string;                            // User-facing title
  category: HelpCategory;                   // One of the 6 core categories
  summary: string;                          // Concise 1-2 sentence description
  content: string[];                        // Paragraph array
  keywords: string[];                       // Local search tokens
  relatedDiagnosticCodes?: string[];        // Diagnostic reason codes mapped to this topic
  recommendedAction?: {
    actionId: SafeActionId;                 // Whitelisted application action
    label: string;                          // Button text
  };
  troubleshootingDetail?: {
    whatHappened: string;                   // Observed condition
    whatIsKnown: string;                      // Verified technical status
    likelyInterpretation: string;           // Plausible interpretation consistent with evidence
    recommendedNextStep: string;            // Safe, reversible user action
    limitations: string;                    // Honest boundaries & edge cases
  };
}
```

---

## 4. Diagnostic-to-Guidance Mapping

When the application encounters an error, policy pause, or capability warning, the reason code is mapped to the most relevant troubleshooting topic via `resolveHelpTopicForDiagnostic()` (`src-ui/services/helpContent.ts`):

| Diagnostic / Reason Code | Mapped Topic ID | User-Facing Topic Title |
| :--- | :--- | :--- |
| `policy.fullscreen_pause` | `ts-playback-paused` | Wallpaper Paused or Not Animating |
| `policy.battery_throttle` | `ts-playback-paused` | Wallpaper Paused or Not Animating |
| `policy.battery_pause` | `ts-playback-paused` | Wallpaper Paused or Not Animating |
| `policy.inactivity_pause` | `ts-playback-paused` | Wallpaper Paused or Not Animating |
| `policy.workstation_lock` | `ts-playback-paused` | Wallpaper Paused or Not Animating |
| `desktop_integration.limited` | `ts-workerw-unsplit` | Wallpaper Not Embedding or Black Desktop |
| `desktop.unsplit_topology` | `ts-workerw-unsplit` | Wallpaper Not Embedding or Black Desktop |
| `video_player.limited` | `ts-video-codec` | Video Wallpaper Fails to Play |
| `playback.video_backend_error`| `ts-video-codec` | Video Wallpaper Fails to Play |
| `web_engine.limited` | `ts-webview2-missing` | Interactive Web Wallpaper Blank or Disabled |
| `playback.webview2_missing` | `ts-webview2-missing` | Interactive Web Wallpaper Blank or Disabled |
| `audio_reactivity.limited` | `ts-audio-wasapi` | Audio Reactivity Visualizer Idle |
| `catalog.asset_missing` | `ts-asset-missing` | Wallpaper Asset Reported Missing on Disk |
| `settings.save_failed` | `ts-settings-save-failed` | Settings Changes Could Not Be Saved |
| *Unrecognized / Null* | `ts-unknown-issue` | Unrecognized Condition or Diagnostic Notice |

---

## 5. Safe Action Execution Dispatch

Topics can guide users to the exact application control required to remediate a situation. Permitted actions are restricted to the `SafeActionId` union:

- `nav.dashboard`: Navigate to Engine Dashboard.
- `nav.library`: Navigate to Wallpaper Library.
- `nav.displays`: Navigate to Displays & Layout.
- `nav.settings`: Navigate to Application Settings.
- `library.refresh`: Re-scan local catalog records.
- `displays.refresh`: Re-discover connected monitors and WorkerW desktop topology.
- `onboarding.reopen`: Launch the first-run guided onboarding tour.
- `readiness.recheck`: Re-run non-invasive system capability readiness checks.
- `help.shortcuts`: Open the keyboard shortcuts reference cheat sheet.

---

## 6. Authoring & Maintenance Guidelines

1. **Assigning Topic Identifiers**:
   - Informational topics use `help-<slug>` (e.g., `help-import-content`).
   - Troubleshooting topics use `ts-<slug>` (e.g., `ts-video-codec`).
   - Identifiers must never be changed after release; display titles may be updated freely.
2. **Writing Troubleshooting Topics**:
   - All 5 sections of `troubleshootingDetail` are mandatory for troubleshooting topics.
   - Never assert that an unknown failure is caused by a specific missing driver or corrupted system file without evidence.
   - Never advise users to terminate unrelated processes, edit Windows registry keys, disable security features, or run arbitrary administrator commands.

---

## 7. Automated Test Suite & Verification Commands

### Running UI & Help Tests
```powershell
npm test
node src-ui/scripts/verifyBuild.ts
```

### Running Native Settings Verification
```powershell
cargo test -p kinetiwall-desktop --test settings_tests
```

### Running Full Verification Gate
```powershell
npm run verify:gate
cargo test --workspace
```

