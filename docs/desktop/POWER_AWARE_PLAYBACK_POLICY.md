# KinetiWall Power-State Awareness, Battery-Sensitive Playback & Safe Suspension Policies

**Phase:** Phase 8, Intelligent Playback Policies & System-Aware Optimization  
**Step:** 49 of 90  
**Target Platform:** Windows 10 & Windows 11  
**Category:** Native Power Awareness, Battery-Sensitive Playback, Safe System Suspension, and Resource Optimization  

---

## 1. Architectural Overview

Step 49 integrates native Windows power-state awareness into KinetiWall's intelligent playback policy engine. The subsystem enables KinetiWall to detect system power sources (AC utility vs DC internal battery), query charge percentages, observe Windows Battery Saver status, register for event-driven power notifications, and apply configurable, reversible playback policies across multi-monitor sessions.

```
┌─────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                       Native Desktop Power Subsystem                                    │
│  - NativeDesktopPowerProvider: queries kernel32!GetSystemPowerStatus                                    │
│  - RegisterPowerSettingNotification: GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE_REMAINING,         │
│    GUID_POWER_SAVING_STATUS (Battery Saver)                                                             │
│  - Window Message Handling: WM_POWERBROADCAST (PBT_POWERSETTINGCHANGE, PBT_APMSUSPEND, PBT_APMRESUME)   │
│  - Desktop Safety: identifies "no system battery" (flag 128) on desktops to prevent false triggers      │
└────────────────────────────────────────────────────┬────────────────────────────────────────────────────┘
                                                     │ SystemPowerSnapshot
                                                     ▼
┌─────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                         PowerPolicyEvaluation                                           │
│  - Modes: Off (default), Balanced, SaveBattery                                                          │
│  - Triggers: on_battery, on_battery_saver, low_battery_threshold_percent (e.g. 20%), on_system_suspend   │
│  - Deterministic evaluation returning (should_pause: bool, trigger_reason: Option<String>)              │
└────────────────────────────────────────────────────┬────────────────────────────────────────────────────┘
                                                     │ Evaluation Decision
                                                     ▼
┌─────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                     SmartPlaybackPolicyEvaluator                                        │
│  - Coordinated Reason Tracking: maintains HashSet<PauseReason> per display                              │
│  - Reasons: PauseReason::PowerSaving, PauseReason::Fullscreen, PauseReason::UserManual,                 │
│    PauseReason::SystemSuspended                                                                         │
│  - Reversibility Guarantee: AC reconnection clears PowerSaving, but leaves UserManual and               │
│    Fullscreen pauses intact                                                                             │
│  - Bounded Diagnostics: ring-buffered diagnostic events (MAX_POLICY_DIAGNOSTIC_EVENTS = 256)            │
└────────────────────────────────────────────────────┬────────────────────────────────────────────────────┘
                                                     │ Suspend / Resume Commands
                                                     ▼
┌─────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                       DisplaySessionCoordinator                                         │
│  - Coordinates independent and grouped display playback sessions                                       │
│  - Integrates seamlessly with multi-monitor layout modes (Independent, Span, Clone)                     │
└─────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

---

## 2. Windows Native Power Integration

### 2.1 Authoritative APIs
- `kernel32!GetSystemPowerStatus`: Retrieves the `SYSTEM_POWER_STATUS` structure containing AC line status, battery flags, battery charge percentage, and Windows Battery Saver status (`system_status_flag`).
- `user32!RegisterPowerSettingNotification`: Subscribes a window handle to targeted power setting changes:
  - `GUID_ACDC_POWER_SOURCE` (`5d3e9a59-e9d5-4b00-a6bd-ff34ff516548`)
  - `GUID_BATTERY_PERCENTAGE_REMAINING` (`a7ad8041-b45a-4cae-87a3-eecbb468a9e1`)
  - `GUID_POWER_SAVING_STATUS` (`e0094548-9a10-44ee-be81-4ddacd272895`)
- `user32!UnregisterPowerSettingNotification`: Reliably releases acquired notification handles during graceful shutdown.

### 2.2 Desktop vs. Laptop Discrimination
On desktop computers without internal batteries, Windows sets `SYSTEM_POWER_STATUS.BatteryFlag = 128` ("No system battery") and `BatteryLifePercent = 255` ("Unknown"). KinetiWall classifies these machines as `BatteryAvailability::NoBattery`. Battery-specific triggers (`pause_on_battery`, `low_battery_threshold_percent`) are skipped entirely on desktops to avoid spurious warnings or unwanted pauses.

### 2.3 Unknown and Out-of-Bounds Value Handling
- `BatteryLifePercent = 255`: Interpreted strictly as `None` (unknown), never as 255% or 0%.
- `ACLineStatus = 255`: Interpreted as `PowerSource::Unknown`.
- `SystemStatusFlag`: 0 is `BatterySaverState::Disabled`, 1 is `BatterySaverState::Enabled`, and others are `BatterySaverState::Unknown`.

---

## 3. Power Playback Policy Modes & Triggers

### 3.1 Policy Modes
1. **Off (Default)**: Power status is monitored for UI display and diagnostics, but wallpaper playback is never modified by power conditions. Existing user behavior is preserved by default.
2. **Balanced**: Reversible, low-disruption optimization. Wallpaper rendering is suspended only when battery charge falls at or below the configured threshold (default: 20%) or when Windows Battery Saver is engaged.
3. **Save Battery**: Proactively suspends live wallpaper rendering whenever the device operates on DC battery power, extending battery runtime on laptops and mobile workstations.

### 3.2 Configurable Triggers
- `pause_on_battery` (`bool`): Triggers in `SaveBattery` mode when running on DC battery power.
- `pause_on_battery_saver` (`bool`): Triggers when Windows Battery Saver mode is engaged.
- `low_battery_threshold_percent` (`Option<u8>`): Triggers when charge drops at or below the threshold.
- `pause_on_system_suspend` (`bool`): Automatically suspends playback sessions during OS sleep/hibernate.

---

## 4. Multi-Cause Pause Reason Tracking & Reversibility

KinetiWall guarantees that automatic power-policy clearances never override manual or competing policy pauses:

- Each logical display maintains an independent `HashSet<PauseReason>`:
  - `PauseReason::PowerSaving`: Added when power constraints are active.
  - `PauseReason::Fullscreen { display_id }`: Added when a fullscreen app is active (Step 48).
  - `PauseReason::UserManual`: Added when the user explicitly pauses playback.
  - `PauseReason::SystemSuspended`: Added when the operating system enters sleep/hibernate.
- **Clearance Rule**: When AC power is reconnected or Battery Saver disengages, `PauseReason::PowerSaving` is removed. The wallpaper session is resumed **only if no other pause reasons remain active**.
- If a user manually pauses playback before unplugging their laptop, plugging AC back in leaves the wallpaper suspended as intended.

---

## 5. System Suspend & Resume Lifecycle

1. **Sleep / Hibernate**:
   - `handle_system_suspend`: Adds `PauseReason::SystemSuspended` to all displays and calls `suspend_session`.
   - Prevents asynchronous background rendering work while the operating system transitions to low-power sleep states.
2. **Wake / Resume**:
   - `handle_system_resume`: Removes `PauseReason::SystemSuspended` from all displays.
   - Queries a fresh `SystemPowerSnapshot` to reconcile power source and battery charge.
   - Evaluates the power policy against the fresh snapshot.
   - If the laptop woke up while running on battery under `SaveBattery` mode, `PauseReason::PowerSaving` immediately applies, keeping sessions safely paused without transient flapping.

---

## 6. Multi-Monitor Presentation Modes & Synchronization

The power policy applies system-wide across all connected monitors while respecting presentation topology:
- **Independent Mode**: All displays suspend when power-saving triggers fire, and resume concurrently when power returns (provided no display-specific fullscreen or manual pause blocks resumption).
- **Span Mode**: The unified virtual desktop session suspends and resumes atomically.
- **Clone Mode**: All cloned displays suspend and resume synchronously.

---

## 7. User Interface & Privacy

- **DisplaysView UI**: Under **Smart Playback Policy**, a dedicated **Power-State & Battery-Sensitive Playback** card presents:
  - Live power status badge (e.g. `⚡ AC Power (Desktop)`, `🔋 Battery 85%`, `⚡ AC Charging 90%`).
  - Segmented mode buttons (`Off (Standard)`, `Balanced`, `Save Battery`).
  - Transparent privacy notice: power status is queried locally via Win32; zero device telemetry, battery logs, or power-plan modifications are ever executed.

