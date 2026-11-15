/**
 * Authoritative TypeScript contracts mirroring `crates/kinetiwall-catalog` wire DTOs (Steps 25–27).
 * Never exposes raw host OS filesystem paths (`C:\...` or `\\server\...`) to presentation components.
 */

export type WallpaperId = string;

export type KnownWallpaperType = 'video' | 'web';

/**
 * Runtime may encounter unknown strings if an untrusted payload is inspected before rejection;
 * UI must represent unknown strings neutrally without claiming playability (Step 27 §7.3).
 */
export type WallpaperTypeWire = KnownWallpaperType | (string & {});

export type WallpaperSourceKind = 'managed_package' | 'external_direct_file';

export type AvailabilityState =
  | { status: 'unchecked' }
  | { status: 'available' }
  | { status: 'missing' }
  | { status: 'inaccessible'; reason: string }
  | { status: 'invalid_asset'; reason: string };

export interface CatalogEntrySummary {
  id: WallpaperId;
  schema_version?: number;
  title: string;
  description?: string;
  wallpaper_type: WallpaperTypeWire;
  source_kind: WallpaperSourceKind;
  author: string;
  version?: string;
  min_engine_version?: string;
  license?: string;
  tags: string[];
  availability: AvailabilityState;
  created_at: string;
  updated_at: string;
  is_builtin: boolean;
  has_thumbnail_ref: boolean;
  thumbnail_ref?: string | null;
}

export interface CatalogLoadIssue {
  directory_name: string;
  redacted_path: string;
  reason: string;
}

export interface CatalogIntegrityReport {
  valid_record_count: number;
  available_count: number;
  missing_count: number;
  inaccessible_count: number;
  invalid_asset_count: number;
  unchecked_count: number;
  load_issue_count: number;
}

export interface CatalogLibrarySnapshot {
  entries: CatalogEntrySummary[];
  integrity: CatalogIntegrityReport;
  load_issues: CatalogLoadIssue[];
}

export type BatchOverallStatus =
  | 'cancelled'
  | 'empty_selection'
  | 'succeeded'
  | 'partial_success'
  | 'all_failed';

export type RegistrationRejectionReason =
  | { code: 'path_safety'; [key: string]: unknown }
  | { code: 'unexpected_directory' }
  | { code: 'unsupported_extension' }
  | { code: 'empty_file' }
  | { code: 'invalid_content_signature'; expected_type: KnownWallpaperType; detail: string }
  | { code: 'validation_failed'; [key: string]: unknown };

export type RegistrationItemOutcome =
  | {
      outcome: 'registered';
      id: WallpaperId;
      summary: CatalogEntrySummary;
    }
  | {
      outcome: 'already_present';
      existing_id: WallpaperId;
      summary: CatalogEntrySummary;
    }
  | {
      outcome: 'rejected';
      redacted_path: string;
      reason: RegistrationRejectionReason;
    }
  | {
      outcome: 'inaccessible_or_missing';
      redacted_path: string;
      availability: AvailabilityState;
    }
  | {
      outcome: 'persistence_failed';
      redacted_path: string;
      detail: string;
    };

export interface RegistrationBatchReport {
  overall_status: BatchOverallStatus;
  total_selected: number;
  registered_count: number;
  already_present_count: number;
  rejected_count: number;
  inaccessible_count: number;
  failed_count: number;
  items: RegistrationItemOutcome[];
}

/**
 * Step 30: Allowlisted user-editable metadata patch.
 * Strictly excludes protected identity, type, source_kind, created_at, and availability fields.
 */
export interface EditableMetadataPatch {
  title: string;
  description: string;
  tags: string[];
  expected_updated_at?: string;
}

export type MetadataUpdateOutcome =
  | {
      outcome: 'updated';
      summary: CatalogEntrySummary;
    }
  | {
      outcome: 'unchanged';
      summary: CatalogEntrySummary;
    };

export type RelinkRejectionReason =
  | {
      kind: 'type_mismatch';
      detail: {
        expected_type: KnownWallpaperType;
        candidate_type: KnownWallpaperType;
      };
    }
  | {
      kind: 'duplicate_owned_by_other';
      detail: {
        existing_id: WallpaperId;
      };
    }
  | {
      kind: 'asset_validation';
      detail: RegistrationRejectionReason;
    };

export type RelinkOutcome =
  | {
      outcome: 'relinked';
      id: WallpaperId;
      summary: CatalogEntrySummary;
    }
  | {
      outcome: 'unchanged_same_asset';
      id: WallpaperId;
      summary: CatalogEntrySummary;
    }
  | {
      outcome: 'cancelled';
      id: WallpaperId;
    }
  | {
      outcome: 'rejected';
      id: WallpaperId;
      redacted_path: string;
      reason: RelinkRejectionReason;
    }
  | {
      outcome: 'inaccessible_or_missing';
      id: WallpaperId;
      redacted_path: string;
      availability: AvailabilityState;
    };

export type RemovalOutcome =
  | {
      outcome: 'removed';
      id: WallpaperId;
    }
  | {
      outcome: 'already_removed';
      id: WallpaperId;
    }
  | {
      outcome: 'blocked_active_assignment';
      id: WallpaperId;
    }
  | {
      outcome: 'blocked_builtin_protected';
      id: WallpaperId;
    };

export const MAX_TITLE_LEN = 120;
export const MAX_DESCRIPTION_LEN = 2000;
export const MAX_TAG_COUNT = 32;
export const MAX_TAG_LEN = 48;

function containsControlCharacters(text: string): boolean {
  for (let i = 0; i < text.length; i += 1) {
    const code = text.charCodeAt(i);
    if ((code >= 0 && code <= 0x1f && code !== 0x09 && code !== 0x0a && code !== 0x0d) || code === 0x7f) {
      return true;
    }
  }
  return false;
}

function containsSingleLineControlCharacters(text: string): boolean {
  for (let i = 0; i < text.length; i += 1) {
    const code = text.charCodeAt(i);
    if ((code >= 0 && code <= 0x1f) || code === 0x7f) {
      return true;
    }
  }
  return false;
}

/**
 * Validates an EditableMetadataPatch on the client before submitting over IPC (`Step 30 §5.2`).
 * Returns a clear user-facing validation error message, or `null` if valid.
 */
export function validateEditableMetadataPatch(patch: EditableMetadataPatch): string | null {
  const trimmedTitle = patch.title.trim();
  if (trimmedTitle.length === 0) {
    return 'Title must not be empty or whitespace-only.';
  }
  if (Array.from(trimmedTitle).length > MAX_TITLE_LEN) {
    return `Title exceeds the maximum length of ${MAX_TITLE_LEN} characters.`;
  }
  if (containsSingleLineControlCharacters(trimmedTitle)) {
    return 'Title contains disallowed control characters.';
  }

  const trimmedDescription = patch.description.trim();
  if (Array.from(trimmedDescription).length > MAX_DESCRIPTION_LEN) {
    return `Description exceeds the maximum length of ${MAX_DESCRIPTION_LEN} characters.`;
  }
  if (containsControlCharacters(trimmedDescription)) {
    return 'Description contains disallowed control characters.';
  }

  if (patch.tags.length > MAX_TAG_COUNT) {
    return `A wallpaper may have at most ${MAX_TAG_COUNT} tags.`;
  }

  for (const tag of patch.tags) {
    const trimmedTag = tag.trim();
    if (trimmedTag.length === 0 || containsSingleLineControlCharacters(tag)) {
      return 'Tags must not be empty or contain control characters.';
    }
    if (Array.from(trimmedTag).length > MAX_TAG_LEN) {
      return `Tag "${trimmedTag}" exceeds the maximum length of ${MAX_TAG_LEN} characters.`;
    }
  }

  return null;
}

/**
 * Normalizes comma-separated or array tags in first-seen case-insensitive order.
 */
export function normalizeEditableTags(rawTags: string[]): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const raw of rawTags) {
    const trimmed = raw.trim();
    if (trimmed.length === 0) {
      continue;
    }
    const key = trimmed.toLowerCase();
    if (!seen.has(key)) {
      seen.add(key);
      result.push(trimmed);
    }
  }
  return result;
}

const UUID_REGEX = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

export function isValidWallpaperId(value: unknown): value is WallpaperId {
  return typeof value === 'string' && UUID_REGEX.test(value);
}

export function isKnownWallpaperType(value: unknown): value is KnownWallpaperType {
  return value === 'video' || value === 'web';
}

/**
 * Validates that a thumbnail relative path is safe (no `..`, drive prefixes, leading slashes, or NUL bytes)
 * and resolves it to the sandboxed Tauri virtual asset protocol URL (`DEC-SEC-05`).
 */
export function resolveSafeThumbnailUrl(
  entry: Pick<CatalogEntrySummary, 'id' | 'has_thumbnail_ref' | 'thumbnail_ref'>,
): string | null {
  if (!entry.has_thumbnail_ref || !entry.thumbnail_ref) {
    return null;
  }
  const ref = entry.thumbnail_ref.trim();
  if (
    ref.length === 0 ||
    ref.includes('\0') ||
    ref.startsWith('/') ||
    ref.startsWith('\\') ||
    /^[a-zA-Z]:/.test(ref)
  ) {
    return null;
  }
  const segments = ref.split(/[\\/]+/);
  if (segments.some((seg) => seg === '' || seg === '.' || seg === '..')) {
    return null;
  }
  const encodedSegments = segments.map((seg) => encodeURIComponent(seg)).join('/');
  return `kinetiwall-asset://localhost/wallpapers/${encodeURIComponent(entry.id)}/${encodedSegments}`;
}

/**
 * Redacts any accidental Windows or POSIX absolute path from error messages before UI presentation.
 */
export function sanitizeUserFacingErrorMessage(rawMessage: unknown): string {
  if (typeof rawMessage !== 'string' || rawMessage.trim().length === 0) {
    return 'Unable to communicate with the local wallpaper catalog service.';
  }
  return rawMessage
    .replace(/[A-Za-z]:\\[^\s"'<>|]+/g, '[redacted-path]')
    .replace(/\\\\[^\s"'<>|]+/g, '[redacted-unc-path]')
    .replace(/\/Users\/[^\s"'<>|]+/g, '[redacted-path]')
    .replace(/\/home\/[^\s"'<>|]+/g, '[redacted-path]')
    .trim();
}

/**
 * Formats an ISO 8601 catalog timestamp deterministically in UTC (`YYYY-MM-DD HH:MM UTC`),
 * or returns `"Timestamp unavailable"` if malformed (`Step 29 §6.6`).
 */
export function formatCatalogTimestampUtc(isoString: unknown): string {
  if (typeof isoString !== 'string' || isoString.trim().length === 0) {
    return 'Timestamp unavailable';
  }
  const timeMs = Date.parse(isoString);
  if (!Number.isFinite(timeMs)) {
    return 'Timestamp unavailable';
  }
  const d = new Date(timeMs);
  const year = d.getUTCFullYear();
  const month = String(d.getUTCMonth() + 1).padStart(2, '0');
  const day = String(d.getUTCDate()).padStart(2, '0');
  const hours = String(d.getUTCHours()).padStart(2, '0');
  const minutes = String(d.getUTCMinutes()).padStart(2, '0');
  return `${year}-${month}-${day} ${hours}:${minutes} UTC`;
}


