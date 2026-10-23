use super::*;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let mut random = [0u8; 16];
        getrandom::getrandom(&mut random).unwrap();
        let directory = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "ev-lifecycle-test-{:032x}",
                u128::from_le_bytes(random)
            ));
        fs::create_dir(&directory).unwrap();
        Self(directory)
    }
    fn install(&self, version: &str) -> Installation {
        use std::os::unix::fs::PermissionsExt;
        fs::write(
            executable(&self.0),
            format!("#!/bin/sh\n[ \"$1\" = --version ] || exit 91\nprintf 'ev {version}\\n'\n"),
        )
        .unwrap();
        fs::set_permissions(executable(&self.0), fs::Permissions::from_mode(0o755)).unwrap();
        let metadata = self.0.join(".ev-install");
        fs::create_dir_all(&metadata).unwrap();
        fs::write(metadata.join("LICENSE"), b"license").unwrap();
        fs::write(metadata.join("THIRD_PARTY_NOTICES"), b"notices").unwrap();
        fs::write(metadata.join(".native-install"), format!("ev-native-install-v2\nversion=ev {version}\nsha256={}\nlicense-sha256={}\nnotices-sha256={}\n", hash_file(&executable(&self.0), MAX_BINARY).unwrap(), hash_file(&metadata.join("LICENSE"), MAX_METADATA).unwrap(), hash_file(&metadata.join("THIRD_PARTY_NOTICES"), MAX_METADATA).unwrap())).unwrap();
        owned_installation(&executable(&self.0)).unwrap()
    }
    fn rollback_review(&self) -> LifecycleReview {
        let prior = self.install("0.1.0");
        retain(&prior).unwrap();
        let current = self.install("0.1.1");
        rollback_review(current, &self.0.join("absent-data")).unwrap()
    }
    fn binary(&self) -> Vec<u8> {
        fs::read(executable(&self.0)).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn reviewed_retained_package_rolls_back_through_real_offline_installer_and_returns() {
    let fixture = Fixture::new();
    let review = fixture.rollback_review();
    assert!(review.can_apply);
    assert!(!fixture.0.join("absent-data").exists());
    apply_review(&review, true).unwrap();
    assert_eq!(
        owned_installation(&executable(&fixture.0)).unwrap().version,
        Version::parse("0.1.0").unwrap()
    );
    assert!(!fixture.0.join("absent-data").exists());
    assert!(inspect_retained(&fixture.0.join(SLOT)).is_ok());
}

#[test]
fn stale_owner_review_refuses_before_installer_without_replacing_current_package() {
    let fixture = Fixture::new();
    let review = fixture.rollback_review();
    fixture.install("0.1.2");
    let before = fixture.binary();
    let result = apply_with(&review, true, |_, _, _, _| {
        panic!("stale review reached installer")
    });
    assert!(result.is_err());
    assert_eq!(fixture.binary(), before);
}

#[test]
fn retained_checksum_and_live_ownership_tampering_refuse_without_install_mutation() {
    for tamper_archive in [true, false] {
        let fixture = Fixture::new();
        let review = fixture.rollback_review();
        let before = fixture.binary();
        if tamper_archive {
            fs::write(
                fixture.0.join(SLOT).join(archive_name().unwrap()),
                b"tampered",
            )
            .unwrap();
        } else {
            fs::write(fixture.0.join(".ev-install/LICENSE"), b"tampered").unwrap();
        }
        assert!(
            apply_with(&review, true, |_, _, _, _| panic!(
                "tampering reached installer"
            ))
            .is_err()
        );
        assert_eq!(fixture.binary(), before);
    }
}

#[test]
fn unknown_or_changed_strict_reader_identity_refuses_without_install_or_data_writes() {
    let fixture = Fixture::new();
    let _review = fixture.rollback_review();
    let before = fixture.binary();
    let manifest = fixture.0.join(SLOT).join(MANIFEST);
    let record = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        record.replace(format_identity(), &"0".repeat(64)),
    )
    .unwrap();
    assert!(
        rollback_review(
            owned_installation(&executable(&fixture.0)).unwrap(),
            &fixture.0.join("absent-data")
        )
        .is_err()
    );
    assert_eq!(fixture.binary(), before);
    assert!(!fixture.0.join("absent-data").exists());
}

#[test]
fn installer_rechecks_frozen_owner_and_archive_under_its_publication_guards() {
    for stale_owner in [true, false] {
        let fixture = Fixture::new();
        let review = fixture.rollback_review();
        let retained = review.rollback.unwrap();
        let before = fixture.binary();
        let wrong = "0".repeat(64);
        let owner = if stale_owner {
            &wrong
        } else {
            &review.installation.owner
        };
        let archive_hash = if stale_owner {
            &retained.archive_hash
        } else {
            &wrong
        };
        assert!(
            run_installer(
                &fixture.0,
                Some(&review.tag),
                true,
                Some(owner),
                Some((
                    &retained.directory.join(archive_name().unwrap()),
                    &retained.directory.join("SHA256SUMS"),
                    archive_hash
                ))
            )
            .is_err()
        );
        assert_eq!(fixture.binary(), before);
    }
}

#[test]
fn read_only_review_and_cancellation_leave_installation_and_retention_unchanged() {
    let fixture = Fixture::new();
    let review = fixture.rollback_review();
    let before = fixture.binary();
    let slot = inspect_retained(&fixture.0.join(SLOT)).unwrap();
    let second =
        rollback_review(review.installation.clone(), &fixture.0.join("absent-data")).unwrap();
    drop(review);
    drop(second);
    assert_eq!(fixture.binary(), before);
    assert_eq!(inspect_retained(&fixture.0.join(SLOT)).unwrap(), slot);
    assert!(!fixture.0.join("absent-data").exists());
}

#[test]
fn abandoned_update_staging_preserves_existing_prior_slot_and_current_installation() {
    let fixture = Fixture::new();
    let review = fixture.rollback_review();
    let previous = inspect_retained(&fixture.0.join(SLOT)).unwrap();
    let before = fixture.binary();
    let staged = stage_retained(&review.installation).unwrap();
    drop(staged);
    assert_eq!(fixture.binary(), before);
    assert_eq!(inspect_retained(&fixture.0.join(SLOT)).unwrap(), previous);
}

#[test]
fn successful_retention_replaces_only_the_single_verified_prior_slot() {
    let fixture = Fixture::new();
    fixture.rollback_review();
    let newer = fixture.install("0.1.2");
    retain(&newer).unwrap();
    let retained = inspect_retained(&fixture.0.join(SLOT)).unwrap();
    assert_eq!(
        retained.installation.version,
        Version::parse("0.1.2").unwrap()
    );
    assert_eq!(retained.format, format_identity());
    assert!(fs::read_dir(&fixture.0).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".ev-retain-")
    }));
}

#[test]
fn explicit_and_latest_older_update_targets_cannot_bypass_verified_rollback() {
    for explicit in [true, false] {
        let fixture = Fixture::new();
        let installation = fixture.install("0.1.2");
        let before = fixture.binary();
        let mut review = update_review(installation, "v0.1.0".into(), explicit).unwrap();
        assert!(!review.can_apply);
        // Display fields cannot turn an ineligible immutable review into a grant.
        review.can_apply = true;
        assert!(
            apply_with(&review, true, |_, _, _, _| panic!(
                "downgrade reached installer"
            ))
            .is_err()
        );
        assert_eq!(fixture.binary(), before);
        assert!(!fixture.0.join(SLOT).exists());
    }
}
