# Per-Monitor Wallpaper Assignment Model, Persistence & Conflict Resolution (Step 43)

## 1. Overview & Architectural Role

Step 43 establishes the logical assignment subsystem within `crates/kinetiwall-desktop`. This subsystem connects wallpaper catalog entries (`WallpaperId`) to validated application-level display identities (`DisplayId` from Step 42), while maintaining a strict architectural separation between **user intent**, **catalog availability**, **display topology presence**, and **runtime playback/attachment state**.

### Key Architectural Tenets
1. **Saved Intent vs. Runtime Reality**: A saved assignment represents user intent. It does not prove that the display is connected, that the wallpaper asset is currently on disk, that the player process has started, or that a native surface is attached.
2. **Handle Independence**: Assignments reference stable application-level `DisplayId` strings (e.g. `disp_DISPLAY1_MONITOR_DEL4182_1`), never transient Win32 `HMONITOR` handles.
3. **Preservation of Disconnected Displays**: When a display is unplugged or powered down, its saved assignment is preserved safely with status `DisplayPresenceStatus::Disconnected` rather than being discarded or erroneously transferred to another monitor. When the monitor returns, its deterministic `DisplayId` immediately matches and re-activates the assignment.
4. **Preservation of Missing Assets**: When an assigned wallpaper asset is missing or moved on disk, the assignment record remains intact with status `AssignmentResolutionOutcome::WallpaperUnavailable`, allowing the user to repair or relink the asset without losing their configuration.
5. **Deterministic Precedence**: Explicit per-display assignments take precedence over the global wallpaper preference. If no explicit assignment is set, a display falls back to the global wallpaper. If neither is set, it resolves to `Unassigned`.
6. **Strict Non-Rendering Boundary**: This subsystem does not start player processes, spawn windows, or execute native surface attachments.

---

## 2. Domain Entities & State Types

### 2.1 `DisplayAssignment`
```rust
pub struct DisplayAssignment {
    pub assignment_id: Uuid,
    pub display_id: DisplayId,
    pub wallpaper_id: WallpaperId,
    pub kind: AssignmentKind,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

- **`AssignmentKind::Explicit`**: User explicitly chose this wallpaper for this display.
- **`AssignmentKind::GlobalFallback`**: Inherited from the application-level global wallpaper preference.

### 2.2 Uniqueness Rules
- At most **one** active explicit assignment is stored per `DisplayId`.
- Replacing an assignment for an already-assigned display is atomic and deterministic.
- Assigning the identical wallpaper to an already-assigned display is an idempotent no-op.

---

## 3. Resolution Engine (`AssignmentService`)

The `AssignmentService::resolve_display` operation cross-references:
1. Persisted assignments (`AssignmentStoreData`).
2. The active display topology snapshot (`DisplayTopologySnapshot` from Step 42).
3. The wallpaper catalog (`WallpaperCatalogRepository` from Step 26–30).

### Resolution Outcomes (`AssignmentResolutionOutcome`)
| Outcome | Description | `is_playable` |
|---|---|---|
| `ValidExplicit` | Display is connected; explicit assignment exists; wallpaper is verified Available. | `true` |
| `ValidGlobalFallback` | Display is connected; no explicit assignment; global fallback is Available. | `true` |
| `Unassigned` | Display is connected; no explicit assignment and no global fallback configured. | `false` |
| `WallpaperMissing` | Stored `WallpaperId` does not exist in the catalog. | `false` |
| `WallpaperUnavailable` | Wallpaper exists in catalog, but underlying file is Missing or Inaccessible on disk. | `false` |
| `DisplayDisconnected` | Target display is absent from the active snapshot; saved intent is preserved safely. | `false` |
| `DisplayAmbiguous` | Target display has ambiguous hardware identity matches in current topology. | `false` |
| `IncompatibleContentType` | Wallpaper type or format is unsupported by engine capabilities. | `false` |
| `Conflicting` | Contradictory rules or corrupted store records detected. | `false` |

---

## 4. Atomic Persistence & Schema Evolution

### 4.1 Storage Format (`assignments.json`)
The store is persisted as formatted JSON with atomic write guarantees:
```json
{
  "schema_version": 1,
  "global_wallpaper_id": "019318b8-4c3e-7654-9a8b-123456789abc",
  "assignments": {
    "disp_DISPLAY1_MONITOR_DEL4182_1": {
      "assignment_id": "8889c614-7253-4f13-8954-00004e51caff",
      "display_id": "disp_DISPLAY1_MONITOR_DEL4182_1",
      "wallpaper_id": "019318b8-4c3e-7654-9a8b-123456789abc",
      "kind": "explicit",
      "created_at": "2026-11-15T10:00:00Z",
      "updated_at": "2026-11-15T10:00:00Z"
    }
  },
  "updated_at": "2026-11-15T10:00:00Z"
}
```

### 4.2 Atomic Transaction (`FileAssignmentRepository`)
- Serializes data to a temporary file (`assignments.json.tmp`).
- Flushes and syncs buffers to disk (`file.sync_all()`).
- Replaces the target file atomically via `fs::rename`.
- Guarantees zero partial or corrupted writes on power loss or crash.

---

## 5. Diagnostic History & Observability

`AssignmentService` maintains a bounded diagnostic history ring (`MAX_ASSIGNMENT_DIAGNOSTIC_EVENTS = 100`) recording every assignment mutation, replacement, removal, and global default update with timestamp, display ID, wallpaper ID, and outcome.

---

## 6. Verification Summary

- **Unit Tests (`assignment_tests.rs`)**: 11 tests covering creation, retrieval, idempotency, replacement, removal, global fallback, resolution with available catalog entries, missing wallpaper handling, unavailable asset handling, disconnected display preservation, full display resolution including disconnected saved targets, file persistence round-trip, and diagnostic event ring bounds.
- **Workspace Tests**: All 180 tests pass across all 6 workspace crates with zero regressions.
- **Frontend / UI Build**: `npm test` (21 tests pass) and `npm run verify:build` (clean build verification).

