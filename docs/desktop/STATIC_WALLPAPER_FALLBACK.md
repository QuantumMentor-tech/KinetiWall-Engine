# KinetiWall Static Wallpaper Fallback, Last-Frame Preservation & Reversible Low-Power Transitions

**Phase:** Phase 8, Intelligent Playback Policies & System-Aware Optimization  
**Step:** 50 of 90  
**Target Platform:** Windows 10 & Windows 11  
**Category:** Native Window Lifecycle, Static Fallback Transitions, Capability Boundaries, Cache Governance, and Lossless Resumption  

---

## 1. Architectural Overview

Step 50 implements a reliable, capability-driven static wallpaper fallback mechanism that preserves the visual presentation of active wallpaper sessions when explicitly configured power-saving policies (from Step 49) require live playback to be suspended. Rather than leaving the desktop surface blank or detached, KinetiWall transitions eligible sessions into a high-fidelity still-image presentation, snapshots all vital playback state, and guarantees lossless restoration when normal power conditions resume.

```
┌─────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                   Fallback Capability Assessment                                        │
│  - Video Media (libmpv): Supports static image, poster, live frame capture, media position restoration  │
│  - Web Bundles (WebView2): Supports static image & catalog poster; strictly NO live frame capture/JS    │
│    state serialization (truthful capability advertising)                                                │
└────────────────────────────────────────────────────┬────────────────────────────────────────────────────┘
                                                     │ Capability Descriptor
                                                     ▼
┌─────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                   Source Resolution Priority Pipeline                                   │
│  1. Explicit user-configured fallback file (if path exists and passes size/dimension limits)             │
│  2. Catalog approved poster / thumbnail (metadata.thumbnail_ref if verified on disk)                    │
│  3. Backend-native live frame capture of KinetiWall's own surface HWND (video only)                     │
│  4. Graceful degradation: Fall back to standard suspension if no image is available                     │
└────────────────────────────────────────────────────┬────────────────────────────────────────────────────┘
                                                     │ FallbackSourceKind
                                                     ▼
┌─────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                   Static Fallback Coordinator & Cache                                   │
│  - FallbackAssetCache: LRU eviction, MAX_FALLBACK_DIMENSION (4096px), MAX_FILE_SIZE (32MB)                │
│  - Storage Safety: ONLY deletes transient capture files on eviction; NEVER touches catalog/user files   │
│  - PreservedSessionState: Captures session_id, wallpaper_id, geometry, seek position, volume, mute      │
│  - State Machine: Idle -> Evaluating -> AcquiringImage -> DisplayingFallback -> RestoringLive           │
└────────────────────────────────────────────────────┬────────────────────────────────────────────────────┘
                                                     │ DisplaySessionState::StaticFallback
                                                     ▼
┌─────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                   DisplaySessionCoordinator & Multi-Monitor                             │
│  - Independent: Each monitor evaluates and transitions to fallback separately                           │
│  - Clone Mode: Secondary displays share the authoritative primary monitor's fallback asset              │
│  - Span Mode: Virtual desktop spanned bounds covered with coordinated fallback asset                    │
│  - Pause Interlock: Resuming clears fallback ONLY if no other pause reasons (UserManual/Fullscreen)     │
└─────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

---

## 2. Fallback Capability Matrix

KinetiWall establishes strict architectural and capability boundaries between media backends:

| Capability Field | Video (`libmpv`) | Web (`WebView2`) | Rationale / Architectural Boundary |
|---|:---:|:---:|---|
| `supports_static_image` | **Yes** | **Yes** | Both presentation engines or fallback host windows can render static images on the desktop surface. |
| `supports_catalog_poster` | **Yes** | **Yes** | Both wallpaper types can supply a verified thumbnail or poster image in catalog metadata. |
| `supports_live_frame_capture` | **Yes** | **No** | Video frames can be deterministically captured from the dedicated player window HWND (`PrintWindow` / GDI). Web engines run out-of-process multi-threaded DOM renderers where freezing frames without continuous GPU compositor execution is non-deterministic. |
| `supports_media_position_restore` | **Yes** | **No** | Video players track playback position in milliseconds (`media_position_ms`) and seek accurately upon resume. |
| `supports_js_state_restore` | **No** | **No** | Arbitrary JavaScript runtime state, event listener closures, and DOM tree states cannot be safely serialized across process lifetimes. Interactive web wallpapers restart cleanly upon resume. |

---

## 3. Fallback Source Resolution Priority

When power policy signals that a wallpaper session should enter low-power presentation, `StaticFallbackCoordinator` resolves the image asset using an explicit 4-tier pipeline:

1. **Priority 1: Explicit Static Fallback Asset**  
   If the user has explicitly configured a dedicated fallback image file for the wallpaper or display, and the file exists on disk within size limits, it is chosen.
2. **Priority 2: Catalog Poster / Preview Thumbnail**  
   If no explicit file exists, the coordinator inspects `WallpaperMetadata::thumbnail_ref`. If the thumbnail file exists on local disk, it is selected as the static presentation asset.
3. **Priority 3: Backend-Native Live Frame Capture**  
   If no static file or poster is available and the backend supports live capture (Video), the coordinator invokes `FrameCaptureProvider::capture_frame` on KinetiWall's own surface HWND. The frame is written to the cache directory as a transient PNG.
4. **Priority 4: Standard Suspension Degradation**  
   If none of the above sources yield a usable image (e.g. Web wallpaper with missing poster, or locked compositor), the session safely degrades to standard suspension (`DisplaySessionState::Suspended`) without crashing or corrupting state.

---

## 4. Cache Management & Storage Safety

The fallback subsystem enforces strict bounds to prevent runaway disk usage or memory bloat:

- **Maximum Image Dimension:** 4096 pixels (`MAX_FALLBACK_DIMENSION`). Images exceeding this are rejected with `FallbackError::AssetExceedsBounds`.
- **Maximum File Size:** 32 MB (`MAX_FALLBACK_FILE_SIZE_BYTES`). Oversized assets are rejected with `FallbackError::AssetExceedsFileSize`.
- **Maximum Retained Descriptors:** 16 assets in LRU memory (`MAX_RETAINED_FALLBACK_ASSETS`).
- **Disk Safety Rule:** The `FallbackAsset::is_transient` flag distinguishes temporary frame captures generated by KinetiWall from user files and catalog records. When the cache evicts an asset, **only transient files are deleted from disk**. User-owned files and catalog assets are strictly read-only and never deleted.

---

## 5. Lossless Resumption & Pause Interlock

Resumption from static fallback guarantees complete fidelity and policy interlock:

1. **Position Restoration:** When returning to live video playback, the player backend restores the preserved timestamp (`media_position_ms`), audio volume, and mute configuration.
2. **Pause Interlock:** When low power conditions clear (e.g. AC power reconnected), `restore_from_fallback` inspects all active pause reasons (`HashSet<PauseReason>`).
   - If **no other pause reasons survive**, the session transitions from `StaticFallback` to `Active`.
   - If **competing pause reasons survive** (such as `PauseReason::UserManual` or `PauseReason::Fullscreen`), the session transitions to `Suspended` and live playback remains paused, upholding user intent.

---

## 6. Multi-Monitor Presentation Modes

- **Independent Mode:** Each monitor evaluates its assigned wallpaper's capability, acquires its own fallback asset, and transitions independently.
- **Clone Mode:** Secondary clone displays share the authoritative primary display's resolved fallback source presentation, guaranteeing synchronous visual identity.
- **Span Mode:** Spanned wallpaper surfaces utilize the virtual desktop bounding geometry with coordinated fallback presentation.

---

## 7. Verification & Test Suite

The static wallpaper fallback implementation is validated by 12 dedicated automated tests in `crates/kinetiwall-desktop/tests/fallback_tests.rs`:

1. `test_fallback_capability_matrix_video_vs_web`: Validates truthful capability reflection between Video and Web media.
2. `test_fallback_source_resolution_priority`: Validates exact 4-tier resolution hierarchy.
3. `test_static_fallback_transition_and_session_state`: Verifies state mutation to `DisplaySessionState::StaticFallback` and state preservation.
4. `test_fallback_duplicate_transition_is_idempotent`: Validates that duplicate transition requests do not overwrite preserved state.
5. `test_safe_restoration_to_active_when_power_cleared`: Validates clean return to `Active` with preserved media position.
6. `test_reversibility_multi_reason_pause_interlock`: Proves that user manual pause blocks automatic resumption when AC power returns.
7. `test_fallback_cache_budget_and_dimension_validation`: Enforces 4096px and 32MB bounds.
8. `test_cache_eviction_strictly_protects_non_transient_assets`: Confirms user and catalog assets are never deleted during eviction.
9. `test_multi_monitor_independent_fallback`: Validates isolated fallback per monitor.
10. `test_multi_monitor_clone_mode_shared_fallback`: Validates shared primary fallback across cloned monitors.
11. `test_frame_capture_failure_gracefully_degrades_to_standard_suspension`: Tests error containment and fallback degradation.
12. `test_bounded_diagnostic_event_history`: Verifies bounded 128-event diagnostic ring log.

