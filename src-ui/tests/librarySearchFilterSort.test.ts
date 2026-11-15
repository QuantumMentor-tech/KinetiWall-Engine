/**
 * Step 28 Automated Verification Suite (`AC-01` through `AC-15`):
 * Wallpaper Library Search, Filtering & Sorting.
 */

import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { AppShellController } from '../components/shell/AppShell.ts';
import {
  activateByKeyboard,
  changeNodeValue,
  clickNode,
  findAllByTestIdPrefix,
  findNodeByTestId,
  getTextContent,
} from '../components/primitives/uiNode.ts';
import { CatalogIpcClient } from '../services/catalogIpcClient.ts';
import { CatalogStore } from '../stores/catalogStore.ts';
import { RouterStore } from '../stores/routerStore.ts';
import type { CatalogEntrySummary, CatalogLibrarySnapshot } from '../types/catalog.ts';

const ID_A = '11111111-1111-4111-8111-111111111111';
const ID_B = '22222222-2222-4222-8222-222222222222';
const ID_C = '33333333-3333-4333-8333-333333333333';
const ID_D = '44444444-4444-4444-8444-444444444444';
const ID_E = '55555555-5555-4555-8555-555555555555';

function makeEntry(overrides: Partial<CatalogEntrySummary> & { id: string }): CatalogEntrySummary {
  return {
    id: overrides.id,
    title: overrides.title ?? 'Untitled Wallpaper',
    description: overrides.description ?? '',
    wallpaper_type: overrides.wallpaper_type ?? 'video',
    source_kind: overrides.source_kind ?? 'managed_package',
    author: overrides.author ?? 'Unknown',
    tags: overrides.tags ?? [],
    availability: overrides.availability ?? { status: 'available' },
    created_at: overrides.created_at ?? '2026-11-12T00:00:00Z',
    updated_at: overrides.updated_at ?? '2026-11-12T00:00:00Z',
    is_builtin: overrides.is_builtin ?? false,
    has_thumbnail_ref: overrides.has_thumbnail_ref ?? false,
    thumbnail_ref: overrides.thumbnail_ref ?? null,
  };
}

function makeSnapshot(entries: CatalogEntrySummary[]): CatalogLibrarySnapshot {
  return {
    entries,
    integrity: {
      valid_record_count: entries.length,
      available_count: entries.filter((e) => e.availability.status === 'available').length,
      missing_count: entries.filter((e) => e.availability.status === 'missing').length,
      inaccessible_count: entries.filter((e) => e.availability.status === 'inaccessible').length,
      invalid_asset_count: entries.filter((e) => e.availability.status === 'invalid_asset').length,
      unchecked_count: entries.filter((e) => e.availability.status === 'unchecked').length,
      load_issue_count: 0,
    },
    load_issues: [],
  };
}

function getRenderedCardIds(tree: ReturnType<AppShellController['render']>): string[] {
  const grid = findNodeByTestId(tree, 'library-catalog-grid');
  if (!grid) {
    return [];
  }
  return findAllByTestIdPrefix(grid, 'wallpaper-card-')
    .filter((node) => node.tag === 'article')
    .map((node) => String(node.attrs['data-wallpaper-id']));
}

describe('Step 28: Wallpaper Library Search, Filtering & Sorting', () => {
  const sampleCatalog: CatalogEntrySummary[] = [
    makeEntry({
      id: ID_A,
      title: 'Cyber-Grid: [4K] (Loop #1)',
      description: 'High-contrast synthwave matrix',
      author: 'Nova Studio',
      wallpaper_type: 'video',
      availability: { status: 'available' },
      tags: ['synthwave', '60fps'],
      created_at: '2026-11-10T10:00:00Z',
    }),
    makeEntry({
      id: ID_B,
      title: '東京 ネオン Horizon',
      description: 'Interactive rain reflections',
      author: 'Akira',
      wallpaper_type: 'web',
      availability: { status: 'available' },
      tags: ['tokyo', 'rain'],
      created_at: '2026-11-12T12:00:00Z',
    }),
    makeEntry({
      id: ID_C,
      title: 'Aurora Borealis',
      description: '',
      author: 'Nordic Visuals',
      wallpaper_type: 'video',
      availability: { status: 'missing' },
      tags: [],
      created_at: '2026-11-11T08:00:00Z',
    }),
    makeEntry({
      id: ID_D,
      title: 'aurora borealis', // same case-folded title as ID_C, same timestamp as ID_C to test deterministic tie-breaker
      description: 'Unchecked secondary copy',
      author: 'Nordic Visuals',
      wallpaper_type: 'web',
      availability: { status: 'unchecked' },
      tags: ['polar'],
      created_at: '2026-11-11T08:00:00Z',
    }),
    makeEntry({
      id: ID_E,
      title: 'Experimental Shader Pack',
      description: 'Locked file test',
      author: 'Lab',
      wallpaper_type: 'unknown_future_type',
      availability: { status: 'inaccessible', reason: 'Locked by another process' },
      tags: ['experimental'],
      created_at: '2026-11-09T05:00:00Z',
    }),
  ];

  async function setupLibrary() {
    const client = new CatalogIpcClient({
      async invokeGetLibrarySnapshot() {
        return makeSnapshot(sampleCatalog);
      },
      async invokeRegisterWallpapers() {
        throw new Error('not called');
      },
    });
    const routerStore = new RouterStore('library', 1920, 1080);
    const catalogStore = new CatalogStore(client);
    await catalogStore.loadCatalog();
    const shell = new AppShellController({ routerStore, catalogStore });
    return { routerStore, catalogStore, shell };
  }

  it('§14.1 Search: matches partial titles, case-insensitively, trims whitespace, handles Unicode/punctuation, searches author/description/tags, and never matches UUIDs', async () => {
    const { catalogStore, shell } = await setupLibrary();

    // 1. Empty / whitespace-only query returns all 5 records
    catalogStore.setSearchQuery('   \t  ');
    let tree = shell.render();
    assert.equal(getRenderedCardIds(tree).length, 5);
    assert.equal(
      getTextContent(findNodeByTestId(tree, 'library-results-count')!),
      'Showing all 5 registered wallpapers',
    );
    assert.equal(findNodeByTestId(tree, 'library-active-filter-indicator'), null);

    // 2. Partial case-insensitive title match with leading/trailing spaces + punctuation
    const searchInput = findNodeByTestId(tree, 'library-search-input')!;
    changeNodeValue(searchInput, '  [4k] (loop #1)  ');
    tree = shell.render();
    assert.deepEqual(getRenderedCardIds(tree), [ID_A]);
    assert.equal(
      getTextContent(findNodeByTestId(tree, 'library-results-count')!),
      'Showing 1 of 5 registered wallpapers',
    );
    assert.ok(findNodeByTestId(tree, 'library-active-filter-indicator'));

    // 3. Unicode title match
    changeNodeValue(findNodeByTestId(tree, 'library-search-input')!, 'ネオン');
    tree = shell.render();
    assert.deepEqual(getRenderedCardIds(tree), [ID_B]);

    // 4. Author, description, and tag search
    changeNodeValue(findNodeByTestId(tree, 'library-search-input')!, 'nordic');
    tree = shell.render();
    assert.deepEqual(getRenderedCardIds(tree), [ID_C, ID_D]);

    changeNodeValue(findNodeByTestId(tree, 'library-search-input')!, 'synthwave');
    tree = shell.render();
    assert.deepEqual(getRenderedCardIds(tree), [ID_A]);

    // 5. Internal UUID is never matched as searchable user text
    changeNodeValue(findNodeByTestId(tree, 'library-search-input')!, ID_A);
    tree = shell.render();
    assert.deepEqual(getRenderedCardIds(tree), []);
    assert.ok(findNodeByTestId(tree, 'library-no-matches-state'));
    assert.equal(findNodeByTestId(tree, 'library-empty-state'), null);

    // 6. Keyboard Escape on search input clears search text
    const inputWithUuid = findNodeByTestId(tree, 'library-search-input')!;
    inputWithUuid.onKeyDown?.({ key: 'Escape' });
    tree = shell.render();
    assert.equal(catalogStore.getState().query.searchQuery, '');
    assert.equal(getRenderedCardIds(tree).length, 5);
  });

  it('§14.2 Filtering: filters accurately by supported WallpaperType and verified AvailabilityState', async () => {
    const { shell } = await setupLibrary();
    let tree = shell.render();

    // 1. Filter by Video type (excludes web and unknown_future_type)
    clickNode(findNodeByTestId(tree, 'library-type-filter-video')!);
    tree = shell.render();
    assert.deepEqual(getRenderedCardIds(tree), [ID_A, ID_C]);
    assert.equal(
      findNodeByTestId(tree, 'library-type-filter-video')?.attrs['aria-pressed'],
      'true',
    );

    // 2. Filter by Web type (keyboard activation; ordered by default `oldest_asc`: ID_D [Nov 11] then ID_B [Nov 12])
    activateByKeyboard(findNodeByTestId(tree, 'library-type-filter-web')!, 'Enter');
    tree = shell.render();
    assert.deepEqual(getRenderedCardIds(tree), [ID_D, ID_B]);

    // 3. Reset type filter to All Types
    clickNode(findNodeByTestId(tree, 'library-type-filter-all')!);
    tree = shell.render();
    assert.equal(getRenderedCardIds(tree).length, 5);

    // 4. Filter by Availability: Available (only verified available)
    clickNode(findNodeByTestId(tree, 'library-availability-filter-available')!);
    tree = shell.render();
    assert.deepEqual(getRenderedCardIds(tree), [ID_A, ID_B]);

    // 5. Filter by Availability: Unavailable (missing + inaccessible + invalid_asset; excludes unchecked)
    clickNode(findNodeByTestId(tree, 'library-availability-filter-unavailable')!);
    tree = shell.render();
    assert.deepEqual(getRenderedCardIds(tree), [ID_E, ID_C]);

    // 6. Filter by Availability: Unverified (`unchecked` only)
    clickNode(findNodeByTestId(tree, 'library-availability-filter-unchecked')!);
    tree = shell.render();
    assert.deepEqual(getRenderedCardIds(tree), [ID_D]);
  });

  it('§14.3 Sorting: sorts deterministically by Oldest Added, Recently Added, Title A–Z, and Title Z–A with stable ID tie-breaking and zero record mutation', async () => {
    const { catalogStore, shell } = await setupLibrary();
    const originalSnapshotJson = JSON.stringify(catalogStore.getState().entries);

    // 1. Default is Oldest Added (`oldest_asc`): E (Nov 9), A (Nov 10), C & D (Nov 11, tie-broken ID_C < ID_D), B (Nov 12)
    let tree = shell.render();
    assert.deepEqual(getRenderedCardIds(tree), [ID_E, ID_A, ID_C, ID_D, ID_B]);

    // 2. Recently Added (`recent_desc`): B (Nov 12), C & D (Nov 11, tie-broken ID_C < ID_D), A (Nov 10), E (Nov 9)
    changeNodeValue(findNodeByTestId(tree, 'library-sort-select')!, 'recent_desc');
    tree = shell.render();
    assert.deepEqual(getRenderedCardIds(tree), [ID_B, ID_C, ID_D, ID_A, ID_E]);

    // 3. Title A–Z (`title_asc`): Aurora Borealis (ID_C) before aurora borealis (ID_D), then Cyber-Grid (ID_A), Experimental (ID_E), 東京 ネオン Horizon (ID_B)
    changeNodeValue(findNodeByTestId(tree, 'library-sort-select')!, 'title_asc');
    tree = shell.render();
    assert.deepEqual(getRenderedCardIds(tree), [ID_C, ID_D, ID_A, ID_E, ID_B]);

    // 4. Title Z–A (`title_desc`)
    changeNodeValue(findNodeByTestId(tree, 'library-sort-select')!, 'title_desc');
    tree = shell.render();
    assert.deepEqual(getRenderedCardIds(tree), [ID_B, ID_E, ID_A, ID_D, ID_C]);

    // 5. Verify underlying stored catalog records were not mutated
    assert.equal(JSON.stringify(catalogStore.getState().entries), originalSnapshotJson);
  });

  it('§14.4 & §14.5 Combined behavior, No-Matches vs Empty-Catalog distinction, Clear controls, and Navigation persistence', async () => {
    const { routerStore, catalogStore, shell } = await setupLibrary();
    let tree = shell.render();

    // Combine search ("aurora") + TypeFilter ("web") + Sort ("title_desc")
    changeNodeValue(findNodeByTestId(tree, 'library-search-input')!, 'aurora');
    clickNode(findNodeByTestId(tree, 'library-type-filter-web')!);
    changeNodeValue(findNodeByTestId(tree, 'library-sort-select')!, 'title_desc');

    tree = shell.render();
    assert.deepEqual(getRenderedCardIds(tree), [ID_D]);
    assert.equal(
      getTextContent(findNodeByTestId(tree, 'library-results-count')!),
      'Showing 1 of 5 registered wallpapers',
    );

    // Narrow further with AvailabilityFilter ("available") -> 0 matches
    clickNode(findNodeByTestId(tree, 'library-availability-filter-available')!);
    tree = shell.render();
    assert.deepEqual(getRenderedCardIds(tree), []);
    assert.ok(findNodeByTestId(tree, 'library-no-matches-state'));
    assert.equal(findNodeByTestId(tree, 'library-empty-state'), null);

    // Navigate to Dashboard and back -> preserves active query state (Principle E)
    shell.navigate('dashboard');
    shell.navigate('library');
    tree = shell.render();
    assert.equal(catalogStore.getState().query.searchQuery, 'aurora');
    assert.equal(catalogStore.getState().query.typeFilter, 'web');
    assert.equal(catalogStore.getState().query.availabilityFilter, 'available');
    assert.equal(catalogStore.getState().query.sortOrder, 'title_desc');

    // Click "Clear Search & Filters" inside No-Matches panel -> resets search and filters while keeping sortOrder ('title_desc')
    clickNode(findNodeByTestId(tree, 'library-no-matches-clear-btn')!);
    tree = shell.render();
    assert.equal(catalogStore.getState().query.searchQuery, '');
    assert.equal(catalogStore.getState().query.typeFilter, 'all');
    assert.equal(catalogStore.getState().query.availabilityFilter, 'all');
    assert.equal(catalogStore.getState().query.sortOrder, 'title_desc');
    assert.deepEqual(getRenderedCardIds(tree), [ID_B, ID_E, ID_A, ID_D, ID_C]);
  });
});
