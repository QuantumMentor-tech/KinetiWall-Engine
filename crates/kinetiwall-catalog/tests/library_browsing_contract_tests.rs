use std::fs;
use std::path::{Path, PathBuf};

use kinetiwall_catalog::{
    AssetReference, AssetSelectionSource, AvailabilityState, BatchOverallStatus, CatalogLifecycle,
    LocalJsonCatalogRepository, SelectionOutcome, WallpaperCatalogRepository, WallpaperId,
    WallpaperMetadata, WallpaperRecord, WallpaperRegistrationService, WallpaperSourceKind,
    WallpaperType,
};
use uuid::Uuid;

struct TempTestDir {
    path: PathBuf,
}

impl TempTestDir {
    fn new(prefix: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("kinetiwall_step27_{prefix}_{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp test dir");
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

struct FixedSelector {
    paths: Vec<PathBuf>,
}

impl AssetSelectionSource for FixedSelector {
    fn select_assets(&mut self) -> Result<SelectionOutcome, kinetiwall_catalog::RegistrationError> {
        Ok(SelectionOutcome::Selected(self.paths.clone()))
    }
}

fn mp4_header_bytes() -> Vec<u8> {
    let mut data = vec![0x00, 0x00, 0x00, 0x18];
    data.extend_from_slice(b"ftypmp42");
    data.extend_from_slice(&[0u8; 32]);
    data
}

#[test]
fn library_snapshot_serializes_authoritative_records_availability_and_redacted_issues() {
    let temp = TempTestDir::new("snapshot");
    let lib_root = temp.path().join("wallpapers");
    let media_dir = temp.path().join("media");
    fs::create_dir_all(&media_dir).expect("create media dir");

    let mut repo = LocalJsonCatalogRepository::open(&lib_root).expect("open catalog");

    // 1. Register a real valid MP4 via Step 26 registration service
    let valid_mp4 = media_dir.join("Neon Rain.mp4");
    fs::write(&valid_mp4, mp4_header_bytes()).expect("write mp4");
    let reg_service = WallpaperRegistrationService::new();
    let mut selector = FixedSelector {
        paths: vec![valid_mp4.clone()],
    };
    let reg_report = reg_service
        .select_and_register(&mut repo, &mut selector)
        .expect("register mp4");
    assert_eq!(reg_report.overall_status, BatchOverallStatus::Succeeded);

    // 2. Create a ManagedPackage record whose asset file is missing on disk, with a valid thumbnail_ref
    let missing_id = WallpaperId::new();
    let mut meta = WallpaperMetadata::new("Cyberpunk Atrium");
    meta.description = "Interactive WebGL scene".to_string();
    meta.thumbnail_ref = Some("preview/thumb.png".to_string());
    let missing_record = WallpaperRecord::new(
        missing_id,
        WallpaperType::Web,
        AssetReference {
            source_kind: WallpaperSourceKind::ManagedPackage,
            entrypoint: "index.html".to_string(),
        },
        meta,
        CatalogLifecycle::now(false),
    );
    repo.create(missing_record).expect("create managed record");

    // 3. Snapshot without probing keeps missing_record as Unchecked
    let unprobed = repo
        .library_snapshot(false)
        .expect("unprobed library snapshot");
    assert_eq!(unprobed.entries.len(), 2);
    assert_eq!(unprobed.integrity.valid_record_count, 2);

    // 4. Snapshot with probing updates availability accurately (`Available` + `Missing`)
    let probed = repo
        .library_snapshot(true)
        .expect("probed library snapshot");
    assert_eq!(probed.integrity.available_count, 1);
    assert_eq!(probed.integrity.missing_count, 1);

    let json = serde_json::to_string(&probed).expect("serialize snapshot to JSON");
    // Ensure raw external OS path is never leaked in the serialized snapshot DTO
    assert!(
        !json.contains(valid_mp4.to_string_lossy().as_ref()),
        "CatalogLibrarySnapshot must never leak raw host filesystem paths"
    );
    assert!(json.contains("preview/thumb.png"));
    assert!(json.contains("Interactive WebGL scene"));

    // Verify round-trip deserialization
    let decoded: kinetiwall_catalog::CatalogLibrarySnapshot =
        serde_json::from_str(&json).expect("deserialize snapshot");
    assert_eq!(decoded, probed);
    let missing_summary = decoded
        .entries
        .iter()
        .find(|e| e.id == missing_id)
        .expect("find missing record summary");
    assert_eq!(missing_summary.availability, AvailabilityState::Missing);
    assert_eq!(
        missing_summary.thumbnail_ref.as_deref(),
        Some("preview/thumb.png")
    );
}
