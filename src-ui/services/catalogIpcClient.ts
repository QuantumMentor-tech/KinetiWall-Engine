/**
 * Typed Frontend-to-Rust Catalog IPC Service Bridge (Steps 24, 25, 26 & 27).
 * Enforces boundary validation, path sanitization, and request sequence monotonicity.
 * Never accesses the filesystem directly or spawns media player processes.
 */

import {
  isValidWallpaperId,
  sanitizeUserFacingErrorMessage,
  type AvailabilityState,
  type CatalogEntrySummary,
  type CatalogIntegrityReport,
  type CatalogLibrarySnapshot,
  type CatalogLoadIssue,
  type EditableMetadataPatch,
  type MetadataUpdateOutcome,
  type RegistrationBatchReport,
  type RelinkOutcome,
  type RemovalOutcome,
  type WallpaperId,
} from '../types/catalog.ts';

export interface CatalogIpcTransport {
  invokeGetLibrarySnapshot(): Promise<unknown>;
  invokeRegisterWallpapers(): Promise<unknown>;
  invokeUpdateMetadata?(id: WallpaperId, patch: EditableMetadataPatch): Promise<unknown>;
  invokeRelinkAsset?(id: WallpaperId): Promise<unknown>;
  invokeRemoveCatalogEntry?(id: WallpaperId): Promise<unknown>;
}

function validateAvailabilityState(raw: unknown): AvailabilityState {
  if (!raw || typeof raw !== 'object') {
    return { status: 'unchecked' };
  }
  const obj = raw as Record<string, unknown>;
  switch (obj.status) {
    case 'available':
      return { status: 'available' };
    case 'missing':
      return { status: 'missing' };
    case 'inaccessible':
      return {
        status: 'inaccessible',
        reason: sanitizeUserFacingErrorMessage(obj.reason ?? 'Asset is locked or inaccessible.'),
      };
    case 'invalid_asset':
      return {
        status: 'invalid_asset',
        reason: sanitizeUserFacingErrorMessage(obj.reason ?? 'Asset failed validation.'),
      };
    case 'unchecked':
    default:
      return { status: 'unchecked' };
  }
}

function validateEntrySummary(raw: unknown): CatalogEntrySummary | null {
  if (!raw || typeof raw !== 'object') {
    return null;
  }
  const obj = raw as Record<string, unknown>;
  if (!isValidWallpaperId(obj.id)) {
    return null;
  }
  if (typeof obj.title !== 'string' || obj.title.trim().length === 0) {
    return null;
  }
  const wallpaperType = typeof obj.wallpaper_type === 'string' ? obj.wallpaper_type : 'unknown';
  const sourceKind =
    obj.source_kind === 'external_direct_file' ? 'external_direct_file' : 'managed_package';
  const author =
    typeof obj.author === 'string' && obj.author.trim().length > 0 ? obj.author.trim() : 'Unknown';
  const description = typeof obj.description === 'string' ? obj.description : '';
  const tags = Array.isArray(obj.tags)
    ? obj.tags.filter((t): t is string => typeof t === 'string' && t.trim().length > 0)
    : [];
  const hasThumbnailRef = Boolean(obj.has_thumbnail_ref);
  const thumbnailRef =
    typeof obj.thumbnail_ref === 'string' && obj.thumbnail_ref.trim().length > 0
      ? obj.thumbnail_ref.trim()
      : null;
  const schemaVersion =
    typeof obj.schema_version === 'number' && Number.isInteger(obj.schema_version) && obj.schema_version > 0
      ? obj.schema_version
      : 1;
  const version =
    typeof obj.version === 'string' && obj.version.trim().length > 0 ? obj.version.trim() : '1.0.0';
  const minEngineVersion =
    typeof obj.min_engine_version === 'string' && obj.min_engine_version.trim().length > 0
      ? obj.min_engine_version.trim()
      : '1.0.0';
  const license =
    typeof obj.license === 'string' && obj.license.trim().length > 0 ? obj.license.trim() : 'Custom';

  return {
    id: obj.id,
    schema_version: schemaVersion,
    title: obj.title,
    description,
    wallpaper_type: wallpaperType,
    source_kind: sourceKind,
    author,
    version,
    min_engine_version: minEngineVersion,
    license,
    tags,
    availability: validateAvailabilityState(obj.availability),
    created_at: typeof obj.created_at === 'string' ? obj.created_at : new Date(0).toISOString(),
    updated_at: typeof obj.updated_at === 'string' ? obj.updated_at : new Date(0).toISOString(),
    is_builtin: Boolean(obj.is_builtin),
    has_thumbnail_ref: hasThumbnailRef,
    thumbnail_ref: thumbnailRef,
  };
}

function computeFallbackIntegrity(
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

export function parseCatalogLibrarySnapshot(raw: unknown): CatalogLibrarySnapshot {
  if (!raw || typeof raw !== 'object') {
    throw new Error('Malformed catalog response from native service.');
  }
  const obj = raw as Record<string, unknown>;
  if (!Array.isArray(obj.entries)) {
    throw new Error('Catalog snapshot is missing an entries array.');
  }

  const seenIds = new Set<string>();
  const entries: CatalogEntrySummary[] = [];
  for (const item of obj.entries) {
    const parsed = validateEntrySummary(item);
    if (parsed && !seenIds.has(parsed.id)) {
      seenIds.add(parsed.id);
      entries.push(parsed);
    }
  }

  const loadIssues: CatalogLoadIssue[] = Array.isArray(obj.load_issues)
    ? obj.load_issues
        .filter((issue): issue is Record<string, unknown> => Boolean(issue && typeof issue === 'object'))
        .map((issue) => ({
          directory_name:
            typeof issue.directory_name === 'string' ? issue.directory_name : 'unknown',
          redacted_path: sanitizeUserFacingErrorMessage(issue.redacted_path ?? '[redacted]'),
          reason: sanitizeUserFacingErrorMessage(issue.reason ?? 'Invalid manifest'),
        }))
    : [];

  const integrity = computeFallbackIntegrity(entries, loadIssues);

  return {
    entries,
    integrity,
    load_issues: loadIssues,
  };
}

export function parseRegistrationBatchReport(raw: unknown): RegistrationBatchReport {
  if (!raw || typeof raw !== 'object') {
    throw new Error('Malformed registration report from native service.');
  }
  const obj = raw as Record<string, unknown>;
  const validStatuses = new Set([
    'cancelled',
    'empty_selection',
    'succeeded',
    'partial_success',
    'all_failed',
  ]);
  if (typeof obj.overall_status !== 'string' || !validStatuses.has(obj.overall_status)) {
    throw new Error('Registration report contains an unrecognized overall_status.');
  }

  return {
    overall_status: obj.overall_status as RegistrationBatchReport['overall_status'],
    total_selected: typeof obj.total_selected === 'number' ? obj.total_selected : 0,
    registered_count: typeof obj.registered_count === 'number' ? obj.registered_count : 0,
    already_present_count:
      typeof obj.already_present_count === 'number' ? obj.already_present_count : 0,
    rejected_count: typeof obj.rejected_count === 'number' ? obj.rejected_count : 0,
    inaccessible_count: typeof obj.inaccessible_count === 'number' ? obj.inaccessible_count : 0,
    failed_count: typeof obj.failed_count === 'number' ? obj.failed_count : 0,
    items: Array.isArray(obj.items) ? (obj.items as RegistrationBatchReport['items']) : [],
  };
}

export function parseMetadataUpdateOutcome(raw: unknown): MetadataUpdateOutcome {
  if (!raw || typeof raw !== 'object') {
    throw new Error('Malformed metadata update outcome from native service.');
  }
  const obj = raw as Record<string, unknown>;
  if (obj.outcome !== 'updated' && obj.outcome !== 'unchanged') {
    throw new Error('Metadata update outcome contains an unrecognized status.');
  }
  const summary = validateEntrySummary(obj.summary);
  if (!summary) {
    throw new Error('Metadata update outcome is missing a valid catalog summary.');
  }
  return {
    outcome: obj.outcome,
    summary,
  };
}

export function parseRelinkOutcome(raw: unknown): RelinkOutcome {
  if (!raw || typeof raw !== 'object') {
    throw new Error('Malformed relink outcome from native service.');
  }
  const obj = raw as Record<string, unknown>;
  if (!isValidWallpaperId(obj.id)) {
    throw new Error('Relink outcome is missing a valid wallpaper identifier.');
  }
  switch (obj.outcome) {
    case 'relinked':
    case 'unchanged_same_asset': {
      const summary = validateEntrySummary(obj.summary);
      if (!summary) {
        throw new Error('Relink outcome is missing a valid catalog summary.');
      }
      return {
        outcome: obj.outcome,
        id: obj.id,
        summary,
      };
    }
    case 'cancelled':
      return {
        outcome: 'cancelled',
        id: obj.id,
      };
    case 'inaccessible_or_missing':
      return {
        outcome: 'inaccessible_or_missing',
        id: obj.id,
        redacted_path: sanitizeUserFacingErrorMessage(obj.redacted_path ?? '[redacted]'),
        availability: validateAvailabilityState(obj.availability),
      };
    case 'rejected':
      return {
        outcome: 'rejected',
        id: obj.id,
        redacted_path: sanitizeUserFacingErrorMessage(obj.redacted_path ?? '[redacted]'),
        reason: (obj.reason as RelinkOutcome & { outcome: 'rejected' }['reason']) ?? {
          kind: 'asset_validation',
          detail: { code: 'validation_failed' },
        },
      };
    default:
      throw new Error('Relink outcome contains an unrecognized status.');
  }
}

export function parseRemovalOutcome(raw: unknown): RemovalOutcome {
  if (!raw || typeof raw !== 'object') {
    throw new Error('Malformed removal outcome from native service.');
  }
  const obj = raw as Record<string, unknown>;
  if (!isValidWallpaperId(obj.id)) {
    throw new Error('Removal outcome is missing a valid wallpaper identifier.');
  }
  switch (obj.outcome) {
    case 'removed':
    case 'already_removed':
    case 'blocked_active_assignment':
    case 'blocked_builtin_protected':
      return {
        outcome: obj.outcome,
        id: obj.id,
      };
    default:
      throw new Error('Removal outcome contains an unrecognized status.');
  }
}

export class CatalogIpcClient {
  private readonly transport: CatalogIpcTransport;

  constructor(transport: CatalogIpcTransport) {
    this.transport = transport;
  }

  async getLibrarySnapshot(): Promise<CatalogLibrarySnapshot> {
    try {
      const raw = await this.transport.invokeGetLibrarySnapshot();
      return parseCatalogLibrarySnapshot(raw);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      throw new Error(sanitizeUserFacingErrorMessage(msg));
    }
  }

  async selectAndRegisterWallpapers(): Promise<RegistrationBatchReport> {
    try {
      const raw = await this.transport.invokeRegisterWallpapers();
      return parseRegistrationBatchReport(raw);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      throw new Error(sanitizeUserFacingErrorMessage(msg));
    }
  }

  async updateWallpaperMetadata(
    id: WallpaperId,
    patch: EditableMetadataPatch,
  ): Promise<MetadataUpdateOutcome> {
    if (!this.transport.invokeUpdateMetadata) {
      throw new Error('Catalog metadata update transport is not configured.');
    }
    try {
      const raw = await this.transport.invokeUpdateMetadata(id, patch);
      return parseMetadataUpdateOutcome(raw);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      throw new Error(sanitizeUserFacingErrorMessage(msg));
    }
  }

  async selectAndRelinkWallpaperAsset(id: WallpaperId): Promise<RelinkOutcome> {
    if (!this.transport.invokeRelinkAsset) {
      throw new Error('Catalog asset relink transport is not configured.');
    }
    try {
      const raw = await this.transport.invokeRelinkAsset(id);
      return parseRelinkOutcome(raw);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      throw new Error(sanitizeUserFacingErrorMessage(msg));
    }
  }

  async removeWallpaperFromCatalog(id: WallpaperId): Promise<RemovalOutcome> {
    if (!this.transport.invokeRemoveCatalogEntry) {
      throw new Error('Catalog entry removal transport is not configured.');
    }
    try {
      const raw = await this.transport.invokeRemoveCatalogEntry(id);
      return parseRemovalOutcome(raw);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      throw new Error(sanitizeUserFacingErrorMessage(msg));
    }
  }
}

