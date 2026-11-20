# KinetiWall In-App Notification Architecture & Toast Lifecycle (Step 65)

## 1. Executive Summary & Design Mandate

Step 65 establishes a centralized, typed, accessible in-app notification and toast feedback system for KinetiWall. It communicates operational updates, successful actions, validation problems, and recoverable system conditions without overwhelming the user, cluttering the interface, or leaking raw debug traces into user-facing surfaces.

### Core Architectural Principles
1. **Typed Notification Model (`NotificationItem`)**:
   - Explicit severity levels (`info`, `success`, `warning`, `error`) with truthful status reporting.
   - Stable reason codes (`settings.saved`, `settings.save_failed`, `displays.applied`, etc.).
   - Explicit lifecycle durations: transient by default for success/info; persistent for errors to guarantee accountability.
2. **Authoritative State Management (`NotificationStore`)**:
   - Single point of truth for active notifications, auto-dismiss timers, queue bounds, and user preferences.
   - Bounded presentation: strictly limits concurrent visible toasts (`MAX_VISIBLE_NOTIFICATIONS = 4`) and purges oldest transient toasts during high volume.
3. **Deduplication & Storm Prevention**:
   - Prevents identical repeated events (e.g. repeated battery alerts or network errors within a 1000ms window) from flooding the viewport.
4. **Message Sanitization & Privacy**:
   - Automatically redacts absolute Windows / UNC filesystem paths (`C:\Users\...` -> `[path]/filename.ext`).
   - Strips raw Rust exception wrappers (`Custom { kind: ..., error: ... }`) before presentation.
5. **Actionable Feedback**:
   - Supports safe, validated action callbacks (e.g., "Reconfigure", "Retry", "View Settings") without bypassing feature boundaries or policy arbitration.
6. **User Preferences Control**:
   - Integrated with `KinetiWallConfig` Schema V1 (`GeneralConfig.notifications` with `'all' | 'errors_only' | 'disabled'`).
   - Critical errors are never suppressed even when transient notifications are disabled.
7. **Accessibility & Inclusive Design**:
   - Accessible container (`role="region"`, `aria-label="Notifications"`).
   - High-contrast visual icons paired with text (`✓`, `ℹ`, `⚠`, `✕`).
   - Dynamic politeness mapping via `ScreenReaderAnnouncer`: polite for informational/success; assertive for warnings/errors.
   - Non-focus-stealing presentation; respects reduced motion (`0ms` transitions).

---

## 2. Notification Semantics & Default Durations

| Severity | Purpose | Default Duration | Role | Screen Reader Politeness |
|:---|:---|:---:|:---:|:---:|
| `success` | Confirmation of completed operation (e.g., settings saved, display assigned) | 3,500 ms | `status` | `polite` |
| `info` | Useful update about an application event or state transition | 4,000 ms | `status` | `polite` |
| `warning` | Condition requiring user awareness without critical operation failure | 5,000 ms | `alert` | `assertive` |
| `error` | Operation failure or significant problem requiring attention | Persistent (`null`) | `alert` | `assertive` |

---

## 3. Data Model & Architecture (`src-ui/types/notification.ts`)

```typescript
export interface NotificationRequest {
  id?: string;
  severity: NotificationSeverity;
  title?: string;
  message: string;
  reasonCode?: string;
  durationMs?: number; // null/undefined uses default; 0 means persistent
  dedupKey?: string;
  action?: NotificationAction;
  dismissible?: boolean;
}

export interface NotificationItem {
  id: string;
  severity: NotificationSeverity;
  title?: string;
  message: string;
  reasonCode?: string;
  timestamp: number;
  expiresAt: number | null;
  durationMs: number;
  dedupKey?: string;
  action?: NotificationAction;
  dismissible: boolean;
}
```

---

## 4. Lifecycle Flow & Queue Management

```
[Feature Event Confirmed]
          │
          ▼
[shouldPresentNotification(severity, preference)?] ──No──► [Filtered out (Silent)]
          │ Yes
          ▼
[sanitizeNotificationMessage(msg)]
          │
          ▼
[Check Dedup Window (1000ms)] ──Duplicate──► [Update existing timestamp, return ID]
          │ Distinct
          ▼
[Queue Full (>= 4)?] ──Yes──► [Evict oldest transient toast, cancel its timer]
          │ No
          ▼
[Push to NotificationStore]
[Schedule Auto-Dismiss Timer (if transient)]
[ScreenReaderAnnouncer.announce(msg, politeness)]
          │
          ▼
[Render in ToastContainer Viewport]
          │
          ├────────────────────────┬────────────────────────┐
          ▼                        ▼                        ▼
  [Timer Expires]           [User Clicks ✕]          [User Clicks Action]
          │                        │                        │
          └────────────────────────┴────────────────────────┘
                                   │
                                   ▼
                       [Cancel Expiry Timer]
                   [Remove from NotificationStore]
```

---

## 5. User Notification Preferences

Stored locally in `config.json` under `GeneralConfig.notifications`:

```json
{
  "general": {
    "notifications": "all"
  }
}
```

- **`all` (Default)**: All informational, success, warning, and error toasts are shown.
- **`errors_only`**: Transient success and informational toasts are silenced; warnings and errors remain active.
- **`disabled`**: Only critical error alerts are presented; all routine transient toasts are suppressed.

---

## 6. Verification Matrix

| Verification Target | Command / Test | Outcome |
|:---|:---|:---|
| Rust Backend Settings & Migration | `cargo test -p kinetiwall-desktop --test settings_tests` | 14/14 passed |
| UI Build & Contract Verification | `node src-ui/scripts/verifyBuild.ts` | OK (Pass) |
| Unit Tests (Notifications & Lifecycle) | `node --test src-ui/tests/notifications.test.ts` | 10/10 passed |
| Full UI Regression Suite | `npm test` | 93/93 passed |
| Verification Gate (Build, UI, Rust Policy) | `npm run verify:gate` | OK (Pass) |
| Workspace Rust Integration Tests | `cargo test --workspace` | 341/341 passed |

