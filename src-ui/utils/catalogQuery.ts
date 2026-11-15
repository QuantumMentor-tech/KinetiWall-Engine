/**
 * Pure, Deterministic Local Catalog Search, Filtering & Sorting Engine (`Step 28`).
 *
 * Operates strictly on in-memory `CatalogEntrySummary` DTOs without mutating records,
 * touching the filesystem, or invoking media players.
 */

import { isKnownWallpaperType, type CatalogEntrySummary } from '../types/catalog.ts';

export type CatalogTypeFilter = 'all' | 'video' | 'web';

export type CatalogAvailabilityFilter = 'all' | 'available' | 'unavailable' | 'unchecked';

export type CatalogSortOrder = 'oldest_asc' | 'recent_desc' | 'title_asc' | 'title_desc';

export interface CatalogQueryOptions {
  searchQuery: string;
  typeFilter: CatalogTypeFilter;
  availabilityFilter: CatalogAvailabilityFilter;
  sortOrder: CatalogSortOrder;
}

export const DEFAULT_CATALOG_QUERY: Readonly<CatalogQueryOptions> = {
  searchQuery: '',
  typeFilter: 'all',
  availabilityFilter: 'all',
  sortOrder: 'oldest_asc',
} as const;

export interface CatalogQueryResult {
  visibleEntries: CatalogEntrySummary[];
  totalCatalogCount: number;
  matchingCount: number;
  normalizedSearchQuery: string;
  hasActiveSearch: boolean;
  hasActiveFilters: boolean;
  isRestricted: boolean;
}

/**
 * Normalizes user search input by trimming leading/trailing whitespace and folding case.
 * Never strips meaningful punctuation or alters stored catalog strings (Step 28 §5.4).
 */
export function normalizeSearchQuery(rawQuery: string): string {
  if (typeof rawQuery !== 'string') {
    return '';
  }
  return rawQuery.trim().toLowerCase();
}

export function isQueryRestricted(query: CatalogQueryOptions): boolean {
  return (
    normalizeSearchQuery(query.searchQuery).length > 0 ||
    query.typeFilter !== 'all' ||
    query.availabilityFilter !== 'all'
  );
}

/**
 * Matches a normalized query against verified searchable text metadata (`title`, `author`, `description`, `tags`).
 * Never searches raw host OS paths or internal UUID identifiers (Step 28 §5.2, §5.3).
 */
export function matchesSearchQuery(
  entry: CatalogEntrySummary,
  normalizedQuery: string,
): boolean {
  if (normalizedQuery.length === 0) {
    return true;
  }

  const title = typeof entry.title === 'string' ? entry.title.toLowerCase() : '';
  if (title.includes(normalizedQuery)) {
    return true;
  }

  const author = typeof entry.author === 'string' ? entry.author.toLowerCase() : '';
  if (author.includes(normalizedQuery)) {
    return true;
  }

  const description = typeof entry.description === 'string' ? entry.description.toLowerCase() : '';
  if (description.includes(normalizedQuery)) {
    return true;
  }

  if (Array.isArray(entry.tags)) {
    for (const tag of entry.tags) {
      if (typeof tag === 'string' && tag.toLowerCase().includes(normalizedQuery)) {
        return true;
      }
    }
  }

  return false;
}

/**
 * Matches an entry against the selected `CatalogTypeFilter` (`'all' | 'video' | 'web'`).
 * Unknown/unsupported types never match `'video'` or `'web'` (Step 28 §6.1).
 */
export function matchesTypeFilter(
  entry: CatalogEntrySummary,
  typeFilter: CatalogTypeFilter,
): boolean {
  if (typeFilter === 'all') {
    return true;
  }
  if (!isKnownWallpaperType(entry.wallpaper_type)) {
    return false;
  }
  return entry.wallpaper_type === typeFilter;
}

/**
 * Matches an entry against the selected `CatalogAvailabilityFilter`.
 * Distinguishes verified `'available'`, verified `'unavailable'` (`missing | inaccessible | invalid_asset`),
 * and `'unchecked'` (`Unverified`) states honestly (Step 28 §6.2).
 */
export function matchesAvailabilityFilter(
  entry: CatalogEntrySummary,
  availabilityFilter: CatalogAvailabilityFilter,
): boolean {
  if (availabilityFilter === 'all') {
    return true;
  }
  const status = entry.availability?.status ?? 'unchecked';
  if (availabilityFilter === 'available') {
    return status === 'available';
  }
  if (availabilityFilter === 'unchecked') {
    return status === 'unchecked';
  }
  // 'unavailable'
  return status === 'missing' || status === 'inaccessible' || status === 'invalid_asset';
}

function parseTimestampMs(isoString: string): number {
  if (typeof isoString !== 'string' || isoString.length === 0) {
    return 0;
  }
  const parsed = Date.parse(isoString);
  return Number.isFinite(parsed) ? parsed : 0;
}

function compareCodePointStrings(a: string, b: string): number {
  if (a < b) {
    return -1;
  }
  if (a > b) {
    return 1;
  }
  return 0;
}

/**
 * Deterministic comparator with stable `WallpaperId` tie-breaker (Step 28 §7.1, §7.2).
 */
export function compareCatalogEntries(
  a: CatalogEntrySummary,
  b: CatalogEntrySummary,
  sortOrder: CatalogSortOrder,
): number {
  const titleA = typeof a.title === 'string' ? a.title : '';
  const titleB = typeof b.title === 'string' ? b.title : '';

  switch (sortOrder) {
    case 'title_asc': {
      const lowerCmp = compareCodePointStrings(titleA.toLowerCase(), titleB.toLowerCase());
      if (lowerCmp !== 0) {
        return lowerCmp;
      }
      const exactCmp = compareCodePointStrings(titleA, titleB);
      if (exactCmp !== 0) {
        return exactCmp;
      }
      return compareCodePointStrings(a.id, b.id);
    }
    case 'title_desc': {
      const lowerCmp = compareCodePointStrings(titleB.toLowerCase(), titleA.toLowerCase());
      if (lowerCmp !== 0) {
        return lowerCmp;
      }
      const exactCmp = compareCodePointStrings(titleB, titleA);
      if (exactCmp !== 0) {
        return exactCmp;
      }
      return compareCodePointStrings(a.id, b.id);
    }
    case 'recent_desc': {
      const timeDiff = parseTimestampMs(b.created_at) - parseTimestampMs(a.created_at);
      if (timeDiff !== 0) {
        return timeDiff;
      }
      return compareCodePointStrings(a.id, b.id);
    }
    case 'oldest_asc':
    default: {
      const timeDiff = parseTimestampMs(a.created_at) - parseTimestampMs(b.created_at);
      if (timeDiff !== 0) {
        return timeDiff;
      }
      return compareCodePointStrings(a.id, b.id);
    }
  }
}

/**
 * Applies search, intersection filtering, and deterministic sorting over a catalog entry list
 * without mutating the source array or its records (Step 28 §6.3, §9.1, §9.2).
 */
export function applyCatalogQuery(
  entries: readonly CatalogEntrySummary[],
  query: CatalogQueryOptions,
): CatalogQueryResult {
  const normalizedSearchQuery = normalizeSearchQuery(query.searchQuery);
  const hasActiveSearch = normalizedSearchQuery.length > 0;
  const hasActiveFilters = query.typeFilter !== 'all' || query.availabilityFilter !== 'all';

  const filtered = entries.filter(
    (entry) =>
      matchesSearchQuery(entry, normalizedSearchQuery) &&
      matchesTypeFilter(entry, query.typeFilter) &&
      matchesAvailabilityFilter(entry, query.availabilityFilter),
  );

  const visibleEntries = [...filtered].sort((a, b) =>
    compareCatalogEntries(a, b, query.sortOrder),
  );

  return {
    visibleEntries,
    totalCatalogCount: entries.length,
    matchingCount: visibleEntries.length,
    normalizedSearchQuery,
    hasActiveSearch,
    hasActiveFilters,
    isRestricted: hasActiveSearch || hasActiveFilters,
  };
}

