/**
 * Authoritative Frontend Catalog Store (Step 24 State Integration & Step 27 Library Browsing).
 * Mirrors `kinetiwall-catalog` records with sequence-guarded async loading, stale-data preservation
 * on refresh failure, and library-only selection semantics.
 */

import { CatalogIpcClient } from '../services/catalogIpcClient.ts';
import {
  normalizeEditableTags,
  sanitizeUserFacingErrorMessage,
  validateEditableMetadataPatch,
  type CatalogEntrySummary,
  type CatalogIntegrityReport,
  type CatalogLoadIssue,
  type EditableMetadataPatch,
  type MetadataUpdateOutcome,
  type RegistrationBatchReport,
  type RelinkOutcome,
  type RelinkRejectionReason,
  type RemovalOutcome,
  type WallpaperId,
} from '../types/catalog.ts';
import {
  applyCatalogQuery,
  DEFAULT_CATALOG_QUERY,
  type CatalogAvailabilityFilter,
  type CatalogQueryOptions,
  type CatalogQueryResult,
  type CatalogSortOrder,
  type CatalogTypeFilter,
} from '../utils/catalogQuery.ts';

export type CatalogLoadStatus = 'idle' | 'loading' | 'ready' | 'error';

export interface CatalogManagementFeedback {
  kind: 'success' | 'info' | 'warning' | 'error';
  message: string;
}

export interface MetadataEditDraft {
  title: string;
  description: string;
  tagsInput: string;
}

export interface CatalogStoreState {
  status: CatalogLoadStatus;
  isRefreshing: boolean;
  isStale: boolean;
  entries: CatalogEntrySummary[];
  integrity: CatalogIntegrityReport;
  loadIssues: CatalogLoadIssue[];
  errorMessage: string | null;
  selectedWallpaperId: WallpaperId | null;
  focusReturnWallpaperId: WallpaperId | null;
  failedThumbnailIds: ReadonlySet<WallpaperId>;
  activeWallpaperIds: ReadonlySet<WallpaperId>;
  editingWallpaperId: WallpaperId | null;
  metadataDraft: MetadataEditDraft | null;
  pendingRemovalWallpaperId: WallpaperId | null;
  managementInFlight: boolean;
  managementFeedback: CatalogManagementFeedback | null;
  registrationInFlight: boolean;
  lastRegistrationReport: RegistrationBatchReport | null;
  registrationErrorMessage: string | null;
  lastCommittedRequestId: number;
  query: CatalogQueryOptions;
}

export type CatalogStoreListener = (state: CatalogStoreState) => void;

const EMPTY_INTEGRITY: CatalogIntegrityReport = {
  valid_record_count: 0,
  available_count: 0,
  missing_count: 0,
  inaccessible_count: 0,
  invalid_asset_count: 0,
  unchecked_count: 0,
  load_issue_count: 0,
};

export class CatalogStore {
  private readonly client: CatalogIpcClient;
  private state: CatalogStoreState;
  private listeners: Set<CatalogStoreListener> = new Set();
  private nextRequestId = 0;
  private activeLoadPromise: Promise<void> | null = null;

  constructor(client: CatalogIpcClient) {
    this.client = client;
    this.state = {
      status: 'idle',
      isRefreshing: false,
      isStale: false,
      entries: [],
      integrity: { ...EMPTY_INTEGRITY },
      loadIssues: [],
      errorMessage: null,
      selectedWallpaperId: null,
      focusReturnWallpaperId: null,
      failedThumbnailIds: new Set(),
      activeWallpaperIds: new Set(),
      editingWallpaperId: null,
      metadataDraft: null,
      pendingRemovalWallpaperId: null,
      managementInFlight: false,
      managementFeedback: null,
      registrationInFlight: false,
      lastRegistrationReport: null,
      registrationErrorMessage: null,
      lastCommittedRequestId: 0,
      query: { ...DEFAULT_CATALOG_QUERY },
    };
  }

  getState(): CatalogStoreState {
    return {
      ...this.state,
      entries: [...this.state.entries],
      integrity: { ...this.state.integrity },
      loadIssues: [...this.state.loadIssues],
      failedThumbnailIds: new Set(this.state.failedThumbnailIds),
      activeWallpaperIds: new Set(this.state.activeWallpaperIds),
      metadataDraft: this.state.metadataDraft ? { ...this.state.metadataDraft } : null,
      managementFeedback: this.state.managementFeedback
        ? { ...this.state.managementFeedback }
        : null,
      query: { ...this.state.query },
    };
  }

  /**
   * Derives the filtered and sorted catalog result collection from authoritative `entries`
   * and current view-state `query` without mutating stored records (Step 28 §9.1, §9.2).
   */
  getQueryResult(): CatalogQueryResult {
    return applyCatalogQuery(this.state.entries, this.state.query);
  }

  setSearchQuery(searchQuery: string): void {
    if (this.state.query.searchQuery === searchQuery) {
      return;
    }
    this.state = {
      ...this.state,
      query: {
        ...this.state.query,
        searchQuery,
      },
    };
    this.emit();
  }

  clearSearchQuery(): void {
    this.setSearchQuery('');
  }

  setTypeFilter(typeFilter: CatalogTypeFilter): void {
    if (this.state.query.typeFilter === typeFilter) {
      return;
    }
    this.state = {
      ...this.state,
      query: {
        ...this.state.query,
        typeFilter,
      },
    };
    this.emit();
  }

  setAvailabilityFilter(availabilityFilter: CatalogAvailabilityFilter): void {
    if (this.state.query.availabilityFilter === availabilityFilter) {
      return;
    }
    this.state = {
      ...this.state,
      query: {
        ...this.state.query,
        availabilityFilter,
      },
    };
    this.emit();
  }

  setSortOrder(sortOrder: CatalogSortOrder): void {
    if (this.state.query.sortOrder === sortOrder) {
      return;
    }
    this.state = {
      ...this.state,
      query: {
        ...this.state.query,
        sortOrder,
      },
    };
    this.emit();
  }

  /**
   * Clears active search text and filters while preserving the user's selected sort order (Step 28 §7.3).
   */
  clearSearchAndFilters(): void {
    const { query } = this.state;
    if (
      query.searchQuery === '' &&
      query.typeFilter === 'all' &&
      query.availabilityFilter === 'all'
    ) {
      return;
    }
    this.state = {
      ...this.state,
      query: {
        ...query,
        searchQuery: '',
        typeFilter: 'all',
        availabilityFilter: 'all',
      },
    };
    this.emit();
  }

  /**
   * Resets search, filters, and sort order back to `DEFAULT_CATALOG_QUERY`.
   */
  resetAllQueryControls(): void {
    this.state = {
      ...this.state,
      query: { ...DEFAULT_CATALOG_QUERY },
    };
    this.emit();
  }

  subscribe(listener: CatalogStoreListener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  /**
   * Loads or refreshes the catalog from the authoritative native service.
   * Deduplicates concurrent initial loads and guards against out-of-order stale responses.
   */
  async loadCatalog(options?: { forceRefresh?: boolean }): Promise<void> {
    const forceRefresh = Boolean(options?.forceRefresh);
    if (this.activeLoadPromise && !forceRefresh) {
      return this.activeLoadPromise;
    }

    const requestId = ++this.nextRequestId;
    const hasExistingRecords = this.state.entries.length > 0;

    this.state = {
      ...this.state,
      status: hasExistingRecords ? 'ready' : 'loading',
      isRefreshing: hasExistingRecords,
      errorMessage: null,
    };
    this.emit();

    const task = (async () => {
      try {
        const snapshot = await this.client.getLibrarySnapshot();
        // Drop stale out-of-order responses (Step 24 Principle D)
        if (requestId < this.state.lastCommittedRequestId) {
          return;
        }

        // Preserve current selection only if its stable ID still exists in the refreshed catalog
        const currentSelected = this.state.selectedWallpaperId;
        const selectionStillExists =
          currentSelected !== null &&
          snapshot.entries.some((entry) => entry.id === currentSelected);

        this.state = {
          ...this.state,
          status: 'ready',
          isRefreshing: false,
          isStale: false,
          entries: snapshot.entries,
          integrity: snapshot.integrity,
          loadIssues: snapshot.load_issues,
          errorMessage: null,
          selectedWallpaperId: selectionStillExists ? currentSelected : null,
          lastCommittedRequestId: requestId,
        };
        this.emit();
      } catch (err) {
        if (requestId < this.state.lastCommittedRequestId) {
          return;
        }
        const message = sanitizeUserFacingErrorMessage(
          err instanceof Error ? err.message : String(err),
        );
        // Step 27 §8.4: Do not erase previously valid catalog data on refresh failure; mark stale instead.
        const preserveExisting = this.state.entries.length > 0;
        this.state = {
          ...this.state,
          status: preserveExisting ? 'ready' : 'error',
          isRefreshing: false,
          isStale: preserveExisting,
          errorMessage: message,
          lastCommittedRequestId: requestId,
        };
        this.emit();
      } finally {
        if (this.activeLoadPromise === task) {
          this.activeLoadPromise = null;
        }
      }
    })();

    this.activeLoadPromise = task;
    return task;
  }

  /**
   * Selects or deselects a catalog record for visual inspection in the library.
   * Strictly a UI-level operation: never triggers wallpaper application or playback (Step 27 §9.1, Step 29 §9.1).
   */
  selectWallpaper(id: WallpaperId | null): void {
    if (id === null) {
      if (this.state.selectedWallpaperId !== null) {
        const previousId = this.state.selectedWallpaperId;
        this.state = {
          ...this.state,
          selectedWallpaperId: null,
          focusReturnWallpaperId: previousId,
          editingWallpaperId: null,
          metadataDraft: null,
          pendingRemovalWallpaperId: null,
        };
        this.emit();
      }
      return;
    }
    const exists = this.state.entries.some((entry) => entry.id === id);
    if (!exists) {
      return;
    }
    const isDeselecting = this.state.selectedWallpaperId === id;
    this.state = {
      ...this.state,
      selectedWallpaperId: isDeselecting ? null : id,
      focusReturnWallpaperId: isDeselecting ? id : null,
      editingWallpaperId: null,
      metadataDraft: null,
      pendingRemovalWallpaperId: null,
    };
    this.emit();
  }

  /**
   * Closes the details panel and restores keyboard focus target to the previously selected card (Step 29 §9.2).
   */
  closeDetailsPanel(): void {
    this.selectWallpaper(null);
  }

  getSelectedWallpaper(): CatalogEntrySummary | null {
    if (!this.state.selectedWallpaperId) {
      return null;
    }
    return (
      this.state.entries.find((entry) => entry.id === this.state.selectedWallpaperId) ?? null
    );
  }

  /**
   * Updates the set of currently active/assigned wallpaper IDs so removal safeguards
   * can block deletion of in-use catalog records (`Step 30 §7.2`).
   */
  setActiveWallpaperIds(ids: Iterable<WallpaperId>): void {
    this.state = {
      ...this.state,
      activeWallpaperIds: new Set(ids),
    };
    this.emit();
  }

  isWallpaperActive(id: WallpaperId): boolean {
    return this.state.activeWallpaperIds.has(id);
  }

  startMetadataEdit(id: WallpaperId): void {
    if (this.state.managementInFlight) {
      return;
    }
    const existing = this.state.entries.find((entry) => entry.id === id);
    if (!existing) {
      return;
    }
    if (existing.is_builtin) {
      this.state = {
        ...this.state,
        managementFeedback: {
          kind: 'warning',
          message: 'Built-in wallpapers have read-only metadata.',
        },
      };
      this.emit();
      return;
    }
    this.state = {
      ...this.state,
      selectedWallpaperId: id,
      editingWallpaperId: id,
      metadataDraft: {
        title: existing.title,
        description: existing.description ?? '',
        tagsInput: existing.tags.join(', '),
      },
      pendingRemovalWallpaperId: null,
      managementFeedback: null,
    };
    this.emit();
  }

  updateMetadataDraft(partial: Partial<MetadataEditDraft>): void {
    if (!this.state.metadataDraft || !this.state.editingWallpaperId) {
      return;
    }
    this.state = {
      ...this.state,
      metadataDraft: {
        ...this.state.metadataDraft,
        ...partial,
      },
    };
    this.emit();
  }

  async submitMetadataEdit(): Promise<MetadataUpdateOutcome | null> {
    if (!this.state.editingWallpaperId || !this.state.metadataDraft) {
      return null;
    }
    const rawTags = this.state.metadataDraft.tagsInput
      .split(',')
      .map((t) => t.trim())
      .filter((t) => t.length > 0);
    return this.updateWallpaperMetadata(this.state.editingWallpaperId, {
      title: this.state.metadataDraft.title,
      description: this.state.metadataDraft.description,
      tags: rawTags,
    });
  }

  cancelMetadataEdit(): void {
    if (this.state.editingWallpaperId === null) {
      return;
    }
    this.state = {
      ...this.state,
      editingWallpaperId: null,
      metadataDraft: null,
      managementFeedback: null,
    };
    this.emit();
  }

  dismissManagementFeedback(): void {
    if (!this.state.managementFeedback) {
      return;
    }
    this.state = {
      ...this.state,
      managementFeedback: null,
    };
    this.emit();
  }

  /**
   * Updates allowlisted editable metadata (`title`, `description`, `tags`) for a catalog entry (`Step 30 §5`).
   */
  async updateWallpaperMetadata(
    id: WallpaperId,
    rawPatch: EditableMetadataPatch,
  ): Promise<MetadataUpdateOutcome | null> {
    if (this.state.managementInFlight) {
      return null;
    }
    const existing = this.state.entries.find((entry) => entry.id === id);
    if (!existing) {
      return null;
    }

    const validationError = validateEditableMetadataPatch(rawPatch);
    if (validationError) {
      this.state = {
        ...this.state,
        managementFeedback: {
          kind: 'error',
          message: validationError,
        },
      };
      this.emit();
      return null;
    }

    const normalizedTitle = rawPatch.title.trim();
    const normalizedDescription = rawPatch.description.trim();
    const normalizedTags = normalizeEditableTags(rawPatch.tags);

    const existingTagsNormalized = normalizeEditableTags(existing.tags);
    const isUnchanged =
      existing.title === normalizedTitle &&
      (existing.description ?? '') === normalizedDescription &&
      existingTagsNormalized.length === normalizedTags.length &&
      existingTagsNormalized.every((tag, idx) => tag === normalizedTags[idx]);

    if (isUnchanged) {
      this.state = {
        ...this.state,
        editingWallpaperId: null,
        metadataDraft: null,
        managementFeedback: {
          kind: 'info',
          message: 'No metadata changes to save.',
        },
      };
      this.emit();
      return {
        outcome: 'unchanged',
        summary: existing,
      };
    }

    const patch: EditableMetadataPatch = {
      title: normalizedTitle,
      description: normalizedDescription,
      tags: normalizedTags,
      expected_updated_at: rawPatch.expected_updated_at ?? existing.updated_at,
    };

    this.state = {
      ...this.state,
      managementInFlight: true,
      managementFeedback: null,
    };
    this.emit();

    try {
      const outcome = await this.client.updateWallpaperMetadata(id, patch);
      const nextEntries = this.state.entries.map((entry) =>
        entry.id === outcome.summary.id ? outcome.summary : entry,
      );
      this.state = {
        ...this.state,
        entries: nextEntries,
        integrity: this.recomputeIntegrity(nextEntries, this.state.loadIssues),
        editingWallpaperId: null,
        metadataDraft: null,
        managementInFlight: false,
        managementFeedback: {
          kind: outcome.outcome === 'updated' ? 'success' : 'info',
          message:
            outcome.outcome === 'updated'
              ? `Saved metadata changes for "${outcome.summary.title}".`
              : 'No metadata changes to save.',
        },
      };
      this.emit();
      return outcome;
    } catch (err) {
      const message = sanitizeUserFacingErrorMessage(
        err instanceof Error ? err.message : String(err),
      );
      this.state = {
        ...this.state,
        managementInFlight: false,
        managementFeedback: {
          kind: 'error',
          message,
        },
      };
      this.emit();
      // Step 30 §8.3: Reconcile store state against authoritative catalog after ambiguous failure
      await this.loadCatalog({ forceRefresh: true });
      return null;
    }
  }

  /**
   * Initiates replacement file selection and relinks `id` after validation (`Step 30 §6`).
   */
  async relinkWallpaperAsset(id: WallpaperId): Promise<RelinkOutcome | null> {
    if (this.state.managementInFlight) {
      return null;
    }
    const existing = this.state.entries.find((entry) => entry.id === id);
    if (!existing) {
      return null;
    }

    this.state = {
      ...this.state,
      managementInFlight: true,
      managementFeedback: null,
    };
    this.emit();

    try {
      const outcome = await this.client.selectAndRelinkWallpaperAsset(id);
      switch (outcome.outcome) {
        case 'cancelled': {
          this.state = {
            ...this.state,
            managementInFlight: false,
          };
          this.emit();
          return outcome;
        }
        case 'unchanged_same_asset': {
          const nextEntries = this.state.entries.map((entry) =>
            entry.id === outcome.summary.id ? outcome.summary : entry,
          );
          this.state = {
            ...this.state,
            entries: nextEntries,
            integrity: this.recomputeIntegrity(nextEntries, this.state.loadIssues),
            managementInFlight: false,
            managementFeedback: {
              kind: 'info',
              message: 'Selected file is already linked and verified for this wallpaper.',
            },
          };
          this.emit();
          return outcome;
        }
        case 'relinked': {
          const nextEntries = this.state.entries.map((entry) =>
            entry.id === outcome.summary.id ? outcome.summary : entry,
          );
          this.state = {
            ...this.state,
            entries: nextEntries,
            integrity: this.recomputeIntegrity(nextEntries, this.state.loadIssues),
            managementInFlight: false,
            managementFeedback: {
              kind: 'success',
              message: 'Replacement asset verified and relinked. Status is now Available.',
            },
          };
          this.emit();
          return outcome;
        }
        case 'inaccessible_or_missing': {
          const detail =
            outcome.availability.status === 'inaccessible'
              ? outcome.availability.reason
              : 'Selected file could not be found on disk.';
          this.state = {
            ...this.state,
            managementInFlight: false,
            managementFeedback: {
              kind: 'error',
              message: `Could not relink asset (${outcome.redacted_path}): ${detail} Existing catalog link was preserved.`,
            },
          };
          this.emit();
          return outcome;
        }
        case 'rejected': {
          this.state = {
            ...this.state,
            managementInFlight: false,
            managementFeedback: {
              kind: 'error',
              message: this.formatRelinkRejectionMessage(outcome.redacted_path, outcome.reason),
            },
          };
          this.emit();
          return outcome;
        }
      }
    } catch (err) {
      const message = sanitizeUserFacingErrorMessage(
        err instanceof Error ? err.message : String(err),
      );
      this.state = {
        ...this.state,
        managementInFlight: false,
        managementFeedback: {
          kind: 'error',
          message,
        },
      };
      this.emit();
      await this.loadCatalog({ forceRefresh: true });
      return null;
    }
  }

  /**
   * Opens the catalog-only removal confirmation dialog, or blocks immediately if the entry is
   * actively assigned or a protected built-in entry (`Step 30 §7.2, §7.3`).
   */
  requestRemoveWallpaper(id: WallpaperId): void {
    if (this.state.managementInFlight) {
      return;
    }
    const existing = this.state.entries.find((entry) => entry.id === id);
    if (!existing) {
      return;
    }
    if (existing.is_builtin) {
      this.state = {
        ...this.state,
        pendingRemovalWallpaperId: null,
        managementFeedback: {
          kind: 'warning',
          message: 'Built-in wallpapers are protected and cannot be removed from the library.',
        },
      };
      this.emit();
      return;
    }
    if (this.state.activeWallpaperIds.has(id)) {
      this.state = {
        ...this.state,
        pendingRemovalWallpaperId: null,
        managementFeedback: {
          kind: 'warning',
          message:
            'Cannot remove this wallpaper while it is actively assigned to a monitor. Stop or unassign it first.',
        },
      };
      this.emit();
      return;
    }

    this.state = {
      ...this.state,
      editingWallpaperId: null,
      pendingRemovalWallpaperId: id,
      managementFeedback: null,
    };
    this.emit();
  }

  cancelRemoveWallpaper(): void {
    if (this.state.pendingRemovalWallpaperId === null) {
      return;
    }
    this.state = {
      ...this.state,
      pendingRemovalWallpaperId: null,
    };
    this.emit();
  }

  /**
   * Executes confirmed catalog-only removal (`Step 30 §7`).
   * Deletes only the catalog record while strictly preserving original media files on disk.
   */
  async confirmRemoveWallpaper(explicitId?: WallpaperId): Promise<RemovalOutcome | null> {
    if (this.state.managementInFlight) {
      return null;
    }
    const targetId = explicitId ?? this.state.pendingRemovalWallpaperId;
    if (!targetId) {
      return null;
    }

    const existing = this.state.entries.find((entry) => entry.id === targetId);
    if (existing?.is_builtin) {
      this.state = {
        ...this.state,
        pendingRemovalWallpaperId: null,
        managementFeedback: {
          kind: 'warning',
          message: 'Built-in wallpapers are protected and cannot be removed from the library.',
        },
      };
      this.emit();
      return { outcome: 'blocked_builtin_protected', id: targetId };
    }

    if (this.state.activeWallpaperIds.has(targetId)) {
      this.state = {
        ...this.state,
        pendingRemovalWallpaperId: null,
        managementFeedback: {
          kind: 'warning',
          message:
            'Cannot remove this wallpaper while it is actively assigned to a monitor. Stop or unassign it first.',
        },
      };
      this.emit();
      return { outcome: 'blocked_active_assignment', id: targetId };
    }

    this.state = {
      ...this.state,
      managementInFlight: true,
      managementFeedback: null,
    };
    this.emit();

    try {
      const outcome = await this.client.removeWallpaperFromCatalog(targetId);
      if (outcome.outcome === 'blocked_active_assignment') {
        this.state = {
          ...this.state,
          pendingRemovalWallpaperId: null,
          managementInFlight: false,
          managementFeedback: {
            kind: 'warning',
            message:
              'Cannot remove this wallpaper while it is actively assigned to a monitor. Stop or unassign it first.',
          },
        };
        this.emit();
        return outcome;
      }
      if (outcome.outcome === 'blocked_builtin_protected') {
        this.state = {
          ...this.state,
          pendingRemovalWallpaperId: null,
          managementInFlight: false,
          managementFeedback: {
            kind: 'warning',
            message: 'Built-in wallpapers are protected and cannot be removed from the library.',
          },
        };
        this.emit();
        return outcome;
      }

      // Compute next keyboard focus target among visible query results before removing targetId
      const visibleIds = this.getQueryResult().visibleEntries.map((item) => item.id);
      const removedIndex = visibleIds.indexOf(targetId);
      const nextFocusId =
        removedIndex >= 0
          ? visibleIds[removedIndex + 1] ?? visibleIds[removedIndex - 1] ?? null
          : null;

      const nextEntries = this.state.entries.filter((entry) => entry.id !== targetId);
      const nextFailedThumbnails = new Set(this.state.failedThumbnailIds);
      nextFailedThumbnails.delete(targetId);

      this.state = {
        ...this.state,
        entries: nextEntries,
        integrity: this.recomputeIntegrity(nextEntries, this.state.loadIssues),
        selectedWallpaperId:
          this.state.selectedWallpaperId === targetId ? null : this.state.selectedWallpaperId,
        focusReturnWallpaperId: nextFocusId,
        editingWallpaperId:
          this.state.editingWallpaperId === targetId ? null : this.state.editingWallpaperId,
        pendingRemovalWallpaperId: null,
        failedThumbnailIds: nextFailedThumbnails,
        managementInFlight: false,
        managementFeedback: {
          kind: 'success',
          message:
            'Removed wallpaper from the KinetiWall library. Original media files on disk were preserved.',
        },
      };
      this.emit();
      return outcome;
    } catch (err) {
      const message = sanitizeUserFacingErrorMessage(
        err instanceof Error ? err.message : String(err),
      );
      this.state = {
        ...this.state,
        pendingRemovalWallpaperId: null,
        managementInFlight: false,
        managementFeedback: {
          kind: 'error',
          message,
        },
      };
      this.emit();
      await this.loadCatalog({ forceRefresh: true });
      return null;
    }
  }

  /**
   * Records that a card's thumbnail image failed to load so it safely falls back to the placeholder SVG.
   */
  markThumbnailFailed(id: WallpaperId): void {
    if (this.state.failedThumbnailIds.has(id)) {
      return;
    }
    const nextSet = new Set(this.state.failedThumbnailIds);
    nextSet.add(id);
    this.state = {
      ...this.state,
      failedThumbnailIds: nextSet,
    };
    this.emit();
  }

  /**
   * Invokes the Step 26 local wallpaper registration workflow and refreshes the catalog
   * only after persistence is confirmed (Step 27 §8.5, §9.4).
   */
  async triggerRegistration(): Promise<RegistrationBatchReport | null> {
    if (this.state.registrationInFlight) {
      return null;
    }

    this.state = {
      ...this.state,
      registrationInFlight: true,
      registrationErrorMessage: null,
    };
    this.emit();

    try {
      const report = await this.client.selectAndRegisterWallpapers();
      this.state = {
        ...this.state,
        registrationInFlight: false,
        lastRegistrationReport:
          report.overall_status === 'cancelled' || report.overall_status === 'empty_selection'
            ? this.state.lastRegistrationReport
            : report,
        registrationErrorMessage: null,
      };
      this.emit();

      // Only refresh the catalog when records may have been registered or updated
      if (
        report.overall_status === 'succeeded' ||
        report.overall_status === 'partial_success'
      ) {
        await this.loadCatalog({ forceRefresh: true });
      }
      return report;
    } catch (err) {
      const message = sanitizeUserFacingErrorMessage(
        err instanceof Error ? err.message : String(err),
      );
      this.state = {
        ...this.state,
        registrationInFlight: false,
        registrationErrorMessage: message,
      };
      this.emit();
      return null;
    }
  }

  dismissRegistrationReport(): void {
    if (!this.state.lastRegistrationReport && !this.state.registrationErrorMessage) {
      return;
    }
    this.state = {
      ...this.state,
      lastRegistrationReport: null,
      registrationErrorMessage: null,
    };
    this.emit();
  }

  private recomputeIntegrity(
    entries: CatalogEntrySummary[],
    loadIssues: CatalogLoadIssue[],
  ): CatalogIntegrityReport {
    let availableCount = 0;
    let missingCount = 0;
    let inaccessibleCount = 0;
    let invalidAssetCount = 0;
    let uncheckedCount = 0;

    for (const entry of entries) {
      switch (entry.availability.status) {
        case 'available':
          availableCount += 1;
          break;
        case 'missing':
          missingCount += 1;
          break;
        case 'inaccessible':
          inaccessibleCount += 1;
          break;
        case 'invalid_asset':
          invalidAssetCount += 1;
          break;
        case 'unchecked':
          uncheckedCount += 1;
          break;
      }
    }

    return {
      valid_record_count: entries.length,
      available_count: availableCount,
      missing_count: missingCount,
      inaccessible_count: inaccessibleCount,
      invalid_asset_count: invalidAssetCount,
      unchecked_count: uncheckedCount,
      load_issue_count: loadIssues.length,
    };
  }

  private formatRelinkRejectionMessage(
    redactedPath: string,
    reason: RelinkRejectionReason,
  ): string {
    switch (reason.kind) {
      case 'type_mismatch':
        return `Cannot relink (${redactedPath}): expected a ${reason.detail.expected_type.toUpperCase()} asset, but selected a ${reason.detail.candidate_type.toUpperCase()} file. Existing link preserved.`;
      case 'duplicate_owned_by_other':
        return `Cannot relink (${redactedPath}): this file is already registered to another wallpaper (${reason.detail.existing_id}).`;
      case 'asset_validation': {
        const code = reason.detail.code;
        if (code === 'invalid_content_signature') {
          return `Cannot relink (${redactedPath}): file header signature is invalid (${reason.detail.detail}).`;
        }
        return `Cannot relink (${redactedPath}): asset failed validation (${code}).`;
      }
    }
  }

  private emit(): void {
    const snapshot = this.getState();
    for (const listener of this.listeners) {
      listener(snapshot);
    }
  }
}
