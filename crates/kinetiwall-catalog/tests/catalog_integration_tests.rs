use std::fs;
use std::path::{Path, PathBuf};

use chrono::Duration;
use kinetiwall_catalog::{
    probe_asset_availability, redact_path_for_diagnostics, resolve_asset_path,
    validate_asset_reference, validate_record, AssetReference, AvailabilityState, CatalogError,
    CatalogLifecycle, LocalJsonCatalogRepository, PathSafetyError, StoredManifestV1,
    ValidationError, WallpaperCatalogRepository, WallpaperId, WallpaperMetadata, WallpaperRecord,
    WallpaperSourceKind, WallpaperType, CURRENT_CATALOG_SCHEMA_VERSION,
};
use uuid::Uuid;

struct TempTestDir {
    path: PathBuf,
}

impl TempTestDir {
    fn new(prefix: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("kinetiwall_{prefix}_{}", Uuid::new_v4()));
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

fn make_managed_video_record(id: WallpaperId, title: &str) -> WallpaperRecord {
    let mut meta = WallpaperMetadata::new(title);
    meta.tags = vec!["neon".to_string(), "60fps".to_string()];
    meta.thumbnail_ref = Some("thumbnail.png".to_string());
    WallpaperRecord::new(
        id,
        WallpaperType::Video,
        AssetReference {
            source_kind: WallpaperSourceKind::ManagedPackage,
            entrypoint: "assets/loop.mp4".to_string(),
        },
        meta,
        CatalogLifecycle::now(false),
    )
}

#[test]
fn domain_model_valid_records_and_stable_identity_across_updates() {
    let temp = TempTestDir::new("stable_id");
    let lib_root = temp.path().join("wallpapers");
    let mut repo = LocalJsonCatalogRepository::open(&lib_root).expect("open repo");

    let id = WallpaperId::new();
    let created = make_managed_video_record(id, "Initial Neon City");
    let created_ts = created.lifecycle.created_at;

    let saved = repo.create(created).expect("create valid record");
    assert_eq!(saved.id, id);
    assert_eq!(saved.schema_version, CURRENT_CATALOG_SCHEMA_VERSION);
    assert_eq!(saved.availability, AvailabilityState::Unchecked);

    let mut updated_meta = saved.metadata.clone();
    updated_meta.title = "Renamed Neon City (4K)".to_string();
    updated_meta.description = "Updated description with safe <script> text".to_string();
    let updated_ts = created_ts + Duration::seconds(10);

    let updated = repo
        .update_metadata(id, updated_meta, updated_ts)
        .expect("update metadata");

    assert_eq!(
        updated.id, id,
        "WallpaperId must remain stable across title changes"
    );
    assert_eq!(updated.lifecycle.created_at, created_ts);
    assert_eq!(updated.lifecycle.updated_at, updated_ts);
    assert_eq!(updated.metadata.title, "Renamed Neon City (4K)");

    let summary = updated.to_summary();
    assert_eq!(summary.id, id);
    assert_eq!(summary.title, "Renamed Neon City (4K)");
    assert!(summary.has_thumbnail_ref);
}

#[test]
fn validation_rejects_invalid_metadata_types_versions_and_lifecycle() {
    let base = make_managed_video_record(WallpaperId::new(), "Valid Title");
    assert!(validate_record(&base).is_ok());

    // Blank title
    let mut blank = base.clone();
    blank.metadata.title = "   \t ".to_string();
    assert!(matches!(
        validate_record(&blank),
        Err(CatalogError::Validation(ValidationError::BlankTitle))
    ));

    // Overlong title
    let mut long_title = base.clone();
    long_title.metadata.title = "a".repeat(129);
    assert!(matches!(
        validate_record(&long_title),
        Err(CatalogError::Validation(
            ValidationError::TitleTooLong { .. }
        ))
    ));

    // Control char in title
    let mut ctrl_title = base.clone();
    ctrl_title.metadata.title = "Hello\nWorld".to_string();
    assert!(matches!(
        validate_record(&ctrl_title),
        Err(CatalogError::Validation(
            ValidationError::TitleControlCharacters
        ))
    ));

    // Blank author & invalid SemVer
    let mut bad_author = base.clone();
    bad_author.metadata.author = "  ".to_string();
    assert!(matches!(
        validate_record(&bad_author),
        Err(CatalogError::Validation(ValidationError::BlankAuthor))
    ));

    let mut bad_semver = base.clone();
    bad_semver.metadata.version = "1.0".to_string();
    assert!(matches!(
        validate_record(&bad_semver),
        Err(CatalogError::Validation(
            ValidationError::InvalidSemVer { .. }
        ))
    ));

    // Too many tags
    let mut many_tags = base.clone();
    many_tags.metadata.tags = (0..33).map(|i| format!("tag{i}")).collect();
    assert!(matches!(
        validate_record(&many_tags),
        Err(CatalogError::Validation(
            ValidationError::TooManyTags { .. }
        ))
    ));

    // Updated precedes created
    let mut bad_time = base.clone();
    bad_time.lifecycle.updated_at = bad_time.lifecycle.created_at - Duration::seconds(1);
    assert!(matches!(
        validate_record(&bad_time),
        Err(CatalogError::Validation(
            ValidationError::InvalidLifecycleOrder { .. }
        ))
    ));

    // Unsupported schema version
    let mut bad_schema = base.clone();
    bad_schema.schema_version = 2;
    assert!(matches!(
        validate_record(&bad_schema),
        Err(CatalogError::UnsupportedSchemaVersion {
            found: 2,
            supported: 1
        })
    ));

    // Mismatched asset extension vs wallpaper type
    let mut web_mismatch = base.clone();
    web_mismatch.wallpaper_type = WallpaperType::Web;
    assert!(matches!(
        validate_record(&web_mismatch),
        Err(CatalogError::Validation(
            ValidationError::IncompatibleAssetType { .. }
        ))
    ));

    let mut video_mismatch = base;
    video_mismatch.asset.entrypoint = "index.html".to_string();
    assert!(matches!(
        validate_record(&video_mismatch),
        Err(CatalogError::Validation(
            ValidationError::IncompatibleAssetType { .. }
        ))
    ));
}

#[test]
fn serialization_rejects_unknown_wallpaper_type_and_preserves_extra_fields() {
    let bad_type_json = r#"{
        "manifest_version": 1,
        "id": "550e8400-e29b-41d4-a716-446655440000",
        "title": "Unknown Engine",
        "type": "executable",
        "entrypoint": "payload.exe",
        "created_at": "2026-11-12T10:00:00Z",
        "updated_at": "2026-11-12T10:00:00Z"
    }"#;
    let parsed: Result<StoredManifestV1, _> = serde_json::from_str(bad_type_json);
    assert!(
        parsed.is_err(),
        "unsupported type must not coerce into valid WallpaperType"
    );

    let forward_compat_json = r#"{
        "manifest_version": 1,
        "id": "550e8400-e29b-41d4-a716-446655440000",
        "title": "Interactive Clock",
        "type": "web",
        "source_kind": "managed_package",
        "entrypoint": "index.html",
        "created_at": "2026-11-12T10:00:00Z",
        "updated_at": "2026-11-12T10:00:00Z",
        "permissions": { "webgl": true }
    }"#;
    let manifest: StoredManifestV1 =
        serde_json::from_str(forward_compat_json).expect("parse manifest v1");
    assert_eq!(manifest.wallpaper_type, WallpaperType::Web);
    assert!(manifest.extra_fields.contains_key("permissions"));

    let record = WallpaperRecord::from(manifest);
    assert!(validate_record(&record).is_ok());
    let reserialized = StoredManifestV1::from(&record);
    assert!(reserialized.extra_fields.contains_key("permissions"));
}

#[test]
fn persistence_crud_reopen_duplicate_handling_and_corrupt_manifest_isolation() {
    let temp = TempTestDir::new("persistence");
    let lib_root = temp.path().join("wallpapers");

    let id1 = WallpaperId::new();
    let id2 = WallpaperId::new();

    {
        let mut repo = LocalJsonCatalogRepository::open(&lib_root).expect("open initial repo");
        repo.create(make_managed_video_record(id1, "First Wallpaper"))
            .expect("create first");

        // Duplicate ID must fail
        let dup = repo.create(make_managed_video_record(id1, "Duplicate Wallpaper"));
        assert!(matches!(dup, Err(CatalogError::DuplicateId(x)) if x == id1));

        let web_record = WallpaperRecord::new(
            id2,
            WallpaperType::Web,
            AssetReference {
                source_kind: WallpaperSourceKind::ManagedPackage,
                entrypoint: "index.html".to_string(),
            },
            WallpaperMetadata::new("Second Web Wallpaper"),
            CatalogLifecycle::now(false),
        );
        repo.create(web_record).expect("create second");
        assert_eq!(repo.list().expect("list").len(), 2);
    }

    // Add a corrupted sibling manifest and an unsupported V99 manifest on disk
    let corrupt_id = WallpaperId::new();
    let corrupt_dir = lib_root.join(corrupt_id.to_string());
    fs::create_dir_all(&corrupt_dir).expect("create corrupt dir");
    let corrupt_manifest = corrupt_dir.join("project.json");
    fs::write(&corrupt_manifest, "{ not valid json !!!").expect("write corrupt manifest");

    let future_id = WallpaperId::new();
    let future_dir = lib_root.join(future_id.to_string());
    fs::create_dir_all(&future_dir).expect("create future dir");
    let future_manifest = future_dir.join("project.json");
    fs::write(
        &future_manifest,
        format!(r#"{{"manifest_version": 99, "id": "{future_id}"}}"#),
    )
    .expect("write future manifest");

    // Reopen repository simulating application restart
    let reopened = LocalJsonCatalogRepository::open(&lib_root).expect("reopen repo");
    let items = reopened.list().expect("list valid items");
    assert_eq!(
        items.len(),
        2,
        "valid entries must survive broken sibling manifests"
    );
    assert!(reopened.get(id1).expect("get id1").is_some());
    assert!(reopened.get(id2).expect("get id2").is_some());
    assert_eq!(reopened.load_issues().len(), 2);

    // Corrupted files must NOT be silently deleted
    assert!(corrupt_manifest.exists());
    assert!(future_manifest.exists());

    let report = reopened.integrity_report();
    assert_eq!(report.valid_record_count, 2);
    assert_eq!(report.load_issue_count, 2);
    assert_eq!(report.unchecked_count, 2);

    // Mismatched directory ID vs manifest ID is isolated as a load issue
    let dir_uuid = WallpaperId::new();
    let other_uuid = WallpaperId::new();
    let mismatch_dir = lib_root.join(dir_uuid.to_string());
    fs::create_dir_all(&mismatch_dir).expect("create mismatch dir");
    fs::write(
        mismatch_dir.join("project.json"),
        format!(
            r#"{{
                "manifest_version": 1,
                "id": "{other_uuid}",
                "title": "Mismatched Folder",
                "type": "video",
                "source_kind": "managed_package",
                "entrypoint": "loop.mp4",
                "created_at": "2026-11-12T10:00:00Z",
                "updated_at": "2026-11-12T10:00:00Z"
            }}"#
        ),
    )
    .expect("write mismatched manifest");

    let mut reopened2 = LocalJsonCatalogRepository::open(&lib_root).expect("reopen with mismatch");
    assert_eq!(reopened2.load_issues().len(), 3);
    assert_eq!(reopened2.list().expect("list").len(), 2);

    // NotFound handling on update, check_availability, and remove_record
    let missing_id = WallpaperId::new();
    assert!(matches!(
        reopened2.update_metadata(missing_id, WallpaperMetadata::new("X"), chrono::Utc::now()),
        Err(CatalogError::NotFound(x)) if x == missing_id
    ));
    assert!(matches!(
        reopened2.check_availability(missing_id),
        Err(CatalogError::NotFound(x)) if x == missing_id
    ));
    assert!(!reopened2
        .remove_record(missing_id)
        .expect("remove non-existent"));
}

#[test]
fn invalid_thumbnail_refs_and_passive_untrusted_metadata_handling() {
    let mut record =
        make_managed_video_record(WallpaperId::new(), "$(calc.exe) <script>alert(1)</script>");
    record.metadata.thumbnail_ref = Some("../escape.png".to_string());
    assert!(matches!(
        validate_record(&record),
        Err(CatalogError::Validation(
            ValidationError::InvalidThumbnailRef { .. }
        ))
    ));

    record.metadata.thumbnail_ref = Some("thumb.exe".to_string());
    assert!(matches!(
        validate_record(&record),
        Err(CatalogError::Validation(
            ValidationError::InvalidThumbnailRef { .. }
        ))
    ));

    // Passive untrusted strings in title/tags remain plain serialized data
    record.metadata.thumbnail_ref = Some("preview/thumb.png".to_string());
    assert!(validate_record(&record).is_ok());
}

#[test]
fn open_fails_safely_if_library_root_is_a_file() {
    let temp = TempTestDir::new("open_fail");
    let bad_root = temp.path().join("not_a_directory");
    fs::write(&bad_root, b"occupied").expect("create regular file");

    let res = LocalJsonCatalogRepository::open(&bad_root);
    assert!(matches!(res, Err(CatalogError::Persistence(_))));
}

#[test]
fn path_safety_windows_unc_unicode_spaces_and_forbidden_extensions() {
    // Valid Windows drive path with spaces and Unicode
    let win_unicode = AssetReference {
        source_kind: WallpaperSourceKind::ExternalDirectFile,
        entrypoint: r"D:\Wallpapers Collection\日本語 テスト — scene (4K).mp4".to_string(),
    };
    assert!(validate_asset_reference(&win_unicode).is_ok());

    // Valid UNC network path
    let unc = AssetReference {
        source_kind: WallpaperSourceKind::ExternalDirectFile,
        entrypoint: r"\\nas-media\share\4k_loops\aurora.webm".to_string(),
    };
    assert!(validate_asset_reference(&unc).is_ok());

    // Reject path traversal in managed package
    let traversal = AssetReference {
        source_kind: WallpaperSourceKind::ManagedPackage,
        entrypoint: r"..\..\Windows\System32\evil.mp4".to_string(),
    };
    assert!(matches!(
        validate_asset_reference(&traversal),
        Err(PathSafetyError::TraversalDetected)
    ));

    // Reject absolute path in managed package
    let abs_managed = AssetReference {
        source_kind: WallpaperSourceKind::ManagedPackage,
        entrypoint: r"C:\Users\alice\video.mp4".to_string(),
    };
    assert!(matches!(
        validate_asset_reference(&abs_managed),
        Err(PathSafetyError::ManagedPathMustBeRelative)
    ));

    // Reject relative path in external asset
    let rel_external = AssetReference {
        source_kind: WallpaperSourceKind::ExternalDirectFile,
        entrypoint: r"videos\clip.mp4".to_string(),
    };
    assert!(matches!(
        validate_asset_reference(&rel_external),
        Err(PathSafetyError::ExternalPathMustBeAbsolute)
    ));

    // Reject NUL byte
    let nul_path = AssetReference {
        source_kind: WallpaperSourceKind::ManagedPackage,
        entrypoint: "assets/loop\0.mp4".to_string(),
    };
    assert!(matches!(
        validate_asset_reference(&nul_path),
        Err(PathSafetyError::NullByte)
    ));

    // Reject forbidden executable extensions
    for bad_ext in [
        "evil.exe",
        "script.ps1",
        "batch.bat",
        "dropper.js",
        "shortcut.lnk",
    ] {
        let bad = AssetReference {
            source_kind: WallpaperSourceKind::ManagedPackage,
            entrypoint: bad_ext.to_string(),
        };
        assert!(
            matches!(
                validate_asset_reference(&bad),
                Err(PathSafetyError::ForbiddenExtension { .. })
            ),
            "expected forbidden extension rejection for {bad_ext}"
        );
    }

    // Diagnostic redaction hides usernames/drive prefixes
    let redacted_ext =
        redact_path_for_diagnostics(Path::new(r"C:\Users\SecretUser\Videos\clip.mp4"));
    assert!(!redacted_ext.contains("SecretUser"));
    assert!(redacted_ext.contains("[REDACTED]"));

    let redacted_cat = redact_path_for_diagnostics(Path::new(
        r"C:\Users\SecretUser\AppData\Local\KinetiWall\wallpapers\1234\project.json",
    ));
    assert!(!redacted_cat.contains("SecretUser"));
    assert!(redacted_cat.starts_with("[CATALOG_ROOT]\\wallpapers"));
}

#[test]
fn availability_checks_and_external_file_ownership_protection() {
    let temp = TempTestDir::new("ownership");
    let lib_root = temp.path().join("wallpapers");
    let user_media_dir = temp.path().join("user_videos");
    fs::create_dir_all(&user_media_dir).expect("create user media dir");

    let media_file = user_media_dir.join("space clip (日本語).mp4");
    let original_bytes = b"FAKE_MP4_USER_OWNED_MEDIA_PAYLOAD";
    fs::write(&media_file, original_bytes).expect("write user media file");

    let mut repo = LocalJsonCatalogRepository::open(&lib_root).expect("open repo");
    let id = WallpaperId::new();
    let record = WallpaperRecord::new(
        id,
        WallpaperType::Video,
        AssetReference {
            source_kind: WallpaperSourceKind::ExternalDirectFile,
            entrypoint: media_file.to_string_lossy().into_owned(),
        },
        WallpaperMetadata::new("User Owned Video"),
        CatalogLifecycle::now(false),
    );
    repo.create(record).expect("create external record");

    // Probe reports Available
    let state = repo.check_availability(id).expect("check availability");
    assert_eq!(state, AvailabilityState::Available);
    assert!(state.is_playable_candidate());

    // Directory instead of file reports InvalidAsset
    let dir_asset = AssetReference {
        source_kind: WallpaperSourceKind::ExternalDirectFile,
        entrypoint: user_media_dir.to_string_lossy().into_owned(),
    };
    assert!(matches!(
        probe_asset_availability(&lib_root, id, &dir_asset),
        AvailabilityState::InvalidAsset { .. }
    ));

    // Removing the catalog record MUST NOT delete or modify the user's file!
    assert!(repo.remove_record(id).expect("remove record"));
    assert!(repo.get(id).expect("get after remove").is_none());
    assert!(
        media_file.exists(),
        "external user media file must NEVER be deleted by catalog removal"
    );
    assert_eq!(
        fs::read(&media_file).expect("read user media file"),
        original_bytes,
        "external user media file contents must NEVER be modified"
    );

    // Also test missing asset preservation: missing file stays in catalog as Missing
    let missing_path = user_media_dir.join("moved_away.mp4");
    let id2 = WallpaperId::new();
    let record2 = WallpaperRecord::new(
        id2,
        WallpaperType::Video,
        AssetReference {
            source_kind: WallpaperSourceKind::ExternalDirectFile,
            entrypoint: missing_path.to_string_lossy().into_owned(),
        },
        WallpaperMetadata::new("Temporarily Disconnected Drive Clip"),
        CatalogLifecycle::now(false),
    );
    repo.create(record2)
        .expect("create record for missing path");

    let missing_state = repo.check_availability(id2).expect("check missing");
    assert_eq!(missing_state, AvailabilityState::Missing);
    assert!(!missing_state.is_playable_candidate());
    assert!(
        repo.get(id2).expect("get missing record").is_some(),
        "catalog metadata must be preserved when asset is missing"
    );

    // Managed package asset payload also remains untouched by remove_record
    let id3 = WallpaperId::new();
    let managed = make_managed_video_record(id3, "Managed Package Entry");
    repo.create(managed.clone()).expect("create managed record");
    let resolved_managed =
        resolve_asset_path(&lib_root, id3, &managed.asset).expect("resolve managed");
    fs::create_dir_all(resolved_managed.parent().unwrap()).expect("create assets dir");
    fs::write(&resolved_managed, b"MANAGED_ASSET_BYTES").expect("write managed asset");
    assert_eq!(
        repo.check_availability(id3).expect("check managed"),
        AvailabilityState::Available
    );
    assert!(repo
        .remove_record(id3)
        .expect("remove managed catalog record"));
    assert!(
        resolved_managed.exists(),
        "record removal must remain separate from asset file deletion"
    );
}

#[cfg(windows)]
#[test]
fn windows_exclusively_locked_asset_reports_inaccessible() {
    use std::os::windows::fs::OpenOptionsExt;

    let temp = TempTestDir::new("locked_win");
    let lib_root = temp.path().join("wallpapers");
    let media_dir = temp.path().join("locked_media");
    fs::create_dir_all(&media_dir).expect("create media dir");
    let locked_file = media_dir.join("exclusively_locked.mp4");
    fs::write(&locked_file, b"LOCKED_BYTES").expect("write locked file");

    let mut repo = LocalJsonCatalogRepository::open(&lib_root).expect("open repo");
    let id = WallpaperId::new();
    let record = WallpaperRecord::new(
        id,
        WallpaperType::Video,
        AssetReference {
            source_kind: WallpaperSourceKind::ExternalDirectFile,
            entrypoint: locked_file.to_string_lossy().into_owned(),
        },
        WallpaperMetadata::new("Locked Video"),
        CatalogLifecycle::now(false),
    );
    repo.create(record).expect("create record");

    // Hold an exclusive Windows handle (share_mode = 0) so read-only probing sees ERROR_SHARING_VIOLATION.
    let _exclusive_handle = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(&locked_file)
        .expect("acquire exclusive handle");

    let state = repo.check_availability(id).expect("check availability");
    assert!(
        matches!(state, AvailabilityState::Inaccessible { .. }),
        "expected Inaccessible for exclusively locked file, got {state:?}"
    );
    assert!(!state.is_playable_candidate());
}
