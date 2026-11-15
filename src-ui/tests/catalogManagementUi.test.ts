import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import {
  changeNodeValue,
  clickNode,
  findNodeByTestId,
  getTextContent,
} from '../components/primitives/uiNode.ts';
import { AppShellController } from '../components/shell/AppShell.ts';
import { CatalogIpcClient, type CatalogIpcTransport } from '../services/catalogIpcClient.ts';
import { CatalogStore } from '../stores/catalogStore.ts';
import { RouterStore } from '../stores/routerStore.ts';
import type {
  CatalogEntrySummary,
  CatalogLibrarySnapshot,
  EditableMetadataPatch,
  MetadataUpdateOutcome,
  RelinkOutcome,
  RemovalOutcome,
  WallpaperId,
} from '../types/catalog.ts';

const ID_ALPHA = '11111111-1111-4111-8111-111111111111';
const ID_BETA = '22222222-2222-4222-8222-222222222222';
const ID_BUILTIN = '33333333-3333-4333-8333-333333333333';

function createInitialEntries(): CatalogEntrySummary[] {
  return [
    {
      id: ID_ALPHA,
      schema_version: 1,
      title: 'Aurora Borealis 4K',
      description: 'Atmospheric northern lights over snowy peaks.',
      wallpaper_type: 'video',
      source_kind: 'external_direct_file',
      author: 'Studio North',
      version: '1.2.0',
      min_engine_version: '1.0.0',
      license: 'CC-BY-4.0',
      tags: ['Nature', 'Night'],
      availability: { status: 'missing' },
      created_at: '2026-10-01T10:00:00Z',
      updated_at: '2026-10-01T10:00:00Z',
      is_builtin: false,
      has_thumbnail_ref: false,
      thumbnail_ref: null,
    },
    {
      id: ID_BETA,
      schema_version: 1,
      title: 'Cyberpunk Matrix Clock',
      description: 'Interactive WebGL clock scene.',
      wallpaper_type: 'web',
      source_kind: 'external_direct_file',
      author: 'KinetiLabs',
      version: '2.0.0',
      min_engine_version: '1.0.0',
      license: 'MIT',
      tags: ['Sci-Fi', 'Clock'],
      availability: { status: 'available' },
      created_at: '2026-10-02T12:00:00Z',
      updated_at: '2026-10-02T12:00:00Z',
      is_builtin: false,
      has_thumbnail_ref: false,
      thumbnail_ref: null,
    },
    {
      id: ID_BUILTIN,
      schema_version: 1,
      title: 'KinetiWall Default Pulse',
      description: 'Built-in ambient shader scene.',
      wallpaper_type: 'web',
      source_kind: 'managed_package',
      author: 'KinetiWall Core',
      version: '1.0.0',
      min_engine_version: '1.0.0',
      license: 'Proprietary',
      tags: ['Builtin'],
      availability: { status: 'available' },
      created_at: '2026-09-01T00:00:00Z',
      updated_at: '2026-09-01T00:00:00Z',
      is_builtin: true,
      has_thumbnail_ref: false,
      thumbnail_ref: null,
    },
  ];
}

function buildSnapshot(entries: CatalogEntrySummary[]): CatalogLibrarySnapshot {
  const available = entries.filter((e) => e.availability.status === 'available').length;
  const missing = entries.filter((e) => e.availability.status === 'missing').length;
  return {
    entries: entries.map((e) => ({ ...e, tags: [...e.tags] })),
    integrity: {
      valid_record_count: entries.length,
      available_count: available,
      missing_count: missing,
      inaccessible_count: 0,
      invalid_asset_count: 0,
      unchecked_count: 0,
      load_issue_count: 0,
    },
    load_issues: [],
  };
}

describe('Step 30: Wallpaper Catalog Management, Missing-Asset Recovery & Safe Removal', () => {
  it('§5 Metadata editing validates input, skips unchanged writes, and propagates confirmed updates across grid, search, and details panel', async () => {
    const authoritativeEntries = createInitialEntries();
    let updateCallCount = 0;

    const transport: CatalogIpcTransport = {
      async invokeGetLibrarySnapshot() {
        return buildSnapshot(authoritativeEntries);
      },
      async invokeRegisterWallpapers() {
        return {
          overall_status: 'cancelled',
          total_selected: 0,
          registered_count: 0,
          already_present_count: 0,
          rejected_count: 0,
          inaccessible_count: 0,
          failed_count: 0,
          items: [],
        };
      },
      async invokeUpdateMetadata(
        id: WallpaperId,
        patch: EditableMetadataPatch,
      ): Promise<MetadataUpdateOutcome> {
        updateCallCount += 1;
        const idx = authoritativeEntries.findIndex((e) => e.id === id);
        assert.ok(idx >= 0, 'record exists in authoritative store');
        const current = authoritativeEntries[idx]!;
        assert.equal(patch.expected_updated_at, current.updated_at);
        const updated: CatalogEntrySummary = {
          ...current,
          title: patch.title,
          description: patch.description,
          tags: [...patch.tags],
          updated_at: '2026-10-15T18:30:00Z',
        };
        authoritativeEntries[idx] = updated;
        return {
          outcome: 'updated',
          summary: updated,
        };
      },
    };

    const routerStore = new RouterStore('library', 1920, 1080);
    const store = new CatalogStore(new CatalogIpcClient(transport));
    await store.loadCatalog();
    const shell = new AppShellController({ routerStore, catalogStore: store });

    // Select ID_ALPHA and open inline metadata editor
    store.selectWallpaper(ID_ALPHA);
    let tree = shell.render();
    clickNode(findNodeByTestId(tree, 'inspector-edit-metadata-btn')!);

    tree = shell.render();
    assert.ok(findNodeByTestId(tree, 'inspector-metadata-edit-form'));

    // 1. Submitting unchanged draft skips IPC write
    clickNode(findNodeByTestId(tree, 'inspector-save-metadata-btn')!);
    await Promise.resolve();
    assert.equal(updateCallCount, 0, 'unchanged save must not invoke IPC write');
    assert.equal(store.getState().editingWallpaperId, null);

    // 2. Re-open editor and test client-side validation rejection (blank title)
    store.startMetadataEdit(ID_ALPHA);
    store.updateMetadataDraft({ title: '    ' });
    const invalidResult = await store.submitMetadataEdit();
    assert.equal(invalidResult, null);
    assert.equal(updateCallCount, 0);
    tree = shell.render();
    const feedbackErr = findNodeByTestId(tree, 'inspector-management-feedback');
    assert.ok(feedbackErr);
    assert.match(getTextContent(feedbackErr), /Title must not be empty/i);

    // 3. Valid edit updates title, description, and tags, preserving protected fields
    const titleInput = findNodeByTestId(tree, 'inspector-edit-title-input');
    const descInput = findNodeByTestId(tree, 'inspector-edit-description-input');
    const tagsInput = findNodeByTestId(tree, 'inspector-edit-tags-input');
    assert.ok(titleInput && descInput && tagsInput);

    changeNodeValue(titleInput, '  Solar Flare Timelapse  ');
    changeNodeValue(descInput, '  Updated description with coronal mass ejection. ');
    changeNodeValue(tagsInput, ' Space, Solar, space ');

    await store.submitMetadataEdit();
    assert.equal(updateCallCount, 1);

    tree = shell.render();
    assert.equal(getTextContent(findNodeByTestId(tree, 'inspector-title')!), 'Solar Flare Timelapse');
    assert.equal(
      getTextContent(findNodeByTestId(tree, 'inspector-description')!),
      'Updated description with coronal mass ejection.',
    );
    assert.equal(getTextContent(findNodeByTestId(tree, 'inspector-author')!), 'Studio North');
    assert.equal(getTextContent(findNodeByTestId(tree, 'inspector-record-id')!), ID_ALPHA);
    assert.equal(
      getTextContent(findNodeByTestId(tree, 'inspector-updated-at')!),
      '2026-10-15 18:30 UTC',
    );

    // Search immediately finds the entry by its newly updated tag ("Solar")
    store.setSearchQuery('Solar');
    assert.deepEqual(
      store.getQueryResult().visibleEntries.map((e) => e.id),
      [ID_ALPHA],
    );
  });

  it('§6 Missing-asset relinking handles cancellation, type mismatch rejection, and valid recovery to Available', async () => {
    const authoritativeEntries = createInitialEntries();
    let nextRelinkResult: RelinkOutcome = {
      outcome: 'cancelled',
      id: ID_ALPHA,
    };

    const transport: CatalogIpcTransport = {
      async invokeGetLibrarySnapshot() {
        return buildSnapshot(authoritativeEntries);
      },
      async invokeRegisterWallpapers() {
        throw new Error('unused');
      },
      async invokeRelinkAsset(id: WallpaperId): Promise<RelinkOutcome> {
        if (nextRelinkResult.outcome === 'relinked') {
          const idx = authoritativeEntries.findIndex((e) => e.id === id);
          authoritativeEntries[idx] = nextRelinkResult.summary;
        }
        return nextRelinkResult;
      },
    };

    const routerStore = new RouterStore('library', 1920, 1080);
    const store = new CatalogStore(new CatalogIpcClient(transport));
    await store.loadCatalog();
    const shell = new AppShellController({ routerStore, catalogStore: store });

    store.selectWallpaper(ID_ALPHA);
    let tree = shell.render();
    assert.equal(
      getTextContent(findNodeByTestId(tree, 'inspector-relink-asset-btn')!),
      'Locate & Relink Asset',
    );

    // 1. Cancelled file picker leaves record Missing and shows no error banner
    await store.relinkWallpaperAsset(ID_ALPHA);
    assert.equal(store.getSelectedWallpaper()?.availability.status, 'missing');
    assert.equal(store.getState().managementFeedback, null);

    // 2. Type mismatch rejection keeps record Missing and shows clear diagnostic
    nextRelinkResult = {
      outcome: 'rejected',
      id: ID_ALPHA,
      redacted_path: '[EXTERNAL_FILE]\\scene.html',
      reason: {
        kind: 'type_mismatch',
        detail: {
          expected_type: 'video',
          candidate_type: 'web',
        },
      },
    };
    await store.relinkWallpaperAsset(ID_ALPHA);
    assert.equal(store.getSelectedWallpaper()?.availability.status, 'missing');
    tree = shell.render();
    assert.match(
      getTextContent(findNodeByTestId(tree, 'inspector-management-feedback')!),
      /expected a VIDEO asset, but selected a WEB file/i,
    );

    // 3. Valid replacement file transitions record to Available and updates library integrity counts
    const recoveredSummary: CatalogEntrySummary = {
      ...authoritativeEntries[0]!,
      availability: { status: 'available' },
      updated_at: '2026-10-16T09:00:00Z',
    };
    nextRelinkResult = {
      outcome: 'relinked',
      id: ID_ALPHA,
      summary: recoveredSummary,
    };
    await store.relinkWallpaperAsset(ID_ALPHA);

    assert.equal(store.getSelectedWallpaper()?.availability.status, 'available');
    assert.equal(store.getState().integrity.available_count, 3);
    assert.equal(store.getState().integrity.missing_count, 0);

    tree = shell.render();
    assert.equal(
      getTextContent(findNodeByTestId(tree, 'inspector-availability-badge')!),
      '●Available',
    );
  });

  it('§7 Catalog-only removal enforces active-assignment & built-in safeguards, requires confirmation, and synchronizes UI state on removal', async () => {
    let authoritativeEntries = createInitialEntries();

    const transport: CatalogIpcTransport = {
      async invokeGetLibrarySnapshot() {
        return buildSnapshot(authoritativeEntries);
      },
      async invokeRegisterWallpapers() {
        throw new Error('unused');
      },
      async invokeRemoveCatalogEntry(id: WallpaperId): Promise<RemovalOutcome> {
        authoritativeEntries = authoritativeEntries.filter((e) => e.id !== id);
        return {
          outcome: 'removed',
          id,
        };
      },
    };

    const routerStore = new RouterStore('library', 1920, 1080);
    const store = new CatalogStore(new CatalogIpcClient(transport));
    await store.loadCatalog();
    const shell = new AppShellController({ routerStore, catalogStore: store });

    // 1. Built-in entry has Remove button disabled and blocks removal
    store.selectWallpaper(ID_BUILTIN);
    let tree = shell.render();
    const builtinRemoveBtn = findNodeByTestId(tree, 'inspector-remove-entry-btn');
    assert.equal(builtinRemoveBtn?.attrs?.disabled, true);
    const builtinOutcome = await store.confirmRemoveWallpaper(ID_BUILTIN);
    assert.equal(builtinOutcome?.outcome, 'blocked_builtin_protected');
    assert.equal(store.getState().entries.length, 3);

    // 2. Active/assigned wallpaper blocks removal with a clear warning notice
    store.setActiveWallpaperIds([ID_BETA]);
    store.selectWallpaper(ID_BETA);
    tree = shell.render();
    assert.ok(findNodeByTestId(tree, 'inspector-active-assignment-note'));
    store.requestRemoveWallpaper(ID_BETA);
    assert.equal(store.getState().pendingRemovalWallpaperId, null);
    tree = shell.render();
    assert.match(
      getTextContent(findNodeByTestId(tree, 'inspector-management-feedback')!),
      /Cannot remove this wallpaper while it is actively assigned/i,
    );

    // 3. Unassign ID_BETA, open confirmation dialog, verify catalog-only safety notice, test Cancel, then Confirm
    store.setActiveWallpaperIds([]);
    store.requestRemoveWallpaper(ID_BETA);
    tree = shell.render();
    const dialog = findNodeByTestId(tree, 'confirm-remove-dialog');
    assert.ok(dialog);
    assert.match(
      getTextContent(findNodeByTestId(tree, 'confirm-remove-dialog-safety-notice')!),
      /Original wallpaper media files on disk are NOT deleted, moved, or modified/i,
    );

    // Cancel preserves entry
    clickNode(findNodeByTestId(tree, 'confirm-remove-dialog-cancel-btn')!);
    assert.equal(store.getState().pendingRemovalWallpaperId, null);
    assert.equal(store.getState().entries.length, 3);

    // Re-open and confirm removal
    store.requestRemoveWallpaper(ID_BETA);
    await store.confirmRemoveWallpaper();

    assert.equal(store.getState().entries.length, 2);
    assert.equal(store.getState().selectedWallpaperId, null);
    assert.ok(store.getState().focusReturnWallpaperId !== null);

    tree = shell.render();
    assert.equal(findNodeByTestId(tree, 'library-selection-inspector'), null);
    assert.equal(findNodeByTestId(tree, `wallpaper-card-${ID_BETA}`), null);
    const topFeedback = findNodeByTestId(tree, 'library-management-feedback-banner');
    assert.ok(topFeedback);
    assert.match(getTextContent(topFeedback), /Original media files on disk were preserved/i);
  });

  it('§8 Ambiguous persistence failure triggers automatic state reconciliation against authoritative snapshot', async () => {
    const authoritativeEntries = createInitialEntries();
    let snapshotFetchCount = 0;

    const transport: CatalogIpcTransport = {
      async invokeGetLibrarySnapshot() {
        snapshotFetchCount += 1;
        return buildSnapshot(authoritativeEntries);
      },
      async invokeRegisterWallpapers() {
        throw new Error('unused');
      },
      async invokeUpdateMetadata() {
        throw new Error('Persistence failure writing C:\\Users\\Alice\\project.json');
      },
    };

    const store = new CatalogStore(new CatalogIpcClient(transport));
    await store.loadCatalog();
    assert.equal(snapshotFetchCount, 1);

    const result = await store.updateWallpaperMetadata(ID_ALPHA, {
      title: 'Attempted Title',
      description: 'Attempted Description',
      tags: ['Tag1'],
    });
    assert.equal(result, null);
    assert.equal(snapshotFetchCount, 2);
    const feedback = store.getState().managementFeedback;
    assert.ok(feedback);
    assert.equal(feedback.kind, 'error');
    assert.ok(!feedback.message.includes('C:\\Users\\Alice'));
    assert.equal(store.getState().entries[0]?.title, 'Aurora Borealis 4K');
  });
});
