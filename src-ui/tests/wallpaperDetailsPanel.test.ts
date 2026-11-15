/**
 * Step 29 Automated Verification Suite (`AC-01` through `AC-15`):
 * Wallpaper Details Panel & Metadata Inspector.
 */

import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { AppShellController } from '../components/shell/AppShell.ts';
import {
  activateByKeyboard,
  changeNodeValue,
  clickNode,
  findNodeByTestId,
  getTextContent,
  renderToHtml,
} from '../components/primitives/uiNode.ts';
import { CatalogIpcClient } from '../services/catalogIpcClient.ts';
import { CatalogStore } from '../stores/catalogStore.ts';
import { RouterStore } from '../stores/routerStore.ts';
import type { CatalogEntrySummary, CatalogLibrarySnapshot } from '../types/catalog.ts';

const ID_1 = '11111111-1111-4111-8111-111111111111';
const ID_2 = '22222222-2222-4222-8222-222222222222';
const ID_3 = '33333333-3333-4333-8333-333333333333';

function makeEntry(overrides: Partial<CatalogEntrySummary> & { id: string }): CatalogEntrySummary {
  return {
    id: overrides.id,
    schema_version: overrides.schema_version ?? 1,
    title: overrides.title ?? 'Sample Wallpaper',
    description: overrides.description ?? '',
    wallpaper_type: overrides.wallpaper_type ?? 'video',
    source_kind: overrides.source_kind ?? 'managed_package',
    author: overrides.author ?? 'Unknown',
    version: overrides.version ?? '1.0.0',
    min_engine_version: overrides.min_engine_version ?? '1.0.0',
    license: overrides.license ?? 'Custom',
    tags: overrides.tags ?? [],
    availability: overrides.availability ?? { status: 'available' },
    created_at: overrides.created_at ?? '2026-11-10T14:30:00Z',
    updated_at: overrides.updated_at ?? '2026-11-12T09:15:00Z',
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

describe('Step 29: Wallpaper Details Panel & Metadata Inspector', () => {
  it('§14.1 Renders complete verified metadata, preview image with load-failure fallback, missing optional fields, and duplicate-title separation', async () => {
    const entries: CatalogEntrySummary[] = [
      makeEntry({
        id: ID_1,
        title: 'Identical Title',
        description: 'Rich cyberpunk cityscape with volumetric rain.',
        wallpaper_type: 'video',
        source_kind: 'managed_package',
        author: 'Studio Kineti',
        version: '2.4.1',
        min_engine_version: '1.2.0',
        license: 'CC-BY-4.0',
        schema_version: 1,
        is_builtin: true,
        tags: ['neon', 'rain', 'city', '4k', 'hdr'],
        availability: { status: 'available' },
        has_thumbnail_ref: true,
        thumbnail_ref: 'previews/hero.png',
        created_at: '2026-11-01T08:05:00Z',
        updated_at: '2026-11-11T19:45:00Z',
      }),
      makeEntry({
        id: ID_2,
        title: 'Identical Title',
        description: '',
        wallpaper_type: 'shader_v9',
        source_kind: 'external_direct_file',
        author: 'External Creator',
        tags: [],
        availability: { status: 'invalid_asset', reason: 'Header signature mismatch' },
        has_thumbnail_ref: false,
        thumbnail_ref: null,
        is_builtin: false,
      }),
    ];

    const client = new CatalogIpcClient({
      async invokeGetLibrarySnapshot() {
        return makeSnapshot(entries);
      },
      async invokeRegisterWallpapers() {
        throw new Error('not called');
      },
    });

    const routerStore = new RouterStore('library', 1920, 1080);
    const catalogStore = new CatalogStore(client);
    await catalogStore.loadCatalog();
    const shell = new AppShellController({ routerStore, catalogStore });

    // Select first record (ID_1)
    let tree = shell.render();
    clickNode(findNodeByTestId(tree, `wallpaper-card-select-${ID_1}`)!);
    tree = shell.render();

    const inspector = findNodeByTestId(tree, 'library-selection-inspector');
    assert.ok(inspector);
    assert.equal(inspector.attrs['data-selected-id'], ID_1);
    assert.equal(getTextContent(findNodeByTestId(tree, 'inspector-title')!), 'Identical Title');
    assert.equal(
      getTextContent(findNodeByTestId(tree, 'inspector-description')!),
      'Rich cyberpunk cityscape with volumetric rain.',
    );
    assert.equal(getTextContent(findNodeByTestId(tree, 'inspector-author')!), 'Studio Kineti');
    assert.equal(getTextContent(findNodeByTestId(tree, 'inspector-version')!), '2.4.1');
    assert.equal(getTextContent(findNodeByTestId(tree, 'inspector-min-engine')!), '1.2.0');
    assert.equal(getTextContent(findNodeByTestId(tree, 'inspector-license')!), 'CC-BY-4.0');
    assert.equal(getTextContent(findNodeByTestId(tree, 'inspector-schema-version')!), 'v1');
    assert.equal(getTextContent(findNodeByTestId(tree, 'inspector-origin')!), 'Built-in Package');
    assert.equal(
      getTextContent(findNodeByTestId(tree, 'inspector-created-at')!),
      '2026-11-01 08:05 UTC',
    );
    assert.equal(
      getTextContent(findNodeByTestId(tree, 'inspector-updated-at')!),
      '2026-11-11 19:45 UTC',
    );
    assert.equal(getTextContent(findNodeByTestId(tree, 'inspector-record-id')!), ID_1);

    // All 5 tags are displayed in the details panel (unlike card which truncates at 3)
    const tagsNode = findNodeByTestId(tree, 'inspector-tags')!;
    assert.ok(tagsNode);
    assert.equal(tagsNode.children.length, 5);

    // Preview image is rendered and falls back cleanly on load error
    const previewImg = findNodeByTestId(tree, 'inspector-preview-img');
    assert.ok(previewImg);
    previewImg.onImageError?.();
    tree = shell.render();
    assert.equal(findNodeByTestId(tree, 'inspector-preview-img'), null);
    const failedPlaceholder = findNodeByTestId(tree, 'inspector-preview-placeholder')!;
    assert.ok(failedPlaceholder);
    assert.equal(failedPlaceholder.attrs['data-reason'], 'load_failed');
    assert.ok(getTextContent(failedPlaceholder).includes('Preview Image Could Not Be Loaded'));

    // Switch selection to ID_2 (which shares the identical title "Identical Title"!)
    clickNode(findNodeByTestId(tree, `wallpaper-card-select-${ID_2}`)!);
    tree = shell.render();
    assert.equal(
      findNodeByTestId(tree, 'library-selection-inspector')?.attrs['data-selected-id'],
      ID_2,
    );
    assert.equal(getTextContent(findNodeByTestId(tree, 'inspector-author')!), 'External Creator');
    assert.ok(findNodeByTestId(tree, 'inspector-description-empty'));
    assert.ok(findNodeByTestId(tree, 'inspector-tags-empty'));
    assert.equal(
      getTextContent(findNodeByTestId(tree, 'inspector-canonical-type')!),
      'unsupported (shader_v9)',
    );
    assert.ok(
      getTextContent(findNodeByTestId(tree, 'inspector-availability-badge')!).includes(
        'Invalid Asset',
      ),
    );
    assert.equal(
      findNodeByTestId(tree, 'inspector-preview-placeholder')?.attrs['data-reason'],
      'absent',
    );
  });

  it('§14.2 & §14.3 Preserves selected record across sort/filter/refresh, indicates when hidden by filter, and restores focus on Escape or Close', async () => {
    let entries: CatalogEntrySummary[] = [
      makeEntry({
        id: ID_1,
        title: 'Alpha Video',
        wallpaper_type: 'video',
        availability: { status: 'unchecked' },
      }),
      makeEntry({
        id: ID_2,
        title: 'Beta Web',
        wallpaper_type: 'web',
        availability: { status: 'available' },
      }),
    ];

    const client = new CatalogIpcClient({
      async invokeGetLibrarySnapshot() {
        return makeSnapshot(entries);
      },
      async invokeRegisterWallpapers() {
        throw new Error('not called');
      },
    });

    const routerStore = new RouterStore('library', 1280, 800);
    const catalogStore = new CatalogStore(client);
    await catalogStore.loadCatalog();
    const shell = new AppShellController({ routerStore, catalogStore });

    // Select ID_1 via keyboard Space
    let tree = shell.render();
    activateByKeyboard(findNodeByTestId(tree, `wallpaper-card-select-${ID_1}`)!, ' ');
    tree = shell.render();
    assert.equal(
      findNodeByTestId(tree, 'library-selection-inspector')?.attrs['data-selected-id'],
      ID_1,
    );
    assert.equal(findNodeByTestId(tree, 'inspector-hidden-by-filter-banner'), null);

    // Change sort order -> selection identity remains ID_1
    changeNodeValue(findNodeByTestId(tree, 'library-sort-select')!, 'title_desc');
    tree = shell.render();
    assert.equal(
      findNodeByTestId(tree, 'library-selection-inspector')?.attrs['data-selected-id'],
      ID_1,
    );

    // Filter by Web type -> ID_1 is hidden from grid, but details panel stays open and shows hidden-by-filter notice
    clickNode(findNodeByTestId(tree, 'library-type-filter-web')!);
    tree = shell.render();
    assert.equal(findNodeByTestId(tree, `wallpaper-card-${ID_1}`), null);
    assert.ok(findNodeByTestId(tree, 'library-selection-inspector'));
    assert.ok(findNodeByTestId(tree, 'inspector-hidden-by-filter-banner'));

    // Clear filters -> ID_1 is back in grid and hidden-by-filter banner disappears
    clickNode(findNodeByTestId(tree, 'library-clear-filters-btn')!);
    tree = shell.render();
    assert.equal(findNodeByTestId(tree, 'inspector-hidden-by-filter-banner'), null);

    // Refresh catalog with updated availability for ID_1 -> inspector reflects updated availability
    entries = [
      makeEntry({
        id: ID_1,
        title: 'Alpha Video',
        wallpaper_type: 'video',
        availability: { status: 'available' },
      }),
      entries[1]!,
    ];
    await catalogStore.loadCatalog({ forceRefresh: true });
    tree = shell.render();
    assert.ok(
      getTextContent(findNodeByTestId(tree, 'inspector-availability-badge')!).includes('Available'),
    );

    // Close panel via Escape key -> closes panel, preserves sort order ('title_desc'), and marks ID_1 card with data-focus-restored="true"
    const panel = findNodeByTestId(tree, 'library-selection-inspector')!;
    panel.onKeyDown?.({ key: 'Escape' });
    tree = shell.render();
    assert.equal(findNodeByTestId(tree, 'library-selection-inspector'), null);
    assert.equal(catalogStore.getState().query.sortOrder, 'title_desc');
    assert.equal(
      findNodeByTestId(tree, `wallpaper-card-select-${ID_1}`)?.attrs['data-focus-restored'],
      'true',
    );
  });

  it('§14.4 Enforces read-only inspection, escapes untrusted XSS metadata in details panel, and redacts raw OS paths', async () => {
    const xssRecord = makeEntry({
      id: ID_3,
      title: '<script>window.pwned=true</script>Danger Title',
      description: '<iframe src="javascript:alert(1)"></iframe>',
      author: '<svg onload=alert(2)>',
      license: '<img src=x onerror=alert(3)>',
      tags: ['<script>tag</script>'],
      availability: {
        status: 'inaccessible',
        reason: 'Denied opening C:\\Users\\Admin\\Private\\secret.mp4',
      },
    });

    const client = new CatalogIpcClient({
      async invokeGetLibrarySnapshot() {
        return makeSnapshot([xssRecord]);
      },
      async invokeRegisterWallpapers() {
        throw new Error('not called');
      },
    });

    const routerStore = new RouterStore('library', 1280, 800);
    const catalogStore = new CatalogStore(client);
    await catalogStore.loadCatalog();
    const beforeInspectJson = JSON.stringify(catalogStore.getState().entries);

    const shell = new AppShellController({ routerStore, catalogStore });
    catalogStore.selectWallpaper(ID_3);
    const tree = shell.render();
    const html = renderToHtml(findNodeByTestId(tree, 'library-selection-inspector')!);

    assert.ok(!html.includes('<script>'));
    assert.ok(!html.includes('<iframe'));
    assert.ok(!html.includes('<svg onload'));
    assert.ok(!html.includes('C:\\Users\\Admin'));
    assert.ok(html.includes('[redacted-path]'));
    assert.equal(JSON.stringify(catalogStore.getState().entries), beforeInspectJson);
  });
});

