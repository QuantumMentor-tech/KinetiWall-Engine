use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use kinetiwall_catalog::{
    derive_display_title, AssetReference, AssetSelectionSource, AvailabilityState,
    BatchOverallStatus, CatalogError, LocalJsonCatalogRepository, PersistenceError,
    RegistrationBatchReport, RegistrationError, RegistrationItemOutcome,
    RegistrationRejectionReason, SelectionOutcome, WallpaperCatalogRepository, WallpaperId,
    WallpaperMetadata, WallpaperRecord, WallpaperRegistrationService, WallpaperSourceKind,
    WallpaperType,
};
use uuid::Uuid;

struct TempTestDir {
    path: PathBuf,
}

impl TempTestDir {
    fn new(prefix: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("kinetiwall_reg_{prefix}_{}", Uuid::new_v4()));
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

struct MockSelector {
    outcome: SelectionOutcome,
}

impl AssetSelectionSource for MockSelector {
    fn select_assets(&mut self) -> Result<SelectionOutcome, RegistrationError> {
        Ok(self.outcome.clone())
    }
}

fn mp4_header_bytes() -> Vec<u8> {
    let mut data = vec![0x00, 0x00, 0x00, 0x18];
    data.extend_from_slice(b"ftypmp42");
    data.extend_from_slice(&[0u8; 32]);
    data
}

fn webm_header_bytes() -> Vec<u8> {
    vec![0x1A, 0x45, 0xDF, 0xA3, 0x9F, 0x42, 0x86, 0x81, 0x01]
}

fn avi_header_bytes() -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(b"RIFF");
    data.extend_from_slice(&[0x00, 0x10, 0x00, 0x00]);
    data.extend_from_slice(b"AVI ");
    data.extend_from_slice(b"LIST");
    data
}

#[test]
fn valid_video_and_web_assets_register_and_persist_without_modifying_sources() {
    let temp = TempTestDir::new("valid_all");
    let lib_root = temp.path().join("wallpapers");
    let user_dir = temp.path().join("User Media");
    fs::create_dir_all(&user_dir).expect("create user dir");

    let mp4_path = user_dir.join("Neon Horizon (日本語).mp4");
    let webm_path = user_dir.join("Aurora Loop.webm");
    let avi_path = user_dir.join("Retro Grid.avi");
    let html_path = user_dir.join("Interactive Clock.html");

    let mp4_bytes = mp4_header_bytes();
    let webm_bytes = webm_header_bytes();
    let avi_bytes = avi_header_bytes();
    let html_bytes = b"<!DOCTYPE html><html><body><canvas></canvas></body></html>".to_vec();

    fs::write(&mp4_path, &mp4_bytes).expect("write mp4");
    fs::write(&webm_path, &webm_bytes).expect("write webm");
    fs::write(&avi_path, &avi_bytes).expect("write avi");
    fs::write(&html_path, &html_bytes).expect("write html");

    let service = WallpaperRegistrationService::new();
    let mut repo = LocalJsonCatalogRepository::open(&lib_root).expect("open repo");
    let mut selector = MockSelector {
        outcome: SelectionOutcome::Selected(vec![
            mp4_path.clone(),
            webm_path.clone(),
            avi_path.clone(),
            html_path.clone(),
        ]),
    };

    let report = service
        .select_and_register(&mut repo, &mut selector)
        .expect("register valid selection");

    assert_eq!(report.overall_status, BatchOverallStatus::Succeeded);
    assert_eq!(report.total_selected, 4);
    assert_eq!(report.registered_count, 4);
    assert_eq!(report.rejected_count, 0);
    assert_eq!(report.failed_count, 0);
    assert!(!service.is_in_flight());

    // Verify types & persistence across reopen
    let reopened = LocalJsonCatalogRepository::open(&lib_root).expect("reopen repo");
    let items = reopened.list().expect("list persisted");
    assert_eq!(items.len(), 4);
    assert!(items.iter().any(|r| {
        r.wallpaper_type == WallpaperType::Video
            && r.asset.source_kind == WallpaperSourceKind::ExternalDirectFile
            && r.metadata.title == "Neon Horizon (日本語)"
    }));
    assert!(items.iter().any(|r| {
        r.wallpaper_type == WallpaperType::Web && r.metadata.title == "Interactive Clock"
    }));

    // Source files remain 100% unmodified on disk
    assert_eq!(fs::read(&mp4_path).unwrap(), mp4_bytes);
    assert_eq!(fs::read(&webm_path).unwrap(), webm_bytes);
    assert_eq!(fs::read(&avi_path).unwrap(), avi_bytes);
    assert_eq!(fs::read(&html_path).unwrap(), html_bytes);
}

#[test]
fn cancellation_creates_no_records_and_clears_in_flight_state() {
    let temp = TempTestDir::new("cancel");
    let lib_root = temp.path().join("wallpapers");
    let mut repo = LocalJsonCatalogRepository::open(&lib_root).expect("open repo");
    let service = WallpaperRegistrationService::new();
    let mut selector = MockSelector {
        outcome: SelectionOutcome::Cancelled,
    };

    let report = service
        .select_and_register(&mut repo, &mut selector)
        .expect("cancel must not error");

    assert_eq!(report.overall_status, BatchOverallStatus::Cancelled);
    assert_eq!(report.total_selected, 0);
    assert_eq!(report.registered_count, 0);
    assert!(report.items.is_empty());
    assert!(!service.is_in_flight());
    assert!(repo.list().expect("list").is_empty());
}

#[test]
fn rejects_spoofed_headers_empty_files_unsupported_extensions_and_directories() {
    let temp = TempTestDir::new("rejections");
    let lib_root = temp.path().join("wallpapers");
    let user_dir = temp.path().join("inputs");
    let nested_dir = user_dir.join("some_folder");
    fs::create_dir_all(&nested_dir).expect("create nested dir");

    let fake_mp4 = user_dir.join("disguised_exe.mp4");
    fs::write(&fake_mp4, b"MZ\x90\x00ThisIsAnExecutable").expect("write fake mp4");

    let text_as_mp4 = user_dir.join("plain_text.mp4");
    fs::write(&text_as_mp4, b"Not a real MP4 container").expect("write text mp4");

    let empty_webm = user_dir.join("empty.webm");
    fs::write(&empty_webm, b"").expect("write empty webm");

    let binary_html = user_dir.join("corrupt.html");
    fs::write(&binary_html, b"\x00\x01\x02\xFF\xFE").expect("write binary html");

    let unsupported_txt = user_dir.join("notes.txt");
    fs::write(&unsupported_txt, b"hello").expect("write txt");

    let forbidden_ps1 = user_dir.join("script.ps1");
    fs::write(&forbidden_ps1, b"Write-Host 'hi'").expect("write ps1");

    let mut repo = LocalJsonCatalogRepository::open(&lib_root).expect("open repo");
    let service = WallpaperRegistrationService::new();

    let report = service
        .register_selection(
            &mut repo,
            SelectionOutcome::Selected(vec![
                fake_mp4,
                text_as_mp4,
                empty_webm,
                binary_html,
                unsupported_txt,
                forbidden_ps1,
                nested_dir,
            ]),
        )
        .expect("run batch");

    assert_eq!(report.overall_status, BatchOverallStatus::AllFailed);
    assert_eq!(report.registered_count, 0);
    assert_eq!(report.rejected_count, 7);
    assert!(repo.list().expect("list").is_empty());

    assert!(matches!(
        &report.items[0],
        RegistrationItemOutcome::Rejected {
            reason: RegistrationRejectionReason::InvalidContentSignature {
                expected_type: WallpaperType::Video,
                ..
            },
            ..
        }
    ));
    assert!(matches!(
        &report.items[2],
        RegistrationItemOutcome::Rejected {
            reason: RegistrationRejectionReason::EmptyFile,
            ..
        }
    ));
    assert!(matches!(
        &report.items[4],
        RegistrationItemOutcome::Rejected {
            reason: RegistrationRejectionReason::UnsupportedExtension,
            ..
        }
    ));
    assert!(matches!(
        &report.items[6],
        RegistrationItemOutcome::Rejected {
            reason: RegistrationRejectionReason::UnexpectedDirectory,
            ..
        }
    ));
}

#[test]
fn duplicate_detection_preserves_existing_id_and_metadata_across_batches_and_within_batch() {
    let temp = TempTestDir::new("duplicates");
    let lib_root = temp.path().join("wallpapers");
    let media_dir = temp.path().join("media");
    fs::create_dir_all(&media_dir).expect("create media dir");

    let mp4_path = media_dir.join("loop.mp4");
    fs::write(&mp4_path, mp4_header_bytes()).expect("write mp4");

    let mut repo = LocalJsonCatalogRepository::open(&lib_root).expect("open repo");
    let service = WallpaperRegistrationService::new();

    // Batch 1: same file selected twice in a single batch
    let first_report = service
        .register_selection(
            &mut repo,
            SelectionOutcome::Selected(vec![mp4_path.clone(), mp4_path.clone()]),
        )
        .expect("first batch");

    assert_eq!(first_report.registered_count, 1);
    assert_eq!(first_report.already_present_count, 1);
    assert_eq!(first_report.overall_status, BatchOverallStatus::Succeeded);

    let first_id = match &first_report.items[0] {
        RegistrationItemOutcome::Registered { id, .. } => *id,
        other => panic!("expected Registered, got {other:?}"),
    };

    // User customizes title in catalog
    let mut custom_meta = WallpaperMetadata::new("My Custom Title");
    custom_meta.tags = vec!["favorite".to_string()];
    repo.update_metadata(first_id, custom_meta, Utc::now())
        .expect("update title");

    // Batch 2: re-selecting same file must report AlreadyPresent and NOT overwrite custom title
    let second_report = service
        .register_selection(
            &mut repo,
            SelectionOutcome::Selected(vec![mp4_path.clone()]),
        )
        .expect("second batch");

    assert_eq!(second_report.registered_count, 0);
    assert_eq!(second_report.already_present_count, 1);
    match &second_report.items[0] {
        RegistrationItemOutcome::AlreadyPresent {
            existing_id,
            summary,
        } => {
            assert_eq!(*existing_id, first_id);
            assert_eq!(summary.title, "My Custom Title");
        }
        other => panic!("expected AlreadyPresent, got {other:?}"),
    }
    assert_eq!(repo.list().expect("list").len(), 1);
}

#[test]
fn partial_batch_failure_toctou_missing_and_json_report_redaction() {
    let temp = TempTestDir::new("partial");
    let lib_root = temp.path().join("wallpapers");
    let user_dir = temp.path().join("SecretUserFolder");
    fs::create_dir_all(&user_dir).expect("create user dir");

    let valid_mp4 = user_dir.join("valid_scene.mp4");
    fs::write(&valid_mp4, mp4_header_bytes()).expect("write valid mp4");

    // File existed during selection, then disappeared before registration (TOCTOU)
    let disappeared_mp4 = user_dir.join("disappeared.mp4");
    let bad_ext = user_dir.join("malware.exe");

    let mut repo = LocalJsonCatalogRepository::open(&lib_root).expect("open repo");
    let service = WallpaperRegistrationService::new();

    let report = service
        .register_selection(
            &mut repo,
            SelectionOutcome::Selected(vec![disappeared_mp4, valid_mp4.clone(), bad_ext]),
        )
        .expect("run mixed batch");

    assert_eq!(report.overall_status, BatchOverallStatus::PartialSuccess);
    assert_eq!(report.registered_count, 1);
    assert_eq!(report.inaccessible_count, 1);
    assert_eq!(report.rejected_count, 1);

    assert!(matches!(
        &report.items[0],
        RegistrationItemOutcome::InaccessibleOrMissing {
            availability: AvailabilityState::Missing,
            ..
        }
    ));

    // Verify serialized report never exposes full host directory prefix
    let serialized = serde_json::to_string(&report).expect("serialize batch report");
    assert!(!serialized.contains(&temp.path().to_string_lossy().to_string()));
    assert_eq!(
        derive_display_title(Path::new(r"C:\Users\Alice\Videos\  My Scene  .mp4")),
        "My Scene"
    );
}

#[test]
fn concurrent_submission_guard_blocks_overlapping_registration() {
    struct ReentrantSelector<'a> {
        service: &'a WallpaperRegistrationService,
        repo: &'a mut LocalJsonCatalogRepository,
    }

    impl AssetSelectionSource for ReentrantSelector<'_> {
        fn select_assets(&mut self) -> Result<SelectionOutcome, RegistrationError> {
            // Simulate rapid second trigger while first selection is still active
            let nested = self
                .service
                .register_selection(self.repo, SelectionOutcome::Cancelled);
            assert!(matches!(
                nested,
                Err(RegistrationError::OperationInProgress)
            ));
            Ok(SelectionOutcome::Cancelled)
        }
    }

    let temp = TempTestDir::new("reentrant");
    let lib_root = temp.path().join("wallpapers");
    let mut repo = LocalJsonCatalogRepository::open(&lib_root).expect("open repo");
    let mut selector_repo =
        LocalJsonCatalogRepository::open(&lib_root).expect("open second handle");
    let service = WallpaperRegistrationService::new();
    let mut selector = ReentrantSelector {
        service: &service,
        repo: &mut selector_repo,
    };

    let report: RegistrationBatchReport = service
        .select_and_register(&mut repo, &mut selector)
        .expect("outer workflow finishes cleanly");
    assert_eq!(report.overall_status, BatchOverallStatus::Cancelled);
    assert!(!service.is_in_flight());
}

#[test]
fn persistence_failure_is_reported_honestly_without_corrupting_valid_siblings() {
    struct FailingOnSecondCreateRepo {
        inner: LocalJsonCatalogRepository,
        creates_seen: usize,
    }

    impl WallpaperCatalogRepository for FailingOnSecondCreateRepo {
        fn create(&mut self, record: WallpaperRecord) -> Result<WallpaperRecord, CatalogError> {
            self.creates_seen += 1;
            if self.creates_seen == 2 {
                return Err(CatalogError::Persistence(PersistenceError::WriteFile {
                    redacted_path: "[CATALOG_ROOT]\\project.json".to_string(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "simulated disk write failure",
                    ),
                }));
            }
            self.inner.create(record)
        }

        fn get(&self, id: WallpaperId) -> Result<Option<WallpaperRecord>, CatalogError> {
            self.inner.get(id)
        }

        fn list(&self) -> Result<Vec<WallpaperRecord>, CatalogError> {
            self.inner.list()
        }

        fn update_metadata(
            &mut self,
            id: WallpaperId,
            metadata: WallpaperMetadata,
            updated_at: DateTime<Utc>,
        ) -> Result<WallpaperRecord, CatalogError> {
            self.inner.update_metadata(id, metadata, updated_at)
        }

        fn relink_asset(
            &mut self,
            id: WallpaperId,
            new_asset: AssetReference,
            availability: AvailabilityState,
            updated_at: DateTime<Utc>,
        ) -> Result<WallpaperRecord, CatalogError> {
            self.inner
                .relink_asset(id, new_asset, availability, updated_at)
        }

        fn remove_record(&mut self, id: WallpaperId) -> Result<bool, CatalogError> {
            self.inner.remove_record(id)
        }

        fn check_availability(
            &mut self,
            id: WallpaperId,
        ) -> Result<AvailabilityState, CatalogError> {
            self.inner.check_availability(id)
        }
    }

    let temp = TempTestDir::new("persist_fail");
    let lib_root = temp.path().join("wallpapers");
    let media_dir = temp.path().join("media");
    fs::create_dir_all(&media_dir).expect("create media dir");

    let f1 = media_dir.join("first.mp4");
    let f2 = media_dir.join("second.mp4");
    fs::write(&f1, mp4_header_bytes()).expect("write f1");
    fs::write(&f2, mp4_header_bytes()).expect("write f2");

    let mut repo = FailingOnSecondCreateRepo {
        inner: LocalJsonCatalogRepository::open(&lib_root).expect("open repo"),
        creates_seen: 0,
    };
    let service = WallpaperRegistrationService::new();
    let report = service
        .register_selection(&mut repo, SelectionOutcome::Selected(vec![f1, f2]))
        .expect("batch completes with partial failure");

    assert_eq!(report.overall_status, BatchOverallStatus::PartialSuccess);
    assert_eq!(report.registered_count, 1);
    assert_eq!(report.failed_count, 1);
    assert!(matches!(
        &report.items[1],
        RegistrationItemOutcome::PersistenceFailed { .. }
    ));
    assert_eq!(repo.list().expect("list").len(), 1);
}

#[cfg(windows)]
#[test]
fn windows_exclusively_locked_file_reports_inaccessible_during_registration() {
    use std::os::windows::fs::OpenOptionsExt;

    let temp = TempTestDir::new("locked_reg");
    let lib_root = temp.path().join("wallpapers");
    let media_dir = temp.path().join("locked_media");
    fs::create_dir_all(&media_dir).expect("create media dir");
    let locked_mp4 = media_dir.join("locked.mp4");
    fs::write(&locked_mp4, mp4_header_bytes()).expect("write locked mp4");

    let _lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(&locked_mp4)
        .expect("acquire exclusive handle");

    let mut repo = LocalJsonCatalogRepository::open(&lib_root).expect("open repo");
    let service = WallpaperRegistrationService::new();
    let report = service
        .register_selection(&mut repo, SelectionOutcome::Selected(vec![locked_mp4]))
        .expect("run batch");

    assert_eq!(report.overall_status, BatchOverallStatus::AllFailed);
    assert_eq!(report.inaccessible_count, 1);
    assert!(matches!(
        &report.items[0],
        RegistrationItemOutcome::InaccessibleOrMissing {
            availability: AvailabilityState::Inaccessible { .. },
            ..
        }
    ));
    assert!(repo.list().expect("list").is_empty());
}
