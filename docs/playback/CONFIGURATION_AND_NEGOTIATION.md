# Unified Playback Configuration, Backend Capability Negotiation & Safe Runtime Defaults (Step 36)

## 1. Overview & Architectural Scope

Step 36 establishes a unified, typed, and validated configuration and capability-negotiation layer for KinetiWall's live wallpaper playback subsystem. Building upon the abstractions from Step 31, video engine from Step 32, web engine from Step 33, audio capture from Step 34, and supervision layer from Step 35, this module ensures that:
- Common playback settings (`volume`, `muted`, `loop_mode`, `start_mode`, `scaling_mode`, `hardware_acceleration`) are cleanly decoupled from backend-specific options (`VideoBackendOptions`, `WebBackendOptions`).
- Backends declare verified static and runtime capabilities (`BackendCapabilities`) to negotiate options before player process initialization.
- Invalid numeric values and unsupported optional features safely fall back to documented defaults with diagnostic adjustments rather than crashing or stalling playback.
- Dynamic runtime updates (`RuntimeConfigUpdate`) are verified against generation tokens (`PlayerInstanceId`) to prevent race conditions from mutating newly recovered replacement players.
- Effective configurations are preserved across crash recoveries.

## 2. Configuration Model

### 2.1 Common Playback Options (`CommonPlaybackConfig`)
Settings shared across all playback backends:
- **`volume` (`NormalizedVolume`):** Floating-point audio level bounded in `[0.0, 1.0]` (default `1.0`). Clamped automatically during negotiation if out-of-range, NaN, or infinite.
- **`muted` (`bool`):** Audio mute status (default `false`).
- **`loop_mode` (`LoopMode`):** `Infinite` (default), `Once`, `Count(u32)`.
- **`start_mode` (`StartMode`):** `AutoPlay` (default), `Paused`.
- **`scaling_mode` (`ScalingMode`):** `Fill` (default), `Fit`, `Stretch`, `Center` (matching schema `KW-DAT-004`).
- **`hardware_acceleration` (`HwAccelPreference`):** `Auto` (default), `Disabled`, `ForceHardware`.

### 2.2 Backend-Specific Options (`BackendOptions`)
Isolated behind typed enumeration boundaries:
- **`VideoBackendOptions`:**
  - `hwdec` (`VideoHwDec`): `AutoSafe` (default), `Disabled`, `D3d11va`.
  - `audio_track` (`Option<u32>`): Optional audio stream index.
- **`WebBackendOptions`:**
  - `audio_reactivity` (`Option<AudioReactivityOptions>`):
    - `sensitivity`: Bounded in `[0.1, 3.0]` (default `1.0`).
    - `smoothing`: Bounded in `[0.0, 0.95]` (default `0.65`).
  - `background_color` (`Option<String>`): Hex/CSS color code (e.g. `#000000`).

## 3. Backend Capability Discovery & Negotiation

### 3.1 Capability Descriptor (`BackendCapabilities`)
Each adapter reports its static and runtime capabilities:
```rust
pub struct BackendCapabilities {
    pub supported_types: Vec<WallpaperType>,
    pub supports_volume: bool,
    pub supports_mute: bool,
    pub supports_loop_control: bool,
    pub supports_pause: bool,
    pub supports_dynamic_volume: bool,
    pub supports_dynamic_mute: bool,
    pub supports_dynamic_pause: bool,
    pub supports_hw_accel_selection: bool,
    pub supports_audio_reactivity: bool,
    pub supports_custom_scaling: bool,
    pub is_runtime_available: bool,
    pub runtime_name: String,
}
```

### 3.2 Negotiation Pipeline (`Negotiator::negotiate`)
Before player initialization (`prepare` / `start`), the orchestrator passes the requested `PlaybackConfig` and backend `BackendCapabilities` through `Negotiator::negotiate`:
1. **Runtime Verification:** If `is_runtime_available` is `false`, negotiation rejects immediately with `PlaybackErrorKind::BackendUnavailable`.
2. **Type Compatibility:** If requested backend options do not match `supported_types`, rejects with `PlaybackErrorKind::UnsupportedCapability`.
3. **Safe Value Clamping:** Bounded numbers (`volume`, `sensitivity`, `smoothing`) are clamped to valid ranges. Each adjustment is recorded in `ConfigAdjustment`.
4. **Unsupported Feature Fallback:**
   - If volume control is unsupported, volume is defaulted to `1.0` with an adjustment record.
   - If audio reactivity is unsupported by the backend, it is cleanly disabled without failing playback.
   - If custom loop modes or pause startup are unsupported, they fall back safely to `LoopMode::Infinite` and `StartMode::AutoPlay`.
5. **Effective Configuration:** Yields an `EffectivePlaybackConfig` ready for dispatch to `PlayerAdapter::prepare` and `PlayerAdapter::start`.

## 4. Dynamic Runtime Updates & Recovery Preservation

### 4.1 Runtime Configuration Updates (`RuntimeConfigUpdate`)
For active playback sessions, settings such as volume, mute, or pause can be adjusted live:
- Verified against `PlayerInstanceId`: If the player instance has crashed or been replaced, the update is rejected as `ConfigUpdateResult::StaleInstance`.
- Dispatched through `PlayerAdapter::apply_runtime_config` (which translates to IPC commands e.g. `ToHost::SetVolume`, `ToHost::SetMute`, `ToHost::SetPause`).
- Verified against backend dynamic capabilities: If a backend does not support dynamic volume (e.g. web player), returns `ConfigUpdateResult::UnsupportedSetting`.

### 4.2 Config Preservation Across Supervisor Crash Recovery
When a player crashes and the `PlayerSupervisor` auto-recovers it:
- The supervisor retains the target's effective configuration (including dynamic changes like updated volume).
- The newly spawned replacement instance launches with the identical configuration, preventing audio volume jumps or lost user preferences.

## 5. Test Coverage & Verification

Tested in `crates/kinetiwall-playback/tests/configuration_tests.rs`:
- `test_default_playback_configuration_and_serialization`: Verified default values and JSON round-trip serialization.
- `test_volume_clamping_and_bounds_validation`: Verified strict bounds rejection and safe clamping.
- `test_audio_reactivity_options_bounds`: Verified sensitivity/smoothing bounds clamping.
- `test_negotiation_full_support_video_and_web`: Verified clean negotiation for fully supported configurations.
- `test_negotiation_missing_runtime_fails`: Verified error on missing backend runtime.
- `test_negotiation_unsupported_wallpaper_type`: Verified error on wallpaper type mismatch.
- `test_negotiation_unsupported_optional_features_fallback`: Verified graceful degradation for unsupported optional options.
- `test_supervisor_runtime_config_update_success`: Verified dynamic volume/mute update dispatch.
- `test_supervisor_runtime_config_update_stale_instance_rejected`: Verified rejection of updates with obsolete generation tokens.
- `test_supervisor_runtime_config_preserved_across_recovery`: Verified configuration preservation across automatic process recovery.
- `test_supervisor_runtime_config_unsupported_setting_rejected`: Verified rejection when dynamic capability is absent.

