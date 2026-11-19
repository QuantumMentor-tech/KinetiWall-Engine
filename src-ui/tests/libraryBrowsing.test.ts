/**
 * Step 27 Automated Verification Suite (`AC-01` through `AC-15`):
 * Wallpaper Library Browsing UI & Responsive Catalog Grid.
 */

import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { AppShellController } from '../components/shell/AppShell.ts';
import {
  activateByKeyboard,
  clickNode,
  findAllByTestIdPrefix,
  findNodeByTestId,
  getTextContent,
  renderToHtml,
} from '../components/primitives/uiNode.ts';
import { CatalogIpcClient } from '../services/catalogIpcClient.ts';
import { CatalogStore } from '../stores/catalogStore.ts';
import { RouterStore } from '../stores/routerStore.ts';
import type {
  CatalogEntrySummary,
  CatalogLibrarySnapshot,
  RegistrationBatchReport,
} from '../types/catalog.ts';

const ID_1 = '11111111-1111-4111-8111-111111111111';
const ID_2 = '22222222-2222-4222-8222-222222222222';
const ID_3 = '33333333-3333-4333-8333-333333333333';
const ID_4 = '44444444-4444-4444-8444-444444444444';

function makeEntry(overrides: Partial<CatalogEntrySummary> & { id: string }): CatalogEntrySummary {
  return {
    id: overrides.id,
    title: overrides.title ?? 'Sample Wallpaper',
    description: overrides.description ?? '',
    wallpaper_type: overrides.wallpaper_type ?? 'video',
    source_kind: overrides.source_kind ?? 'managed_package',
    author: overrides.author ?? 'Artist',
    tags: overrides.tags ?? ['ambient', 'loop'],
    availability: overrides.availability ?? { status: 'available' },
    created_at: overrides.created_at ?? '2026-11-12T01:00:00Z',
    updated_at: overrides.updated_at ?? '2026-11-12T01:00:00Z',
    is_builtin: overrides.is_builtin ?? false,
    has_thumbnail_ref: overrides.has_thumbnail_ref ?? false,
    thumbnail_ref: overrides.thumbnail_ref ?? null,
  };
}

function makeSnapshot(
  entries: CatalogEntrySummary[],
  loadIssueCount = 0,
): CatalogLibrarySnapshot {
  return {
    entries,
    integrity: {
      valid_record_count: entries.length,
      available_count: entries.filter((e) => e.availability.status === 'available').length,
      missing_count: entries.filter((e) => e.availability.status === 'missing').length,
      inaccessible_count: entries.filter((e) => e.availability.status === 'inaccessible').length,
      invalid_asset_count: entries.filter((e) => e.availability.status === 'invalid_asset').length,
      unchecked_count: entries.filter((e) => e.availability.status === 'unchecked').length,
      load_issue_count: loadIssueCount,
    },
    load_issues:
      loadIssueCount > 0
        ? [
            {
              directory_name: 'corrupt-pkg',
              redacted_path: '[redacted]/project.json',
              reason: 'manifest_version unsupported',
            },
          ]
        : [],
  };
}

describe('Step 27: Wallpaper Library Browsing UI & Responsive Catalog Grid', () => {
  it('AC-01 & AC-07: navigates from Dashboard to Wallpaper Library via AppShell with accurate active state and anti-thrashing', async () => {
    let fetchCount = 0;
    const client = new CatalogIpcClient({
      async invokeGetLibrarySnapshot() {
        fetchCount += 1;
        return makeSnapshot([makeEntry({ id: ID_1, title: 'Cyber Horizon' })]);
      },
      async invokeRegisterWallpapers() {
        throw new Error('not called');
      },
    });

    const routerStore = new RouterStore('dashboard', 1280, 800);
    const catalogStore = new CatalogStore(client);
    const shell = new AppShellController({ routerStore, catalogStore });

    // Initially on Dashboard
    let tree = shell.render();
    assert.ok(findNodeByTestId(tree, 'dashboard-view'));
    const navDashboard = findNodeByTestId(tree, 'nav-item-dashboard');
    const navLibrary = findNodeByTestId(tree, 'nav-item-library');
    const navSettings = findNodeByTestId(tree, 'nav-item-settings');
    assert.equal(navDashboard?.attrs['aria-current'], 'page');
    assert.equal(navLibrary?.attrs['aria-current'], undefined);
    assert.equal(navSettings?.attrs.disabled, false);

    // Navigate via Dashboard "Browse Wallpaper Library" button
    const browseBtn = findNodeByTestId(tree, 'dashboard-browse-library-btn');
    assert.ok(browseBtn);
    clickNode(browseBtn);

    // Wait for automatic initial catalog load
    await catalogStore.loadCatalog();
    tree = shell.render();

    assert.equal(routerStore.getState().activeRoute, 'library');
    assert.ok(findNodeByTestId(tree, 'library-view'));
    assert.equal(findNodeByTestId(tree, 'nav-item-library')?.attrs['aria-current'], 'page');
    assert.equal(fetchCount, 1);

    // Anti-thrashing: clicking active Library nav item again is a no-op and does not refetch
    const activeLibNav = findNodeByTestId(tree, 'nav-item-library')!;
    clickNode(activeLibNav);
    assert.equal(fetchCount, 1);
  });

  it('AC-02, AC-03 & AC-04: renders real catalog records with stable UUID keys, duplicate title separation, and accurate metadata/previews', async () => {
    const entries = [
      makeEntry({
        id: ID_1,
        title: 'Duplicate Title',
        wallpaper_type: 'video',
        source_kind: 'managed_package',
        has_thumbnail_ref: true,
        thumbnail_ref: 'thumbs/cover.png',
        availability: { status: 'available' },
        tags: ['cyber', '60fps', 'hdr', 'loop'],
      }),
      makeEntry({
        id: ID_2,
        title: 'Duplicate Title',
        wallpaper_type: 'web',
        source_kind: 'external_direct_file',
        has_thumbnail_ref: false,
        thumbnail_ref: null,
        availability: { status: 'unchecked' },
        tags: [],
      }),
      makeEntry({
        id: ID_3,
        title: 'Missing Media Item',
        wallpaper_type: 'shader_unknown',
        availability: { status: 'missing' },
        has_thumbnail_ref: true,
        thumbnail_ref: '../outside/secret.png', // unsafe traversal must be rejected to placeholder
      }),
    ];

    const client = new CatalogIpcClient({
      async invokeGetLibrarySnapshot() {
        return makeSnapshot(entries, 1);
      },
      async invokeRegisterWallpapers() {
        throw new Error('not used');
      },
    });

    const routerStore = new RouterStore('library', 1920, 1080);
    const catalogStore = new CatalogStore(client);
    await catalogStore.loadCatalog();
    const shell = new AppShellController({ routerStore, catalogStore });

    let tree = shell.render();
    const card1 = findNodeByTestId(tree, `wallpaper-card-${ID_1}`);
    const card2 = findNodeByTestId(tree, `wallpaper-card-${ID_2}`);
    const card3 = findNodeByTestId(tree, `wallpaper-card-${ID_3}`);
    assert.ok(card1);
    assert.ok(card2);
    assert.ok(card3);

    // Both duplicate-titled records have distinct stable keys and independent selection
    assert.equal(card1.key, ID_1);
    assert.equal(card2.key, ID_2);

    // Card 1 has a valid thumbnail image and "Available" badge
    const img1 = findNodeByTestId(tree, `wallpaper-card-img-${ID_1}`);
    assert.ok(img1);
    assert.ok(String(img1.attrs.src).startsWith('kinetiwall-asset://localhost/wallpapers/'));
    assert.ok(
      getTextContent(findNodeByTestId(tree, `wallpaper-availability-badge-${ID_1}`)!).includes(
        'Available',
      ),
    );

    // Simulate thumbnail load failure on Card 1 -> falls back cleanly to placeholder without breaking card
    img1.onImageError?.();
    tree = shell.render();
    assert.equal(findNodeByTestId(tree, `wallpaper-card-img-${ID_1}`), null);
    assert.ok(findNodeByTestId(tree, `wallpaper-card-placeholder-${ID_1}`));

    // Card 2 has no thumbnail -> renders deliberate placeholder and "Unverified" status (not false Available)
    assert.ok(findNodeByTestId(tree, `wallpaper-card-placeholder-${ID_2}`));
    assert.ok(
      getTextContent(findNodeByTestId(tree, `wallpaper-availability-badge-${ID_2}`)!).includes(
        'Unverified',
      ),
    );
    assert.equal(
      getTextContent(findNodeByTestId(tree, `wallpaper-card-source-${ID_2}`)!),
      'Local Reference',
    );

    // Card 3 has unsafe "../" thumbnail ref -> rejected to placeholder, unknown type -> "Unsupported Type", missing -> "File Missing"
    assert.equal(findNodeByTestId(tree, `wallpaper-card-img-${ID_3}`), null);
    assert.ok(findNodeByTestId(tree, `wallpaper-card-placeholder-${ID_3}`));
    assert.ok(
      getTextContent(findNodeByTestId(tree, `wallpaper-type-badge-${ID_3}`)!).includes(
        'Unsupported Type',
      ),
    );
    assert.ok(
      getTextContent(findNodeByTestId(tree, `wallpaper-availability-badge-${ID_3}`)!).includes(
        'File Missing',
      ),
    );

    // Partial-availability / integrity warning banner is displayed
    assert.ok(findNodeByTestId(tree, 'library-integrity-warning-banner'));
  });

  it('AC-05 & AC-10: handles initial loading, dedicated empty state, initial error state, and preserves existing records on stale refresh failure', async () => {
    let mode: 'empty' | 'error' | 'populated' = 'empty';
    let resolvePendingLoad: (() => void) | null = null;

    const client = new CatalogIpcClient({
      async invokeGetLibrarySnapshot() {
        if (resolvePendingLoad) {
          await new Promise<void>((resolve) => {
            resolvePendingLoad = resolve;
          });
          resolvePendingLoad = null;
        }
        if (mode === 'error') {
          throw new Error('Failed reading C:\\Users\\Alice\\AppData\\Local\\KinetiWall\\wallpapers');
        }
        if (mode === 'empty') {
          return makeSnapshot([]);
        }
        return makeSnapshot([makeEntry({ id: ID_1, title: 'Persistent Aurora' })]);
      },
      async invokeRegisterWallpapers() {
        throw new Error('not used');
      },
    });

    const routerStore = new RouterStore('library', 1280, 800);
    const catalogStore = new CatalogStore(client);
    const shell = new AppShellController({ routerStore, catalogStore });

    // 1. Initial loading state (before promise resolves): shows spinner, NOT false empty state
    resolvePendingLoad = () => {};
    const loadPromise = catalogStore.loadCatalog({ forceRefresh: true });
    let tree = shell.render();
    assert.ok(findNodeByTestId(tree, 'library-loading-state'));
    assert.equal(findNodeByTestId(tree, 'library-empty-state'), null);
    resolvePendingLoad();
    await loadPromise;

    // 2. Empty state when 0 records returned
    tree = shell.render();
    assert.ok(findNodeByTestId(tree, 'library-empty-state'));
    assert.ok(findNodeByTestId(tree, 'library-empty-add-btn'));
    assert.equal(findNodeByTestId(tree, 'library-error-state'), null);

    // 3. Initial error state when 0 prior records exist -> shows error banner with redacted path & retry button
    mode = 'error';
    await catalogStore.loadCatalog({ forceRefresh: true });
    tree = shell.render();
    const errBanner = findNodeByTestId(tree, 'library-error-state');
    assert.ok(errBanner);
    const errText = getTextContent(errBanner);
    assert.ok(!errText.includes('C:\\Users\\Alice'));
    assert.ok(errText.includes('[redacted-path]'));

    // 4. Retry succeeds -> populates grid
    mode = 'populated';
    clickNode(findNodeByTestId(tree, 'library-error-retry-btn')!);
    await catalogStore.loadCatalog();
    tree = shell.render();
    assert.ok(findNodeByTestId(tree, `wallpaper-card-${ID_1}`));

    // 5. Subsequent refresh fails -> preserves previously loaded card AND shows stale-data warning banner
    mode = 'error';
    await catalogStore.loadCatalog({ forceRefresh: true });
    tree = shell.render();
    assert.ok(findNodeByTestId(tree, `wallpaper-card-${ID_1}`));
    assert.ok(findNodeByTestId(tree, 'library-stale-warning-banner'));
    assert.equal(findNodeByTestId(tree, 'library-empty-state'), null);
  });

  it('AC-08 & AC-11: supports keyboard and pointer card selection for inspection only, without false Playing states, and preserves selection across refresh', async () => {
    let entries = [
      makeEntry({ id: ID_1, title: 'First Entry', description: 'Calm ocean waves' }),
      makeEntry({ id: ID_2, title: 'Second Entry' }),
    ];

    const client = new CatalogIpcClient({
      async invokeGetLibrarySnapshot() {
        return makeSnapshot(entries);
      },
      async invokeRegisterWallpapers() {
        throw new Error('not used');
      },
    });

    const routerStore = new RouterStore('library', 1280, 800);
    const catalogStore = new CatalogStore(client);
    await catalogStore.loadCatalog();
    const shell = new AppShellController({ routerStore, catalogStore });

    let tree = shell.render();
    const trigger1 = findNodeByTestId(tree, `wallpaper-card-select-${ID_1}`)!;
    assert.equal(trigger1.attrs['aria-pressed'], 'false');

    // Select via keyboard Enter
    activateByKeyboard(trigger1, 'Enter');
    tree = shell.render();
    assert.equal(
      findNodeByTestId(tree, `wallpaper-card-select-${ID_1}`)?.attrs['aria-pressed'],
      'true',
    );
    const inspector = findNodeByTestId(tree, 'library-selection-inspector');
    assert.ok(inspector);
    assert.equal(getTextContent(findNodeByTestId(tree, 'inspector-title')!), 'First Entry');
    assert.ok(
      getTextContent(findNodeByTestId(tree, 'inspector-boundary-notice')!).includes(
        'inspection only',
      ),
    );
    // Ensure no false "Playing" badge is rendered anywhere
    assert.ok(!renderToHtml(tree).includes('Playing'));

    // Refresh catalog while ID_1 still exists -> selection is preserved
    await catalogStore.loadCatalog({ forceRefresh: true });
    assert.equal(catalogStore.getState().selectedWallpaperId, ID_1);

    // Refresh catalog after ID_1 is removed -> stale selection is cleared
    entries = [makeEntry({ id: ID_2, title: 'Second Entry' })];
    await catalogStore.loadCatalog({ forceRefresh: true });
    assert.equal(catalogStore.getState().selectedWallpaperId, null);
  });

  it('AC-09: integrates with Step 26 registration workflow, handles cancellation cleanly, and refreshes after confirmed persistence', async () => {
    const entries: CatalogEntrySummary[] = [makeEntry({ id: ID_1, title: 'Existing Item' })];
    let nextRegReport: RegistrationBatchReport = {
      overall_status: 'cancelled',
      total_selected: 0,
      registered_count: 0,
      already_present_count: 0,
      rejected_count: 0,
      inaccessible_count: 0,
      failed_count: 0,
      items: [],
    };
    let snapshotCalls = 0;

    const client = new CatalogIpcClient({
      async invokeGetLibrarySnapshot() {
        snapshotCalls += 1;
        return makeSnapshot([...entries]);
      },
      async invokeRegisterWallpapers() {
        if (nextRegReport.overall_status === 'succeeded') {
          entries.push(
            makeEntry({
              id: ID_4,
              title: 'Newly Registered Web Scene',
              wallpaper_type: 'web',
              source_kind: 'external_direct_file',
            }),
          );
        }
        return nextRegReport;
      },
    });

    const routerStore = new RouterStore('library', 1280, 800);
    const catalogStore = new CatalogStore(client);
    await catalogStore.loadCatalog();
    const shell = new AppShellController({ routerStore, catalogStore });
    assert.equal(snapshotCalls, 1);

    // 1. User cancels file picker -> no error banner, no unnecessary catalog reload
    await catalogStore.triggerRegistration();
    let tree = shell.render();
    assert.equal(snapshotCalls, 1);
    assert.equal(findNodeByTestId(tree, 'library-registration-report-banner'), null);
    assert.equal(findNodeByTestId(tree, 'library-registration-error-banner'), null);

    // 2. User registers a new valid wallpaper -> refreshes catalog after persistence and shows outcome banner
    nextRegReport = {
      overall_status: 'succeeded',
      total_selected: 1,
      registered_count: 1,
      already_present_count: 0,
      rejected_count: 0,
      inaccessible_count: 0,
      failed_count: 0,
      items: [],
    };
    await catalogStore.triggerRegistration();
    tree = shell.render();
    assert.equal(snapshotCalls, 2);
    assert.ok(findNodeByTestId(tree, `wallpaper-card-${ID_4}`));
    const reportBanner = findNodeByTestId(tree, 'library-registration-report-banner');
    assert.ok(reportBanner);
    assert.ok(getTextContent(reportBanner).includes('Registered 1 new item(s)'));
  });

  it('AC-06 & AC-12: adapts responsive grid columns across viewports (960px to 4K) and escapes untrusted XSS metadata', async () => {
    const xssEntry = makeEntry({
      id: ID_1,
      title: '<script>alert("xss")</script>Neon Grid',
      description: '<img src=x onerror=alert(1)>',
      author: '<b>Untrusted Author</b>',
      availability: {
        status: 'inaccessible',
        reason: 'Locked at C:\\Users\\SecretUser\\Videos\\loop.mp4',
      },
    });

    const client = new CatalogIpcClient({
      async invokeGetLibrarySnapshot() {
        return makeSnapshot([xssEntry]);
      },
      async invokeRegisterWallpapers() {
        throw new Error('not used');
      },
    });

    const routerStore = new RouterStore('library', 960, 600);
    const catalogStore = new CatalogStore(client);
    await catalogStore.loadCatalog();
    const shell = new AppShellController({ routerStore, catalogStore });

    const expectedColsByWidth: Array<[number, string]> = [
      [960, '2'],
      [1280, '3'],
      [1920, '4'],
      [2560, '5'],
      [3840, '6'],
    ];

    for (const [width, expectedCols] of expectedColsByWidth) {
      routerStore.setViewportSize(width, 900);
      const tree = shell.render();
      const grid = findNodeByTestId(tree, 'library-catalog-grid');
      assert.ok(grid);
      assert.equal(grid.attrs['data-columns'], expectedCols);
      assert.equal(findAllByTestIdPrefix(tree, 'wallpaper-card-wc-').length, 0);
    }

    // Verify HTML serialization strictly escapes `<script>` and `<img>` tags and redacts raw OS paths
    const html = renderToHtml(shell.render());
    assert.ok(!html.includes('<script>'));
    assert.ok(html.includes('&lt;script&gt;alert(&quot;xss&quot;)&lt;/script&gt;'));
    assert.ok(!html.includes('C:\\Users\\SecretUser'));
    assert.ok(html.includes('[redacted-path]'));
  });
});

