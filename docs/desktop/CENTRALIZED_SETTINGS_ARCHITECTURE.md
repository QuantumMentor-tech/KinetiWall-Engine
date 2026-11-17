# Centralized Settings Architecture, Persistent Preferences, Validation & Safe Migrations

## Document ID: KW-DESK-060 | Step 60 of 90
**Subsystem**: Native Desktop Application & Core Subsystem (`kinetiwall-desktop`)  
**Specification Ref**: KW-DAT-004 (`config.json` Schema V1) & KW-DAT-007 (Atomic Storage & Backup)  
**Status**: Implemented & Verified  

---

### 1. Architectural Scope & Mandate

This subsystem establishes a centralized, typed, crash-resilient configuration architecture for the KinetiWall engine. It provides a single source of truth for persistent user preferences across general behavior, performance policies, rendering options, audio settings, and multi-monitor assignments without competing configuration stores, policy bypasses, or data loss.

```
                      +---------------------------------------+
                      |           KinetiWallConfig            |
                      |        (Schema V1 - KW-DAT-004)       |
                      +-------------------+-------------------+
                                          |
         +--------------------------------+--------------------------------+
         |                                |                                |
         v                                v                                v
+------------------+            +------------------+            +--------------------+
|  GeneralConfig   |            | PerformanceConfig|            |  RenderingConfig   |
| - theme          |            | - framerates     |            | - hw acceleration  |
| - autostart      |            | - power actions  |            | - vsync            |
| - language       |            | - policy toggles |            | - default stretch  |
+------------------+            +--------+---------+            +--------------------+
                                         |
         +-------------------------------+---------------------------------+
         |                               |                                 |
         v                               v                                 v
+------------------+            +------------------+            +--------------------+
|   AudioConfig    |            | DiagnosticsConfig|            | Policy Bridges     |
| - master_volume  |            | - log_level      |            | - Fullscreen, Power|
| - master_mute    |            | - retention_days |            | - DisplayPower, Idle|
+------------------+            +------------------+            | - Session, Adaptive|
                                                                +--------------------+
```

---

### 2. Core Architectural Guarantees

1. **Deterministic Single Source of Truth**:
   - Primary configuration file is located at `%LOCALAPPDATA%\KinetiWall\config.json`.
   - Typed Rust model `KinetiWallConfig` defined in `crates/kinetiwall-desktop/src/settings.rs`.
   - Verified backup file maintained automatically at `%LOCALAPPDATA%\KinetiWall\config.json.bak`.

2. **Atomic Write Pipeline (`KW-DAT-007`)**:
   - Target configuration is serialized to pretty-printed JSON.
   - Payload is written to a temporary staging file: `config.json.tmp`.
   - Hardware write buffers are flushed via `file.sync_all()`.
   - Atomically replaces the target destination via `fs::rename()` (Win32 `MoveFileExW`).
   - On successful replacement, a verified replica is copied to `config.json.bak`.

3. **Crash Recovery & Corruption Quarantine**:
   - **Missing Primary**: If `config.json` is missing or deleted, the repository checks for `config.json.bak`. If valid, it auto-restores the backup and logs a warning.
   - **Zero-Byte / Truncated File**: If a sudden power cut truncates `config.json`, the repository auto-recovers from `config.json.bak`.
   - **Corrupted Syntax**: If both primary and backup are corrupt or backup is missing, the repository isolates the corrupted file by renaming it to `config.json.corrupt_<timestamp>`. Fresh factory defaults are generated, saved, and loaded cleanly, ensuring the engine boots reliably without crashing.

4. **Strict Input Validation & Bounded Clamping**:
   - `validate()` strictly enforces runtime bounds:
     - AC Target Framerate: `[15, 240]` FPS.
     - Battery Target Framerate: `[15, 60]` FPS.
     - Audio Master & Monitor Volumes: Finite, `[0.0, 1.0]`. Rejects `NaN` and `Inf`.
     - Log Retention: `[1, 30]` days.
     - Log Level: Validated against `["trace", "debug", "info", "warn", "error"]`.
     - Language: Non-empty string.
   - `sanitize()` provides graceful clamping for corrupted values loaded from disk.

5. **Safe Schema Migrations**:
   - Monotonically increasing `schema_version` (Version 1).
   - Unversioned or V0 configuration files are automatically upgraded to Schema V1, populating missing mandatory sections with safe defaults while preserving custom properties.
   - Future schema versions (`found > CURRENT_CONFIG_SCHEMA_VERSION`) are rejected safely without overwriting or corrupting user files.

6. **Lossless Forward Compatibility**:
   - `#[serde(flatten)] pub unknown_fields: HashMap<String, serde_json::Value>` captures future or experimental configuration options on deserialization and re-serializes them without data loss.

7. **Direct Policy Arbitration Integration**:
   - Bridges configuration settings directly into typed policy structs (`FullscreenPlaybackSettings`, `PowerPlaybackSettings`, `DisplayPowerPlaybackSettings`, `InactivityPlaybackSettings`, `SessionPlaybackSettings`, `AdaptiveQualitySettings`, `ApplicationResourceBudget`) without creating competing policy engines or bypassing central arbitration.

---

### 3. File Layout & Concrete Artifacts

| Component | Path | Description |
| :--- | :--- | :--- |
| **Settings Module** | `crates/kinetiwall-desktop/src/settings.rs` | Types, validation rules, migration engine, and `FileSettingsRepository`. |
| **Module Export** | `crates/kinetiwall-desktop/src/lib.rs` | Public re-exports for workspace consumers. |
| **Settings Integration Tests** | `crates/kinetiwall-desktop/tests/settings_tests.rs` | 12 automated integration tests verifying persistence, recovery, quarantine, and migrations. |
| **Architectural Spec** | `docs/desktop/CENTRALIZED_SETTINGS_ARCHITECTURE.md` | Authoritative documentation of configuration topology and recovery guarantees. |

---

### 4. Verification & Reliability Gate Status

- **Settings Integration Tests**: 12/12 passed (0 warnings, 0 failures).
- **Desktop Subsystem Tests**: 227/227 passed across 20 test suites.
- **Workspace Rust Tests**: 341/341 passed across all 6 crates.
- **UI Policy & Store Tests**: 47/47 passed across 6 test suites.
- **Verification Gate (`npm run verify:gate`)**: Clean pass across all linters, TypeScript contracts, and policy harnesses.

