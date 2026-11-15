# KinetiWall Audio-Reactive Wallpapers & WASAPI Loopback Integration Architecture (`Step 34`)

Crate: [`crates/kinetiwall-audio`](../../crates/kinetiwall-audio)  
Web Player Integration: [`crates/kinetiwall-player-web`](../../crates/kinetiwall-player-web)  
Security Specification: [`docs/security_and_ipc/WEBVIEW2_SECURITY_SPECIFICATION.md`](../security_and_ipc/WEBVIEW2_SECURITY_SPECIFICATION.md)

## 1. Architectural Overview & Boundary Separation
Step 34 introduces real-time audio reactivity into KinetiWall using Windows Audio Session API (WASAPI) shared-mode loopback capture.

In strict adherence to KinetiWall's process isolation architecture (`ADR-0006`, `CTR-02`, `CTR-05`, `CTR-07`):
- **Core Controller / Resident Service (`AudioReactivityService`)**:
  - Owns capture authorization, user preference enforcement, and lifecycle coordination.
  - Ensures capture is **opt-in only** (disabled by default) and only runs when actively requested.
- **Audio Capture Component (`WasapiLoopbackSession` / `NativeWasapiBackend`)**:
  - Executes on a dedicated worker thread (`kinetiwall-audio-worker`) initialized with multithreaded COM (`COINIT_MULTITHREADED`).
  - Directly taps the default console audio rendering endpoint (`eRender`, `eConsole`) in shared loopback mode (`AUDCLNT_STREAMFLAGS_LOOPBACK`).
  - Does **not** access microphone inputs, install virtual audio drivers, or inject code into other processes.
- **Real-Time DSP Feature Extractor (`AudioFeatureExtractor`)**:
  - Normalizes and converts raw interleaved audio buffers (IEEE Float, 16-bit, 24-bit, 32-bit PCM) to mono float vectors.
  - Extracts normalized RMS energy, peak amplitude transients with exponential decay, and 8 acoustic frequency bands via a 256-point Hann-windowed Radix-2 FFT.
  - Enforces finite ranges (`0.0..=1.0`), rejects NaN/Inf, and detects ambient silence (-60 dBFS noise gate).
- **Wallpaper Renderer Interface**:
  - Delivers compact, versionable `VisualizerFrame` payloads to the isolated web player over standard I/O pipes.
  - Uses `PostWebMessageAsJson` / DOM events without granting the DOM any native COM or audio subsystem privileges.

```text
Host System Output Audio (Speakers / Headphones)
         │
         ▼  (Shared-mode WASAPI Loopback: eRender)
[kinetiwall-audio-worker Thread]
   ├── WasapiLoopbackSession (Packet polling / GetBuffer / ReleaseBuffer)
   ├── Format Normalization (Float/PCM16/PCM24/PCM32 -> mono f32)
   └── AudioFeatureExtractor (RMS + Peak + Radix-2 FFT -> 8 Frequency Bands)
         │
         ▼ Bounded Frame Channel (30/60 FPS, drop-oldest policy)
AudioReactivityService (Controller Core)
         │
         ▼ Line-delimited JSON IPC (`ToHost::VisualizerUpdate`)
Isolated Player Process (`kinetiwall-player-web.exe`)
         │
         ▼ `PostWebMessageAsJson` (Sandboxed WebView2)
Wallpaper DOM (Reference Visualizer / Canvas Animation)
```

## 2. Minimum Data Contract (`VisualizerFrame`)
To prevent unbounded memory growth, serialization overhead, or security leaks, raw audio buffers are **never** passed across IPC or exposed to the renderer. Renderers receive exclusively the bounded `VisualizerFrame` schema:

| Field | Type | Range / Format | Description |
|---|---|---|---|
| `sequence` | `u64` | Monotonic integer | Identifies frame order and detects dropped or stale updates. |
| `timestamp_ms` | `u64` | Milliseconds | Monotonic or epoch timestamp for time-synchronized smoothing. |
| `rms` | `f32` | `0.0..=1.0` | Normalized root-mean-square energy level. |
| `peak` | `f32` | `0.0..=1.0` | Peak amplitude with exponential transient decay. |
| `frequency_bands` | `[f32; 8]` | Fixed 8-element array, `0.0..=1.0` each | Normalized acoustic band energies (Sub-bass through Highs). |
| `is_silent` | `bool` | `true` or `false` | True when signal is below noise floor (-60 dBFS). |

### Acoustic Band Groupings:
1. **Sub-bass:** 20 Hz – 60 Hz
2. **Bass:** 60 Hz – 250 Hz
3. **Low-mid:** 250 Hz – 500 Hz
4. **Mid:** 500 Hz – 1500 Hz
5. **Upper-mid:** 1500 Hz – 3000 Hz
6. **Presence:** 3000 Hz – 6000 Hz
7. **Brilliance:** 6000 Hz – 12000 Hz
8. **Highs:** 12000 Hz – 20000 Hz

## 3. Privacy, Security & Resource Guarantees
1. **Strictly Opt-In**: Audio reactivity is disabled by default (`enabled: false`). Enabling requires explicit user action.
2. **Local-Only Processing**: All audio samples are processed in transient RAM buffers and immediately discarded. No audio is ever recorded, written to disk, or transmitted over any network.
3. **No Microphone Access**: Only system output audio is captured via loopback. Microphone endpoints are never opened or queried.
4. **Zero DOM Privilege Escalation**: Wallpaper JavaScript cannot configure audio devices, start/stop streams, or query audio hardware. Visualizers receive only passive numerical JSON messages.
5. **Bounded Resource Use**:
   - Fixed-size arrays: 0 per-frame heap allocations during DSP processing.
   - Bounded rate limiting: Updates are throttled to `target_fps` (e.g. 30 or 60 Hz). If the renderer lags, stale frames are dropped rather than accumulating in queues.
   - Sleep intervals: When audio is silent or buffers are empty, worker threads sleep briefly to prevent CPU spinning.

## 4. Lifecycle & Fault Recovery
- **Pause & Suspend**: When a wallpaper is paused or the desktop is locked, `service.suspend()` suspends the capture worker without tearing down COM allocations.
- **Stop & Shutdown**: `service.stop()` / `service.shutdown()` signals the worker thread via `AtomicBool`, stops `IAudioClient`, drains pending broadcaster queues, releases all COM interfaces, calls `CoUninitialize()`, and cleanly joins the thread.
- **Device Changes**: If the default audio endpoint is invalidated or disconnected, `AudioCaptureState::Unavailable` is reported honestly without crashing the application shell or player processes.
- **Renderer Failure**: If the web player crashes or exits, audio delivery stops immediately without leaking orphaned capture streams.

## 5. Automated Verification Matrix
- **Unit & Integration Tests (`crates/kinetiwall-audio/tests/audio_tests.rs`)**:
  - `test_silence_input_produces_stable_silence_frame`: Confirms all-zero buffers report `is_silent = true` and zero energy.
  - `test_known_amplitude_sine_wave`: Confirms 440 Hz synthetic tone produces expected normalized RMS and peak.
  - `test_peak_transient_detection_and_decay`: Validates transient peak capture and smooth exponential decay.
  - `test_frequency_band_separation_for_synthetic_tones`: Confirms 100 Hz tone excites Bass (Band 1) and 8000 Hz excites Brilliance (Band 6).
  - `test_format_conversions_pcm_and_float`: Validates byte conversion for IEEE Float 32-bit and PCM 16/24/32-bit.
  - `test_numerical_safety_nan_infinity_rejection`: Validates NaN and infinite value neutralization and range clamping.
  - `test_empty_and_truncated_buffers`: Verifies resilience against zero-length or partial buffers.
  - `test_temporal_smoothing_determinism`: Verifies deterministic exponential moving average behavior.
  - `test_radix2_fft_power_of_two`: Verifies Cooley-Tukey Radix-2 spectral accuracy.
  - `test_audio_service_lifecycle_opt_in_and_state`: Verifies opt-in gate, transitions, suspend, resume, and idempotent stop.
  - `test_audio_service_rate_limiter_and_bounded_broadcaster`: Verifies 60 FPS frame distribution and monotonic sequence increments.
  - `test_wasapi_runtime_probe`: Verifies native Windows WASAPI loopback capture on real system audio endpoints.
- **Frontend UI Tests (`src-ui/tests/audioReactivityUi.test.ts`)**:
  - Verifies default disabled state, parameter clamping, truthful status badge updates, and web-vs-video wallpaper conditional rendering.

