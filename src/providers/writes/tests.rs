use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

type AfterLeafWrite = Box<dyn FnOnce()>;
thread_local! {
    static AFTER_LEAF_WRITE: std::cell::RefCell<Option<AfterLeafWrite>> =
        const { std::cell::RefCell::new(None) };
}

pub(super) fn after_leaf_write() {
    let hook = AFTER_LEAF_WRITE.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}

fn on_after_leaf_write(hook: impl FnOnce() + 'static) {
    AFTER_LEAF_WRITE.with(|slot| {
        assert!(slot.borrow_mut().replace(Box::new(hook)).is_none());
    });
}

struct Project {
    root: PathBuf,
    data: PathBuf,
}
impl Project {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "eevee-write-{}-{nonce}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        Self {
            data: root.join("own-data"),
            root,
        }
    }
    fn capture(&self, paths: &[&str]) -> WriteSet {
        WriteSet::capture(
            &self.root,
            &self.data,
            &paths.iter().map(PathBuf::from).collect::<Vec<_>>(),
            &[],
        )
        .unwrap()
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn creation_is_not_an_empty_existing_file_and_preparation_has_no_effect() {
    let project = Project::new();
    std::fs::write(project.root.join("existing-empty.txt"), "").unwrap();
    let set = project.capture(&["nested/한글/empty.txt", "existing-empty.txt"]);
    let creation = set
        .propose("nested/한글/empty.txt".into(), String::new())
        .unwrap();
    let replacement = set
        .propose("existing-empty.txt".into(), String::new())
        .unwrap();
    assert_eq!(creation.kind, WriteKind::Create);
    assert_eq!(replacement.kind, WriteKind::Replace);
    assert_eq!(
        creation.directories,
        [PathBuf::from("nested"), PathBuf::from("nested/한글")]
    );
    assert!(
        creation
            .patch
            .starts_with("--- /dev/null\n+++ b/nested/한글/empty.txt\n")
    );
    assert!(replacement.patch.starts_with("--- a/existing-empty.txt\n"));
    // A denied or abandoned proposal never creates placeholders or parents.
    assert!(!project.root.join("nested").exists());
    assert_eq!(
        std::fs::read(project.root.join("existing-empty.txt")).unwrap(),
        b""
    );
    let payload = set.payload(&[]);
    assert_eq!(payload[0]["kind"], "create");
    assert_eq!(payload[1]["kind"], "replace");
    assert_eq!(payload[1]["working_content"], "");
    assert!(payload[0].get("working_content").is_none());
}

#[test]
fn complete_empty_unicode_nested_and_sibling_files_can_then_be_replaced() {
    let project = Project::new();
    let mut set = project.capture(&["new/하위/empty.txt", "new/하위/hello.txt", "root.txt"]);
    // Both exact sibling proposals can be prepared before either approved write.
    let first = set
        .propose("new/하위/empty.txt".into(), String::new())
        .unwrap();
    let second = set
        .propose("new/하위/hello.txt".into(), "안녕하세요 🌱\n".into())
        .unwrap();
    let result = set.apply(&first).unwrap();
    assert_eq!(result.kind, WriteKind::Create);
    assert_eq!(
        result.directories,
        [PathBuf::from("new"), PathBuf::from("new/하위")]
    );
    assert_eq!(std::fs::read(project.root.join(&first.path)).unwrap(), b"");
    let result = set.apply(&second).unwrap();
    assert_eq!(result.kind, WriteKind::Create);
    assert!(result.directories.is_empty());
    assert_eq!(
        std::fs::read_to_string(project.root.join(&second.path)).unwrap(),
        second.content
    );
    let root_file = set.propose("root.txt".into(), "root\n".into()).unwrap();
    set.apply(&root_file).unwrap();
    let later = set
        .propose(second.path, "later replacement\n".into())
        .unwrap();
    assert_eq!(later.kind, WriteKind::Replace);
    assert!(later.directories.is_empty());
    set.apply(&later).unwrap();
    set.verify().unwrap();
    assert_eq!(
        std::fs::read_to_string(project.root.join("new/하위/hello.txt")).unwrap(),
        "later replacement\n"
    );
    assert_eq!(
        std::fs::read_dir(project.root.join("new/하위"))
            .unwrap()
            .count(),
        2
    );
}

#[test]
fn appeared_file_directory_or_missing_parent_is_never_adopted() {
    for leaf_is_directory in [false, true] {
        let project = Project::new();
        let mut set = project.capture(&["target.txt"]);
        let proposal = set
            .propose("target.txt".into(), "approved\n".into())
            .unwrap();
        if leaf_is_directory {
            std::fs::create_dir(project.root.join("target.txt")).unwrap();
        } else {
            std::fs::write(project.root.join("target.txt"), "appeared user data\n").unwrap();
        }
        let failure = set.apply(&proposal).unwrap_err();
        assert!(!failure.uncertain);
        if leaf_is_directory {
            assert!(project.root.join("target.txt").is_dir());
        } else {
            assert_eq!(
                std::fs::read_to_string(project.root.join("target.txt")).unwrap(),
                "appeared user data\n"
            );
        }
    }
    let project = Project::new();
    let mut set = project.capture(&["missing/deeper/target.txt"]);
    let proposal = set
        .propose("missing/deeper/target.txt".into(), "approved".into())
        .unwrap();
    std::fs::create_dir(project.root.join("missing")).unwrap();
    std::fs::write(project.root.join("missing/unrelated.txt"), "user").unwrap();
    assert!(!set.apply(&proposal).unwrap_err().uncertain);
    assert!(!project.root.join("missing/deeper").exists());
    assert_eq!(
        std::fs::read_to_string(project.root.join("missing/unrelated.txt")).unwrap(),
        "user"
    );
}

#[test]
fn dirty_working_baseline_and_every_selected_sibling_are_revalidated() {
    let project = Project::new();
    std::fs::write(
        project.root.join("dirty.txt"),
        "user dirty line\nold requirement\n",
    )
    .unwrap();
    std::fs::write(project.root.join("other.txt"), "original\n").unwrap();
    let mut set = project.capture(&["dirty.txt", "other.txt", "missing/new.txt"]);
    let proposal = set
        .propose(
            "dirty.txt".into(),
            "user dirty line\nnew requirement\n".into(),
        )
        .unwrap();
    set.apply(&proposal).unwrap();
    assert_eq!(
        std::fs::read_to_string(project.root.join("dirty.txt")).unwrap(),
        proposal.content
    );
    let creation = set
        .propose("missing/new.txt".into(), "new\n".into())
        .unwrap();
    std::fs::write(project.root.join("other.txt"), "user changed sibling\n").unwrap();
    assert!(!set.apply(&creation).unwrap_err().uncertain);
    assert!(!project.root.join("missing").exists());
    assert_eq!(
        std::fs::read_to_string(project.root.join("other.txt")).unwrap(),
        "user changed sibling\n"
    );
    assert_eq!(
        std::fs::read_to_string(project.root.join("dirty.txt")).unwrap(),
        proposal.content
    );
}

#[test]
fn mutated_proposals_never_gain_scope_or_change_create_into_replace() {
    let project = Project::new();
    let mut set = project.capture(&["approved/new.txt"]);
    let proposal = set
        .propose("approved/new.txt".into(), "approved\n".into())
        .unwrap();
    let mut changed = proposal.clone();
    changed.content = "not reviewed".into();
    assert!(!set.apply(&changed).unwrap_err().uncertain);
    changed = proposal.clone();
    changed.kind = WriteKind::Replace;
    assert!(!set.apply(&changed).unwrap_err().uncertain);
    changed = proposal.clone();
    changed.directories.push("extra".into());
    assert!(!set.apply(&changed).unwrap_err().uncertain);
    changed = proposal;
    changed.path = "other.txt".into();
    assert!(!set.apply(&changed).unwrap_err().uncertain);
    assert!(!project.root.join("approved").exists());
    assert!(!project.root.join("extra").exists());
    assert!(!project.root.join("other.txt").exists());
}

#[test]
fn protected_private_excluded_and_nonliteral_paths_are_refused_before_io() {
    let project = Project::new();
    for path in [
        ".DS_Store",
        "folder/.ds_store/child",
        ".ev-install/new",
        ".git/config",
        ".env",
        ".env.local",
        "auth.json",
        "oauth_creds.json",
        ".codex/new",
        ".claude/new",
        ".gemini/new",
        ".opencode/new",
        "id_custom",
        "server.key",
        "credentials.json",
        "own-data/new",
        "../outside",
        "a/../b",
        "a/./b",
        "a//b",
        "a/",
        "file:stream",
        "NUL.txt",
        "COM¹",
        "trailing. ",
        "/absolute",
    ] {
        assert!(
            WriteSet::capture(&project.root, &project.data, &[path.into()], &[]).is_err(),
            "{path}"
        );
    }
    #[cfg(not(windows))]
    assert!(WriteSet::capture(&project.root, &project.data, &["a\\b".into()], &[]).is_err());
    assert!(
        WriteSet::capture(
            &project.root,
            &project.data,
            &["Source/new.txt".into()],
            &["source".into()]
        )
        .is_err()
    );
    assert!(
        WriteSet::capture(
            &project.root,
            &project.data,
            &["new".into(), "new/child".into()],
            &[]
        )
        .is_err()
    );
    assert!(
        WriteSet::capture(
            &project.root,
            &project.data,
            &["new".into(), "new".into()],
            &[]
        )
        .is_err()
    );
    assert_eq!(std::fs::read_dir(&project.root).unwrap().count(), 0);
    let executable = std::env::current_exe().unwrap().canonicalize().unwrap();
    assert!(
        WriteSet::capture(
            executable.parent().unwrap(),
            &project.data,
            &[PathBuf::from(executable.file_name().unwrap())],
            &[]
        )
        .is_err()
    );
}

#[test]
fn payload_cites_only_identical_available_complete_capture_in_bound_root() {
    let project = Project::new();
    let text = "fn answer() -> u8 { 42 }\n";
    std::fs::write(project.root.join("file.rs"), text).unwrap();
    let set = project.capture(&["file.rs"]);
    let capture = context::Excerpt {
        id: 77,
        task_id: 1,
        root: project.root.to_string_lossy().into_owned(),
        path: "file.rs".into(),
        start_line: 1,
        end_line: 1,
        source_sha256: Sha256::digest(text.as_bytes()).into(),
        text: text.into(),
        available: true,
        origin_run: None,
    };
    let entry = &set.payload(std::slice::from_ref(&capture))[0];
    assert_eq!(entry["working_content_same_as_capture"], 77);
    assert!(entry.get("working_content").is_none());
    for changed in 0..5 {
        let mut stale = capture.clone();
        match changed {
            0 => stale.root.push_str("-other"),
            1 => stale.start_line = 2,
            2 => stale.source_sha256 = [0; 32],
            3 => stale.available = false,
            _ => stale.text.push_str("different"),
        }
        let payload = set.payload(&[stale]);
        assert_eq!(payload[0]["working_content"], text);
        assert!(payload[0].get("working_content_same_as_capture").is_none());
    }
}

#[cfg(unix)]
#[test]
fn links_hardlinks_and_ancestor_substitution_preserve_unrelated_data() {
    use std::os::unix::fs::symlink;
    let project = Project::new();
    std::fs::write(project.root.join("outside.txt"), "unrelated user content").unwrap();
    symlink("outside.txt", project.root.join("linked.txt")).unwrap();
    symlink("missing-destination", project.root.join("dangling.txt")).unwrap();
    for path in ["linked.txt", "dangling.txt"] {
        assert!(WriteSet::capture(&project.root, &project.data, &[path.into()], &[]).is_err());
    }
    let mut set = project.capture(&["new.txt"]);
    let proposal = set.propose("new.txt".into(), "approved".into()).unwrap();
    symlink("outside.txt", project.root.join("new.txt")).unwrap();
    assert!(!set.apply(&proposal).unwrap_err().uncertain);
    std::fs::hard_link(
        project.root.join("outside.txt"),
        project.root.join("hard.txt"),
    )
    .unwrap();
    assert!(WriteSet::capture(&project.root, &project.data, &["hard.txt".into()], &[]).is_err());
    std::fs::create_dir(project.root.join("parent")).unwrap();
    let mut nested = project.capture(&["parent/new.txt"]);
    let proposal = nested
        .propose("parent/new.txt".into(), "approved".into())
        .unwrap();
    std::fs::rename(
        project.root.join("parent"),
        project.root.join("original-parent"),
    )
    .unwrap();
    std::fs::create_dir(project.root.join("parent")).unwrap();
    std::fs::write(project.root.join("parent/user.txt"), "keep").unwrap();
    assert!(!nested.apply(&proposal).unwrap_err().uncertain);
    assert!(!project.root.join("parent/new.txt").exists());
    assert!(!project.root.join("original-parent/new.txt").exists());
    assert_eq!(
        std::fs::read_to_string(project.root.join("outside.txt")).unwrap(),
        "unrelated user content"
    );
    assert_eq!(
        std::fs::read_to_string(project.root.join("parent/user.txt")).unwrap(),
        "keep"
    );
}

#[cfg(unix)]
#[test]
fn failure_after_approved_parent_creation_is_honestly_uncertain() {
    let project = Project::new();
    // The leaf cannot exist on native POSIX NAME_MAX=255 filesystems, but its
    // reviewed parent is absent and must first be created to attempt the leaf.
    let path = PathBuf::from(format!("approved-parent/{}", "x".repeat(256)));
    let mut set = WriteSet::capture(
        &project.root,
        &project.data,
        std::slice::from_ref(&path),
        &[],
    )
    .unwrap();
    let proposal = set.propose(path, "content".into()).unwrap();
    let failure = set.apply(&proposal).unwrap_err();
    assert!(failure.uncertain);
    assert!(project.root.join("approved-parent").is_dir());
    assert_eq!(
        std::fs::read_dir(project.root.join("approved-parent"))
            .unwrap()
            .count(),
        0
    );
    assert!(failure.message.contains("approved-parent"));
}

#[cfg(unix)]
#[test]
fn no_replace_publication_does_not_touch_appeared_regular_or_dangling_entries() {
    use std::ffi::OsStr;
    use std::os::unix::fs::symlink;
    let project = Project::new();
    let directory = platform::Directory::open_root(&project.root).unwrap();
    std::fs::write(project.root.join("appeared.txt"), "keep").unwrap();
    symlink("missing", project.root.join("dangling.txt")).unwrap();
    for path in ["appeared.txt", "dangling.txt"] {
        assert!(
            !directory
                .publish(OsStr::new(path), b"must not replace")
                .unwrap_err()
                .uncertain
        );
    }
    directory
        .publish(OsStr::new("complete.txt"), "안녕\n".as_bytes())
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(project.root.join("complete.txt")).unwrap(),
        "안녕\n"
    );
    assert_eq!(
        std::fs::read_to_string(project.root.join("appeared.txt")).unwrap(),
        "keep"
    );
    assert!(
        std::fs::symlink_metadata(project.root.join("dangling.txt"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read_dir(&project.root).unwrap().count(), 3);
}

#[cfg(windows)]
#[test]
fn native_windows_selected_separators_support_creation_and_replacement() {
    let project = Project::new();
    let mut set = project.capture(&["native\\하위\\new.txt"]);
    let create = set
        .propose("native\\하위\\new.txt".into(), "새 파일\n".into())
        .unwrap();
    set.apply(&create).unwrap();
    let replace = set
        .propose("native\\하위\\new.txt".into(), "변경\n".into())
        .unwrap();
    assert_eq!(replace.kind, WriteKind::Replace);
    set.apply(&replace).unwrap();
    assert_eq!(
        std::fs::read_to_string(project.root.join("native\\하위\\new.txt")).unwrap(),
        "변경\n"
    );
}

#[test]
fn actual_case_aliases_are_rejected_without_capture_mutation_when_supported() {
    let project = Project::new();
    // The test, never capture, establishes an actual alias on this volume.
    std::fs::create_dir(project.root.join("case-witness")).unwrap();
    let aliased = project.root.join("CASE-WITNESS").exists();
    std::fs::remove_dir(project.root.join("case-witness")).unwrap();
    if !aliased {
        return;
    }
    for paths in [
        ["new.txt", "NEW.txt"],
        ["new/first.txt", "NEW/second.txt"],
        ["new/first.txt", "NEW"],
    ] {
        let paths = paths.map(PathBuf::from);
        assert!(WriteSet::capture(&project.root, &project.data, &paths, &[]).is_err());
        assert!(verify_path_names(&project.root, &paths).is_err());
        assert_eq!(std::fs::read_dir(&project.root).unwrap().count(), 0);
    }
    std::fs::write(project.root.join("existing.txt"), "baseline").unwrap();
    let paths = ["existing.txt".into(), "EXISTING.txt".into()];
    assert!(WriteSet::capture(&project.root, &project.data, &paths, &[]).is_err());
    assert!(verify_path_names(&project.root, &paths).is_err());
    assert_eq!(
        std::fs::read_to_string(project.root.join("existing.txt")).unwrap(),
        "baseline"
    );
    std::fs::create_dir(project.root.join("existing-parent")).unwrap();
    let paths = [
        "existing-parent/missing/first.txt".into(),
        "EXISTING-PARENT/missing/second.txt".into(),
    ];
    assert!(WriteSet::capture(&project.root, &project.data, &paths, &[]).is_err());
    assert!(verify_path_names(&project.root, &paths).is_err());
    assert!(!project.root.join("existing-parent/missing").exists());
    #[cfg(unix)]
    {
        // Existing ancestor aliases may be proven by native ID; distinct
        // children need neither lowercased identity merging nor shared parents.
        let paths = ["existing-parent/first.txt", "EXISTING-PARENT/second.txt"];
        verify_path_names(&project.root, &paths.map(PathBuf::from)).unwrap();
        let mut set = project.capture(&paths);
        for path in paths {
            let proposal = set.propose(path.into(), path.into()).unwrap();
            set.apply(&proposal).unwrap();
        }
        set.verify().unwrap();
    }
}

#[test]
fn actual_normalization_aliases_never_overwrite_or_manufacture_uncertain_effects() {
    let project = Project::new();
    std::fs::create_dir(project.root.join("é-witness")).unwrap();
    let aliased = project.root.join("e\u{301}-witness").exists();
    std::fs::remove_dir(project.root.join("é-witness")).unwrap();
    if !aliased {
        return;
    }
    for paths in [
        ["é.txt", "e\u{301}.txt"],
        ["é/first.txt", "e\u{301}/second.txt"],
    ] {
        let project = Project::new();
        let paths = paths.map(PathBuf::from);
        match WriteSet::capture(&project.root, &project.data, &paths, &[]) {
            Err(_) => assert_eq!(std::fs::read_dir(&project.root).unwrap().count(), 0),
            Ok(mut scope) => {
                // Unknown collation is not guessed into an identity. A known
                // first publication stays settled; its alias is then stale.
                let proposal = scope
                    .propose(paths[0].clone(), "first approved".into())
                    .unwrap();
                scope.apply(&proposal).unwrap();
                assert_eq!(
                    std::fs::read_to_string(project.root.join(&paths[0])).unwrap(),
                    "first approved"
                );
                assert!(
                    scope
                        .propose(paths[1].clone(), "must not replace".into())
                        .is_err()
                );
                assert_eq!(
                    std::fs::read_to_string(project.root.join(&paths[0])).unwrap(),
                    "first approved"
                );
            }
        }
    }
}

#[test]
fn proven_case_sensitive_distinct_targets_and_missing_parents_remain_distinct() {
    let project = Project::new();
    std::fs::create_dir(project.root.join("case-witness")).unwrap();
    let aliased = project.root.join("CASE-WITNESS").exists();
    std::fs::remove_dir(project.root.join("case-witness")).unwrap();
    let directory = platform::Directory::open_root(&project.root).unwrap();
    if aliased
        || !matches!(
            directory.name_policy(),
            NamePolicy::ByteSensitive | NamePolicy::AsciiSensitive
        )
    {
        return;
    }
    let paths = [
        "new.txt",
        "NEW.txt",
        "parent/first.txt",
        "PARENT/second.txt",
    ];
    verify_path_names(&project.root, &paths.map(PathBuf::from)).unwrap();
    let mut set = project.capture(&paths);
    for (index, path) in paths.iter().enumerate() {
        let content = format!("distinct selected content {index}\n");
        let proposal = set.propose((*path).into(), content.clone()).unwrap();
        set.apply(&proposal).unwrap();
        assert_eq!(
            std::fs::read_to_string(project.root.join(path)).unwrap(),
            content
        );
    }
    set.verify().unwrap();
    assert_eq!(std::fs::read_dir(&project.root).unwrap().count(), 4);
}

#[test]
fn unrelated_selected_change_after_leaf_write_does_not_falsify_known_success() {
    for replacement in [false, true] {
        for unrelated_parent in [false, true] {
            let project = Project::new();
            if replacement {
                std::fs::write(project.root.join("written.txt"), "original").unwrap();
                std::fs::write(project.root.join("other.txt"), "other original").unwrap();
            }
            let other = if unrelated_parent {
                "other-parent/other.txt"
            } else {
                "other.txt"
            };
            let mut set = project.capture(&["written.txt", other]);
            let proposal = set
                .propose("written.txt".into(), "exact approved\n".into())
                .unwrap();
            let root = project.root.clone();
            on_after_leaf_write(move || {
                if unrelated_parent {
                    std::fs::create_dir(root.join("other-parent")).unwrap();
                } else {
                    std::fs::write(root.join("other.txt"), "concurrent user change").unwrap();
                }
            });
            let result = set.apply(&proposal).unwrap();
            assert_eq!(result.path, PathBuf::from("written.txt"));
            assert_eq!(
                std::fs::read_to_string(project.root.join("written.txt")).unwrap(),
                proposal.content
            );
            assert!(set.verify().is_err());
            assert!(
                set.propose(other.into(), "must review again".into())
                    .is_err()
            );
            if !unrelated_parent {
                assert_eq!(
                    std::fs::read_to_string(project.root.join("other.txt")).unwrap(),
                    "concurrent user change"
                );
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn actual_local_post_write_loss_or_same_content_substitution_is_uncertain() {
    for replacement in [false, true] {
        for mutation in 0..3 {
            let project = Project::new();
            if replacement {
                std::fs::write(project.root.join("written.txt"), "original").unwrap();
            }
            let mut set = project.capture(&["written.txt"]);
            let proposal = set
                .propose("written.txt".into(), "exact approved\n".into())
                .unwrap();
            let root = project.root.clone();
            on_after_leaf_write(move || match mutation {
                0 => std::fs::write(root.join("written.txt"), "concurrent changed bytes").unwrap(),
                1 => std::fs::remove_file(root.join("written.txt")).unwrap(),
                _ => {
                    std::fs::rename(root.join("written.txt"), root.join("original-written.txt"))
                        .unwrap();
                    std::fs::write(root.join("written.txt"), "exact approved\n").unwrap();
                }
            });
            let failure = set.apply(&proposal).unwrap_err();
            assert!(failure.uncertain, "{failure}");
            if mutation == 2 {
                assert!(failure.message.contains("identity/content"));
                assert_eq!(
                    std::fs::read_to_string(project.root.join("written.txt")).unwrap(),
                    proposal.content
                );
                assert_eq!(
                    std::fs::read_to_string(project.root.join("original-written.txt")).unwrap(),
                    proposal.content
                );
            }
        }
    }
}
