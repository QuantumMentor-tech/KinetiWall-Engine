# Wallpaper Library Domain Model & Local Asset Catalog Foundation
**Document ID:** `KW-CAT-001`  
**Phase:** 5 (Wallpaper Library & Local Asset Management)  
**Step:** 25 of 90  
**Status:** Implemented Foundation (`crates/kinetiwall-catalog`)  
**Classification:** Core Domain & Local Persistence Architecture  

---

## 1. Overview & Architectural Alignment

This document describes the **implemented** wallpaper catalog foundation in [`crates/kinetiwall-catalog/`](file:///e:/Projects/All%20working%20projects/App%20projects%20working/KinetiWall%20Engine%20(working)/crates/kinetiwall-catalog/Cargo.toml), built strictly in conformance with KinetiWall's Phase 2 Data Architecture (`KW-DAT-001` through `KW-DAT-007`) and Architecture Decision Records (`ADR-0001`, `ADR-0009`, `DEC-DATA-01`, `DEC-DATA-02`).

### Scope Boundaries
The `kinetiwall-catalog` crate **owns only** local wallpaper catalog identity, metadata validation, asset reference path safety, read-only availability probing, and local Hybrid JSON persistence.

It intentionally **excludes**:
- Media playback (`libmpv`) or web rendering (`WebView2`).
- Win32 desktop attachment (`WorkerW` / `Progman`), child process spawning, or monitor assignment.
- Library browsing UI, drag-and-drop import, recursive directory scanning, or thumbnail generation.
- Cloud connectivity, remote accounts, or telemetry uploads.

---

## 2. Implemented Crate Layout

| Module | File Path | Responsibility |
| :--- | :--- | :--- |
| **Crate Root** | [`lib.rs`](file:///e:/Projects/All%20working%20projects/App%20projects%20working/KinetiWall%20Engine%20(working)/crates/kinetiwall-catalog/src/lib.rs) | Enforces `#![forbid(unsafe_code)]`; exports public domain, validation, path-safety, and repository APIs. |
| **Domain Model** | [`model.rs`](file:///e:/Projects/All%20working%20projects/App%20projects%20working/KinetiWall%20Engine%20(working)/crates/kinetiwall-catalog/src/model.rs) | Defines `WallpaperId`, `WallpaperType`, `WallpaperSourceKind`, `AssetReference`, `AvailabilityState`, `WallpaperMetadata`, `CatalogLifecycle`, `WallpaperRecord`, `StoredManifestV1`, and `CatalogEntrySummary`. |
| **Validation** | [`validation.rs`](file:///e:/Projects/All%20working%20projects/App%20projects%20working/KinetiWall%20Engine%20(working)/crates/kinetiwall-catalog/src/validation.rs) | Enforces schema version `1`, metadata field bounds, SemVer `X.Y.Z`, lifecycle ordering, and type-to-entrypoint compatibility. |
| **Path Safety** | [`path_safety.rs`](file:///e:/Projects/All%20working%20projects/App%20projects%20working/KinetiWall%20Engine%20(working)/crates/kinetiwall-catalog/src/path_safety.rs) | Validates relative managed vs. absolute/UNC external paths, blocks traversal & forbidden executables, checks canonical containment, normalizes comparison keys, and probes read-only availability. |
| **Registration** | [`registration.rs`](file:///e:/Projects/All%20working%20projects/App%20projects%20working/KinetiWall%20Engine%20(working)/crates/kinetiwall-catalog/src/registration.rs) | Implements `WallpaperRegistrationService`, `AssetSelectionSource`, bounded container/header inspection (`512` bytes max), duplicate/idempotency rules, and `RegistrationBatchReport`. |
| **Repository** | [`repository.rs`](file:///e:/Projects/All%20working%20projects/App%20projects%20working/KinetiWall%20Engine%20(working)/crates/kinetiwall-catalog/src/repository.rs) | Defines `WallpaperCatalogRepository` and `LocalJsonCatalogRepository` (atomic temp-file replace, startup index load, corrupt manifest isolation). |
| **Errors & Privacy** | [`error.rs`](file:///e:/Projects/All%20working%20projects/App%20projects%20working/KinetiWall%20Engine%20(working)/crates/kinetiwall-catalog/src/error.rs) | Structured `CatalogError`, `ValidationError`, `PathSafetyError`, `PersistenceError`, and `redact_path_for_diagnostics`. |

---

## 3. Domain Model & Identity Rules

### 3.1 Stable Identity (`WallpaperId`)
- Implemented as a transparent newtype around RFC 4122 UUIDv4 (`uuid::Uuid`).
- Independent of wallpaper title, filename, array position, or source directory path.
- Preserved across `update_metadata()` and catalog reopen cycles.

### 3.2 Explicit Type & Source Representation
- **`WallpaperType`:** Constrained enum `{ Video, Web }` serialized in `snake_case` (`"video"`, `"web"`). Unknown or unsupported strings fail deserialization rather than coercing to a valid type.
- **`WallpaperSourceKind`:** Constrained enum `{ ManagedPackage, ExternalDirectFile }`.
- **Type-to-Entrypoint Consistency:**
  - `Video`: accepts `.mp4`, `.webm`, `.avi`, `.mkv`, `.mov`, `.m4v`.
  - `Web`: accepts `.html`, `.htm`.
  - Mismatches (e.g., `Video` pointing to `.html` or `Web` pointing to `.mp4`) are rejected with `ValidationError::IncompatibleAssetType`.

### 3.3 Representation Separation
- **Domain Record (`WallpaperRecord`):** Internal model used by catalog operations.
- **Persistent Manifest (`StoredManifestV1`):** Disk schema (`manifest_version: 1`) stored in `{library_root}/{id}/project.json`, preserving unknown forward-compatible fields (`extra_fields`).
- **IPC Summary (`CatalogEntrySummary`):** Service/frontend DTO omitting raw host absolute paths.

---

## 4. Persistence & Schema Versioning

In accordance with **`ADR-0009`** and **`DEC-DATA-01`** (which explicitly rejected SQLite for V1.0 in favor of the Hybrid JSON Architecture):
- Each entry is stored at `{library_root}/{wallpaper_id}/project.json`.
- Writes use `atomic_write_json`: serialize JSON $\rightarrow$ write `{path}.json.tmp` $\rightarrow$ `sync_all()` (`FlushFileBuffers`) $\rightarrow$ `std::fs::rename()` (`MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`).
- Schema version is checked explicitly via `manifest_version` before typed parsing; unexpected versions (e.g. `0` or `>= 2`) fail safely with `CatalogError::UnsupportedSchemaVersion`.
- On `LocalJsonCatalogRepository::open()`, invalid or corrupt sibling manifests are preserved untouched on disk and recorded in `load_issues()` so one broken entry never crashes or drops the rest of the catalog.

---

## 5. Path Safety & User File Ownership Invariants

1. **Managed vs. External Path Boundaries:**
   - `ManagedPackage` entrypoints and `thumbnail_ref` values must be relative paths without `..`, drive prefixes, or leading slashes, and are canonicalization-checked against `{library_root}/{id}` when present on disk.
   - `ExternalDirectFile` entrypoints must be strictly absolute local (`C:\...`) or UNC (`\\server\share\...`) paths supporting Unicode filenames and spaces.
   - Executable and script extensions (`.exe`, `.dll`, `.bat`, `.cmd`, `.ps1`, `.vbs`, `.js`, `.lnk`, etc.) and NUL bytes are unconditionally rejected.
2. **Non-Destructive Ownership:**
   - `create`, `update_metadata`, `check_availability`, and `remove_record` **never delete, rename, move, or rewrite** user-owned external files or package media payloads.
   - `remove_record` deletes only `project.json` and calls non-recursive `std::fs::remove_dir` on the package folder (which only succeeds if empty).
3. **Honest Unavailable Asset States:**
   - `AvailabilityState` defaults to `Unchecked` on load and transitions via `check_availability()` to `Available`, `Missing`, `Inaccessible { reason }`, or `InvalidAsset { reason }`.
   - Missing or locked assets are never silently purged from the catalog.

---

## 6. Local Asset Selection, Validation & Registration Workflow (`Step 26`)

`WallpaperRegistrationService` ([`registration.rs`](file:///e:/Projects/All%20working%20projects/App%20projects%20working/KinetiWall%20Engine%20(working)/crates/kinetiwall-catalog/src/registration.rs)) orchestrates user-initiated registration of local files without copying, moving, or altering source files:

1. **Decoupled Selection Boundary:**
   - Selection is injected via `AssetSelectionSource::select_assets()`, returning `SelectionOutcome::Selected(Vec<PathBuf>)` or `SelectionOutcome::Cancelled`.
   - Cancellation resolves cleanly with `BatchOverallStatus::Cancelled`, zero records created, and zero active loading state.
   - An atomic in-flight guard (`RegistrationError::OperationInProgress`) prevents concurrent or rapid repeated submissions from racing.
   - Single batches are bounded at `MAX_REGISTRATION_BATCH_SIZE = 500`.
2. **Authoritative Validation & Bounded Header Inspection:**
   - Rejects unexpected directories (`UnexpectedDirectory`), forbidden executable/script extensions, NUL bytes, empty (`0`-byte) files, and executable `MZ` headers disguised under media extensions.
   - Reads **at most 512 bytes** (never loading multi-GB media files into memory or executing HTML/JS):
     - **Video (`.mp4`, `.m4v`, `.mov`):** Verifies `ftyp` ISO-BMFF container header.
     - **Video (`.webm`, `.mkv`):** Verifies EBML signature (`0x1A 0x45 0xDF 0xA3`).
     - **Video (`.avi`):** Verifies `RIFF....AVI ` header.
     - **Web (`.html`, `.htm`):** Verifies non-empty UTF-8 text prefix without NUL bytes.
   - *Limitation:* Header inspection verifies container/format eligibility at registration time only; full codec/frame decoding and runtime rendering remain owned by future isolated player steps.
3. **Reference Registration & Ownership:**
   - Valid selections are registered as `WallpaperSourceKind::ExternalDirectFile` with display titles derived safely from the filename stem (`derive_display_title`) without mutating source filenames or contents.
4. **Duplicate & Partial-Failure Semantics:**
   - Uses `normalize_external_comparison_key()` (case-normalized on Windows) to detect duplicates both against existing catalog entries and within the same batch.
   - Duplicates return `RegistrationItemOutcome::AlreadyPresent { existing_id, summary }` without overwriting customized metadata.
   - Multi-item selections validate and persist independently, distinguishing `Succeeded`, `PartialSuccess`, `AllFailed`, `Cancelled`, and `EmptySelection`, with all paths redacted in returned reports.

---

## 7. Wallpaper Library Browsing UI & Responsive Catalog Grid (`Step 27`)

The frontend library browsing experience in [`src-ui/`](file:///e:/Projects/All%20working%20projects/App%20projects%20working/KinetiWall%20Engine%20(working)/src-ui/views/LibraryView.ts) connects the Step 22 Application Shell to `kinetiwall-catalog`'s `CatalogLibrarySnapshot` DTO (`LocalJsonCatalogRepository::library_snapshot()`):

1. **Authoritative Catalog Snapshot Contract:**
   - `CatalogEntrySummary` exposes `id`, `title`, `description`, `wallpaper_type`, `source_kind`, `author`, `tags`, `availability`, `created_at`, `updated_at`, `is_builtin`, `has_thumbnail_ref`, and validated package-relative `thumbnail_ref` without ever leaking host filesystem paths (`C:\...` or `\\server\...`).
   - `CatalogLibrarySnapshot` bundles `entries`, `integrity: CatalogIntegrityReport`, and redacted `load_issues: Vec<CatalogLoadIssue>`.
2. **Responsive Collection Grid & Card Hierarchy:**
   - `LibraryView` ([`src-ui/views/LibraryView.ts`](file:///e:/Projects/All%20working%20projects/App%20projects%20working/KinetiWall%20Engine%20(working)/src-ui/views/LibraryView.ts)) adapts column counts across supported desktop viewports (`960px` $\rightarrow$ 2 cols, `1280px` $\rightarrow$ 3 cols, `1920px` $\rightarrow$ 4 cols, `2560px` $\rightarrow$ 5 cols, `3840px` $\rightarrow$ 6 cols).
   - `WallpaperCard` ([`src-ui/components/domain/WallpaperCard.ts`](file:///e:/Projects/All%20working%20projects/App%20projects%20working/KinetiWall%20Engine%20(working)/src-ui/components/domain/WallpaperCard.ts)) keys cards by `entry.id` (`WallpaperId`), resolves valid package-relative thumbnails to `kinetiwall-asset://` URLs with graceful placeholder fallback on absence or load error, and displays icon+text badges for both `WallpaperType` and `AvailabilityState` (`Available`, `Unverified`, `File Missing`, `Inaccessible`, `Invalid Asset`).
3. **Truthful View States & Safe Selection Semantics:**
   - Distinctly renders initial loading, empty library (`"No Wallpapers in Your Library Yet"` with `"Add Wallpaper"` CTA), initial load error with retry, and stale-refresh warnings that preserve previously loaded records.
   - Card selection (`aria-pressed="true"`) opens a read-only inspection panel and never starts playback, spawns player processes, or modifies desktop assignments.

---

## 8. Wallpaper Library Search, Filtering & Sorting (`Step 28`)

The local discovery engine in [`src-ui/utils/catalogQuery.ts`](file:///e:/Projects/All%20working%20projects/App%20projects%20working/KinetiWall%20Engine%20(working)/src-ui/utils/catalogQuery.ts) and [`src-ui/stores/catalogStore.ts`](file:///e:/Projects/All%20working%20projects/App%20projects%20working/KinetiWall%20Engine%20(working)/src-ui/stores/catalogStore.ts) extends `LibraryView` with synchronous, local-first search, intersection filtering, and deterministic sorting over authoritative `CatalogEntrySummary` records:

1. **Searchable Metadata Contract:**
   - Normalizes user search queries by trimming leading/trailing whitespace and folding case without stripping meaningful punctuation.
   - Matches partial substrings against verified catalog text fields (`title`, `author`, `description`, and `tags`), supporting Unicode and punctuation while never matching internal UUIDs or private host paths.
2. **Evidence-Based Filters (`Type` & `Availability`):**
   - **Wallpaper Type Filter (`all | video | web`):** Maps directly to `WallpaperType`; unrecognized/unsupported runtime types are excluded when `'video'` or `'web'` is active.
   - **Availability Filter (`all | available | unavailable | unchecked`):** Maps directly to `AvailabilityState.status` (`available` vs `missing | inaccessible | invalid_asset` vs `unchecked`), never conflating unverified records with available or missing files.
   - Active search and filters combine via logical intersection.
3. **Deterministic Sorting & Tie-Breaking:**
   - Supports `oldest_asc` (`created_at` ascending), `recent_desc` (`created_at` descending), `title_asc` (case-insensitive `title` ascending, secondary exact-case comparison), and `title_desc` (case-insensitive `title` descending).
   - Every sort mode uses `WallpaperId` (`a.id` vs `b.id`) as its final deterministic tie-breaker and never mutates stored catalog records.
4. **Distinct Empty Catalog vs. No Matching Results:**
   - When the catalog has `0` registered records, `LibraryView` renders `library-empty-state` with the Step 26 `"Add Wallpaper"` CTA.
   - When the catalog contains records but active search/filters match `0` items, `LibraryView` renders `library-no-matches-state` with `"Clear Search & Filters"` (`clearSearchAndFilters()`), preserving the user's selected sort order.

---

## 9. Wallpaper Details Panel & Metadata Inspector (`Step 29`)

The read-only details panel in [`src-ui/components/domain/WallpaperDetailsPanel.ts`](file:///e:/Projects/All%20working%20projects/App%20projects%20working/KinetiWall%20Engine%20(working)/src-ui/components/domain/WallpaperDetailsPanel.ts) presents comprehensive, verified metadata for the selected `WallpaperId` without mutating source files, executing media, or starting playback:

1. **Verified Metadata & Provenance Hierarchy:**
   - Displays escaped `title`, `WallpaperType` badge + canonical wire value, `AvailabilityState` badge + detailed explanation + explicit non-playback distinction notice, full `description` (or neutral `"No description provided in catalog metadata."`), all stored `tags` (or `"No tags assigned."`), `author`, `source_kind` (`Managed Package` vs `Local Reference`), `origin` (`Built-in Package` vs `User Registered`), `version`, `min_engine_version`, `license`, `schema_version`, UTC-formatted `created_at` / `updated_at` timestamps, and `Catalog Record ID`.
   - Intentionally omits unverified media-stream properties (duration, resolution, byte size) rather than fabricating values.
2. **Preview & Fallback Separation:**
   - Reuses validated package-relative `thumbnail_ref` via `kinetiwall-asset://` when present; distinguishes `"No Preview Image Referenced"` (`data-reason="absent"`) from `"Preview Image Could Not Be Loaded"` (`data-reason="load_failed"` on image error) without implying the underlying wallpaper asset is broken.
3. **Selection & Focus Lifecycle:**
   - Keyed by stable `WallpaperId`; remains accurate across sort/filter/refresh transitions and displays an informational notice (`inspector-hidden-by-filter-banner`) if active search/filters hide the selected card from the grid.
   - Closing via `Close Details` or `Escape` preserves search, filter, and sort state and restores keyboard focus target (`data-focus-restored="true"`) to the invoking library card.





---

## 10. Wallpaper Catalog Management, Missing-Asset Recovery & Safe Removal (`Step 30`)

Step 30 completes **Phase 5 (`Wallpaper Library & Local Asset Management`)** by adding controlled catalog maintenance workflows in [`crates/kinetiwall-catalog/src/management.rs`](file:///e:/Projects/All%20working%20projects/App%20projects%20working/KinetiWall%20Engine%20(working)/crates/kinetiwall-catalog/src/management.rs), [`src-ui/stores/catalogStore.ts`](file:///e:/Projects/All%20working%20projects/App%20projects%20working/KinetiWall%20Engine%20(working)/src-ui/stores/catalogStore.ts), and [`src-ui/components/domain/WallpaperDetailsPanel.ts`](file:///e:/Projects/All%20working%20projects/App%20projects%20working/KinetiWall%20Engine%20(working)/src-ui/components/domain/WallpaperDetailsPanel.ts):

1. **Allowlisted Metadata Editing (`EditableMetadataPatch`):**
   - Permits updating only user-editable descriptive fields (`title`, `description`, and `tags`) while strictly protecting immutable/system fields (`id`, `schema_version`, `wallpaper_type`, `source_kind`, `created_at`, `is_builtin`, `availability`, `author`, `version`, `min_engine_version`, `license`, `thumbnail_ref`, and `preserved_extra`).
   - Enforces validation at both the UI layer (`validateEditableMetadataPatch`) and Rust domain layer (`validate_metadata`), supports optimistic concurrency checks (`expected_updated_at` $\rightarrow$ `CatalogManagementError::StaleRecordConflict`), and skips redundant disk writes when submitted values match existing metadata (`MetadataUpdateOutcome::Unchanged`).
2. **Missing-Asset Recovery & Validated Relinking (`select_and_relink_asset` / `relink_asset_from_path`):**
   - Allows explicit user-initiated relinking of an existing `WallpaperId` to a replacement local file without creating a duplicate record or altering `id`.
   - Re-runs full path safety (`validate_asset_reference`), filesystem metadata checks, extension classification (`classify_extension_wallpaper_type`), compatibility verification against the record's immutable `wallpaper_type` (`RelinkRejectionReason::TypeMismatch`), cross-record duplicate ownership checks (`RelinkRejectionReason::DuplicateOwnedByOther`), and bounded 512-byte header inspection (`verify_bounded_content_signature`) before atomically updating the manifest and setting `AvailabilityState::Available`.
   - Leaves both the old and new external files untouched on disk and preserves the existing catalog reference if selection is cancelled or validation fails.
3. **Safe Catalog-Only Removal (`remove_catalog_entry` & `ConfirmationDialog`):**
   - Enforces active-assignment (`RemovalOutcome::BlockedActiveAssignment`) and built-in protection (`RemovalOutcome::BlockedBuiltinProtected`) guards so actively assigned or built-in wallpapers cannot be removed while in use.
   - Requires explicit user confirmation via `ConfirmationDialog` (`confirm-remove-dialog`) that clearly states only the catalog record is removed and original media files on disk are **never** deleted, moved, or modified.
4. **End-to-End State Synchronization & Reconciliation:**
   - Confirmed updates immediately synchronize across the library grid, active search/filter/sort results, integrity counters, and `WallpaperDetailsPanel`, while ambiguous persistence errors trigger automatic state reconciliation via `loadCatalog({ forceRefresh: true })`.
