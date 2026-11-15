use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{Duration, Utc};
use kinetiwall_catalog::{
    AssetReference, AvailabilityState, CatalogError, CatalogLifecycle, CatalogManagementError,
    CatalogManagementService, EditableMetadataPatch, LocalJsonCatalogRepository,
    MetadataUpdateOutcome, RegistrationRejectionReason, RelinkOutcome, RelinkRejectionReason,
    RemovalOutcome, StaticRelinkSelectionSource, ValidationError, WallpaperCatalogRepository,
    WallpaperId, WallpaperMetadata, WallpaperRecord, WallpaperSourceKind, WallpaperType,
};
use uuid::Uuid;

struct TempTestDir {
    path: PathBuf,
}

impl TempTestDir {
    fn new(prefix: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("kinetiwall_mgmt_{prefix}_{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create isolated temp dir");
        Self { path: dir }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempTestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn valid_mp4_bytes() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(64);
    bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x18]);
    bytes.extend_from_slice(b"ftypmp42");
    bytes.extend_from_slice(&[0u8; 32]);
    bytes
}

fn valid_html_bytes() -> Vec<u8> {
    b"<!DOCTYPE html><html><head><title>KinetiWall</title></head><body><h1>Scene</h1></body></html>"
        .to_vec()
}

fn write_test_file(dir: &TempTestDir, name: &str, contents: &[u8]) -> PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, contents).expect("write test media file");
    path
}

#[test]
fn metadata_edit_updates_permitted_fields_and_preserves_protected_fields() {
    let library_dir = TempTestDir::new("lib_meta_edit");
    let media_dir = TempTestDir::new("media_meta_edit");
    let video_path = write_test_file(&media_dir, "aurora.mp4", &valid_mp4_bytes());

    let mut repo = LocalJsonCatalogRepository::open(library_dir.path().to_path_buf())
        .expect("open repository");
    let service = CatalogManagementService::new();

    let id = WallpaperId::new();
    let created_at = Utc::now() - Duration::minutes(10);
    let mut metadata = WallpaperMetadata::new("Original Aurora");
    metadata.author = "Studio North".to_string();
    metadata.version = "1.2.0".to_string();
    metadata.license = "CC-BY-4.0".to_string();

    let mut record = WallpaperRecord::new(
        id,
        WallpaperType::Video,
        AssetReference {
            source_kind: WallpaperSourceKind::ExternalDirectFile,
            entrypoint: video_path.to_string_lossy().into_owned(),
        },
        metadata,
        CatalogLifecycle {
            created_at,
            updated_at: created_at,
            is_builtin: false,
        },
    );
    record.availability = AvailabilityState::Available;
    repo.create(record).expect("create initial record");

    let patch = EditableMetadataPatch::new(
        "  Northern Lights 4K  ",
        "  Updated atmospheric time-lapse  ",
        vec![
            " Ambient ".to_string(),
            "Night".to_string(),
            "ambient".to_string(), // case-insensitive duplicate deduplicated
        ],
    )
    .with_expected_updated_at(created_at);

    let outcome = service
        .update_editable_metadata(&mut repo, id, patch)
        .expect("update editable metadata");

    let summary = match outcome {
        MetadataUpdateOutcome::Updated { summary } => summary,
        other => panic!("expected Updated outcome, got {other:?}"),
    };

    assert_eq!(summary.id, id);
    assert_eq!(summary.title, "Northern Lights 4K");
    assert_eq!(summary.description, "Updated atmospheric time-lapse");
    assert_eq!(
        summary.tags,
        vec!["Ambient".to_string(), "Night".to_string()]
    );
    assert_eq!(summary.wallpaper_type, WallpaperType::Video);
    assert_eq!(summary.source_kind, WallpaperSourceKind::ExternalDirectFile);
    assert_eq!(summary.created_at, created_at);
    assert!(summary.updated_at > created_at);

    // Reload from disk to verify persistence and protected fields.
    let reloaded = LocalJsonCatalogRepository::open(library_dir.path().to_path_buf())
        .expect("reopen repository")
        .get(id)
        .expect("query record")
        .expect("record exists");
    assert_eq!(reloaded.metadata.title, "Northern Lights 4K");
    assert_eq!(
        reloaded.metadata.description,
        "Updated atmospheric time-lapse"
    );
    assert_eq!(reloaded.metadata.author, "Studio North");
    assert_eq!(reloaded.metadata.version, "1.2.0");
    assert_eq!(reloaded.metadata.license, "CC-BY-4.0");
    assert_eq!(reloaded.lifecycle.created_at, created_at);
    assert_eq!(
        fs::read(&video_path).expect("read original file"),
        valid_mp4_bytes()
    );
}

#[test]
fn metadata_edit_detects_unchanged_noop_and_rejects_invalid_or_stale_updates() {
    let library_dir = TempTestDir::new("lib_meta_noop");
    let media_dir = TempTestDir::new("media_meta_noop");
    let video_path = write_test_file(&media_dir, "nebula.mp4", &valid_mp4_bytes());

    let mut repo = LocalJsonCatalogRepository::open(library_dir.path().to_path_buf())
        .expect("open repository");
    let service = CatalogManagementService::new();

    let id = WallpaperId::new();
    let created_at = Utc::now() - Duration::minutes(5);
    let mut metadata = WallpaperMetadata::new("Deep Nebula");
    metadata.description = "Space scene".to_string();
    metadata.tags = vec!["Space".to_string()];

    repo.create(WallpaperRecord::new(
        id,
        WallpaperType::Video,
        AssetReference {
            source_kind: WallpaperSourceKind::ExternalDirectFile,
            entrypoint: video_path.to_string_lossy().into_owned(),
        },
        metadata,
        CatalogLifecycle {
            created_at,
            updated_at: created_at,
            is_builtin: false,
        },
    ))
    .expect("create record");

    // 1. Unchanged metadata returns Unchanged without advancing updated_at.
    let unchanged = service
        .update_editable_metadata(
            &mut repo,
            id,
            EditableMetadataPatch::new("  Deep Nebula ", "Space scene", vec!["Space".to_string()]),
        )
        .expect("unchanged update succeeds");
    assert!(matches!(unchanged, MetadataUpdateOutcome::Unchanged { .. }));
    assert_eq!(unchanged.summary().updated_at, created_at);

    // 2. Empty title is rejected and preserves previous state.
    let err = service
        .update_editable_metadata(
            &mut repo,
            id,
            EditableMetadataPatch::new("   ", "Valid description", vec![]),
        )
        .expect_err("empty title must fail");
    assert!(matches!(
        err,
        CatalogManagementError::Catalog(CatalogError::Validation(ValidationError::BlankTitle))
    ));

    // 3. Stale expected_updated_at conflict is rejected cleanly.
    let stale_ts = created_at - Duration::seconds(30);
    let conflict = service
        .update_editable_metadata(
            &mut repo,
            id,
            EditableMetadataPatch::new("Renamed Nebula", "Space scene", vec![])
                .with_expected_updated_at(stale_ts),
        )
        .expect_err("stale expected_updated_at must fail");
    assert!(matches!(
        conflict,
        CatalogManagementError::StaleRecordConflict { .. }
    ));

    // Confirm original title is still intact.
    let record = repo.get(id).expect("get").expect("exists");
    assert_eq!(record.metadata.title, "Deep Nebula");
}

#[test]
fn relink_recovers_missing_asset_and_preserves_old_and_new_files() {
    let library_dir = TempTestDir::new("lib_relink_ok");
    let media_dir = TempTestDir::new("media_relink_ok");
    let old_path = media_dir.path().join("missing_original.mp4");
    let replacement_bytes = valid_mp4_bytes();
    let replacement_path = write_test_file(&media_dir, "relocated_aurora.mp4", &replacement_bytes);

    let mut repo = LocalJsonCatalogRepository::open(library_dir.path().to_path_buf())
        .expect("open repository");
    let service = CatalogManagementService::new();

    let id = WallpaperId::new();
    let mut record = WallpaperRecord::new(
        id,
        WallpaperType::Video,
        AssetReference {
            source_kind: WallpaperSourceKind::ExternalDirectFile,
            entrypoint: old_path.to_string_lossy().into_owned(),
        },
        WallpaperMetadata::new("Missing Aurora"),
        CatalogLifecycle::now(false),
    );
    record.availability = AvailabilityState::Missing;
    repo.create(record).expect("create missing record");

    let mut selector = StaticRelinkSelectionSource::selected(replacement_path.clone());
    let outcome = service
        .select_and_relink_asset(&mut repo, id, &mut selector)
        .expect("relink asset");

    let summary = match outcome {
        RelinkOutcome::Relinked {
            id: out_id,
            summary,
        } => {
            assert_eq!(out_id, id);
            summary
        }
        other => panic!("expected Relinked outcome, got {other:?}"),
    };

    assert_eq!(summary.availability, AvailabilityState::Available);
    let updated_record = repo.get(id).expect("get").expect("record exists");
    assert!(updated_record
        .asset
        .entrypoint
        .ends_with("relocated_aurora.mp4"));

    // Verify replacement file is untouched on disk.
    assert_eq!(
        fs::read(&replacement_path).expect("read replacement file"),
        replacement_bytes
    );
}

#[test]
fn relink_handles_cancellation_type_mismatch_invalid_header_missing_and_duplicate_ownership() {
    let library_dir = TempTestDir::new("lib_relink_edge");
    let media_dir = TempTestDir::new("media_relink_edge");

    let original_video = write_test_file(&media_dir, "original.mp4", &valid_mp4_bytes());
    let other_owned_video = write_test_file(&media_dir, "other_owned.mp4", &valid_mp4_bytes());
    let html_candidate = write_test_file(&media_dir, "scene.html", &valid_html_bytes());
    let spoofed_video = write_test_file(&media_dir, "spoofed.mp4", b"MZ\x90\x00not_a_video");
    let nonexistent_video = media_dir.path().join("does_not_exist.mp4");

    let mut repo = LocalJsonCatalogRepository::open(library_dir.path().to_path_buf())
        .expect("open repository");
    let service = CatalogManagementService::new();

    let target_id = WallpaperId::new();
    let other_id = WallpaperId::new();

    let mut target_record = WallpaperRecord::new(
        target_id,
        WallpaperType::Video,
        AssetReference {
            source_kind: WallpaperSourceKind::ExternalDirectFile,
            entrypoint: original_video.to_string_lossy().into_owned(),
        },
        WallpaperMetadata::new("Target Video"),
        CatalogLifecycle::now(false),
    );
    target_record.availability = AvailabilityState::Available;
    repo.create(target_record).expect("create target record");

    let mut other_record = WallpaperRecord::new(
        other_id,
        WallpaperType::Video,
        AssetReference {
            source_kind: WallpaperSourceKind::ExternalDirectFile,
            entrypoint: other_owned_video.to_string_lossy().into_owned(),
        },
        WallpaperMetadata::new("Other Video"),
        CatalogLifecycle::now(false),
    );
    other_record.availability = AvailabilityState::Available;
    repo.create(other_record).expect("create other record");

    // 1. Cancellation leaves record untouched.
    let mut cancel_selector = StaticRelinkSelectionSource::cancelled();
    let cancelled = service
        .select_and_relink_asset(&mut repo, target_id, &mut cancel_selector)
        .expect("cancelled relink");
    assert_eq!(cancelled, RelinkOutcome::Cancelled { id: target_id });

    // 2. Same file when already Available returns UnchangedSameAsset.
    let same_outcome = service
        .relink_asset_from_path(&mut repo, target_id, &original_video)
        .expect("same asset relink");
    assert!(matches!(
        same_outcome,
        RelinkOutcome::UnchangedSameAsset { id, .. } if id == target_id
    ));

    // 3. Incompatible wallpaper type (Web HTML for a Video record) is rejected.
    let mismatch_outcome = service
        .relink_asset_from_path(&mut repo, target_id, &html_candidate)
        .expect("type mismatch relink");
    assert!(matches!(
        mismatch_outcome,
        RelinkOutcome::Rejected {
            reason: RelinkRejectionReason::TypeMismatch {
                expected_type: WallpaperType::Video,
                candidate_type: WallpaperType::Web,
            },
            ..
        }
    ));

    // 4. Duplicate asset owned by another record is rejected.
    let dup_outcome = service
        .relink_asset_from_path(&mut repo, target_id, &other_owned_video)
        .expect("duplicate owned relink");
    assert!(matches!(
        dup_outcome,
        RelinkOutcome::Rejected {
            reason: RelinkRejectionReason::DuplicateOwnedByOther { existing_id },
            ..
        } if existing_id == other_id
    ));

    // 5. Spoofed MZ header is rejected.
    let spoofed_outcome = service
        .relink_asset_from_path(&mut repo, target_id, &spoofed_video)
        .expect("spoofed header relink");
    assert!(matches!(
        spoofed_outcome,
        RelinkOutcome::Rejected {
            reason: RelinkRejectionReason::AssetValidation(
                RegistrationRejectionReason::InvalidContentSignature { .. }
            ),
            ..
        }
    ));

    // 6. Missing replacement file returns InaccessibleOrMissing and preserves original reference.
    let missing_outcome = service
        .relink_asset_from_path(&mut repo, target_id, &nonexistent_video)
        .expect("missing candidate relink");
    assert!(matches!(
        missing_outcome,
        RelinkOutcome::InaccessibleOrMissing {
            availability: AvailabilityState::Missing,
            ..
        }
    ));

    // Verify original record still points to original_video and remains Available.
    let current = repo.get(target_id).expect("get").expect("exists");
    assert_eq!(current.availability, AvailabilityState::Available);
    assert!(current.asset.entrypoint.ends_with("original.mp4"));
}

#[test]
fn remove_catalog_entry_enforces_active_guard_and_preserves_source_media_on_disk() {
    let library_dir = TempTestDir::new("lib_remove");
    let media_dir = TempTestDir::new("media_remove");
    let media_bytes = valid_mp4_bytes();
    let video_path = write_test_file(&media_dir, "preserve_me.mp4", &media_bytes);

    let mut repo = LocalJsonCatalogRepository::open(library_dir.path().to_path_buf())
        .expect("open repository");
    let service = CatalogManagementService::new();

    let id = WallpaperId::new();
    repo.create(WallpaperRecord::new(
        id,
        WallpaperType::Video,
        AssetReference {
            source_kind: WallpaperSourceKind::ExternalDirectFile,
            entrypoint: video_path.to_string_lossy().into_owned(),
        },
        WallpaperMetadata::new("Preserve Media Wallpaper"),
        CatalogLifecycle::now(false),
    ))
    .expect("create record");

    // 1. Removal is blocked while wallpaper ID is marked active/assigned.
    let mut active_ids = BTreeSet::new();
    active_ids.insert(id);
    let blocked = service
        .remove_catalog_entry(&mut repo, id, &active_ids)
        .expect("attempt active removal");
    assert_eq!(blocked, RemovalOutcome::BlockedActiveAssignment { id });
    assert!(repo.get(id).expect("get").is_some());

    // 2. Once unassigned, catalog removal succeeds and leaves the source file intact on disk.
    active_ids.clear();
    let removed = service
        .remove_catalog_entry(&mut repo, id, &active_ids)
        .expect("remove inactive record");
    assert_eq!(removed, RemovalOutcome::Removed { id });
    assert!(repo.get(id).expect("get").is_none());
    assert!(
        video_path.exists(),
        "source media file must never be deleted during catalog removal"
    );
    assert_eq!(
        fs::read(&video_path).expect("read source file"),
        media_bytes
    );

    // 3. Repeated removal is idempotent.
    let repeated = service
        .remove_catalog_entry(&mut repo, id, &active_ids)
        .expect("repeated removal");
    assert_eq!(repeated, RemovalOutcome::AlreadyRemoved { id });
}
