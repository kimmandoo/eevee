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

pub(super) fn on_after_leaf_write(hook: impl FnOnce() + 'static) {
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
fn launching_inside_protected_directories_cannot_create_read_or_write_authority() {
    let project = Project::new();
    for private in [
        ".codex",
        ".claude",
        ".gemini",
        ".opencode",
        ".ssh",
        "secrets",
    ] {
        let nested = project.root.join(private).join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("opaque.txt"), b"unrecognizable private bytes").unwrap();
        assert!(WriteSet::capture_workspace(&nested, &project.data, &[]).is_err());
        assert!(WriteSet::capture_readonly(&nested, &project.data, &[]).is_err());
        assert_eq!(
            std::fs::read(nested.join("opaque.txt")).unwrap(),
            b"unrecognizable private bytes"
        );
    }
    let mut allowed = WriteSet::capture_readonly(&project.root, &project.data, &[]).unwrap();
    std::fs::write(project.root.join("public.txt"), b"public bytes").unwrap();
    let result = allowed
        .observe_tool(&FileTool::Read {
            path: "public.txt".into(),
            start_line: 1,
            end_line: 0,
        })
        .unwrap();
    assert_eq!(result["content"], "public bytes");
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
        || directory.name_policy().compare(
            std::ffi::OsStr::new("case-witness"),
            std::ffi::OsStr::new("CASE-WITNESS"),
        ) != NameRelation::Distinct
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

#[test]
fn workspace_starts_without_targets_and_admits_only_exact_frozen_proposals() {
    let project = Project::new();
    std::fs::write(project.root.join("unrelated.txt"), b"\0private baseline").unwrap();
    let mut set = WriteSet::capture_workspace(&project.root, &project.data, &[]).unwrap();
    assert!(set.targets.is_empty());
    assert_eq!(set.directories.len(), 1);
    assert_eq!(set.payload(&[]), json!([]));
    assert!(
        set.propose("new/한글/empty.txt".into(), String::new())
            .is_err()
    );
    set.admit_paths(&["new/한글/empty.txt".into(), "new/한글/code.txt".into()])
        .unwrap();
    assert!(!project.root.join("new").exists());
    let empty = set
        .propose("new/한글/empty.txt".into(), String::new())
        .unwrap();
    let code = set
        .propose("new/한글/code.txt".into(), "안녕 🌱\r\n".into())
        .unwrap();
    assert!(!code.description().contains('\r'));
    assert!(code.patch.contains("+안녕 🌱 [CRLF]\n"));
    assert!(code.patch.contains("proposed 1 CRLF, 0 LF"));
    set.apply(&empty).unwrap();
    set.apply(&code).unwrap();
    set.admit_paths(&["new/한글/later.txt".into()]).unwrap();
    let later = set
        .propose("new/한글/later.txt".into(), "later\n".into())
        .unwrap();
    assert!(later.directories.is_empty());
    set.apply(&later).unwrap();
    assert_eq!(
        std::fs::read(project.root.join(code.path)).unwrap(),
        code.content.as_bytes()
    );
    assert_eq!(std::fs::read(project.root.join(empty.path)).unwrap(), b"");
    assert_eq!(
        std::fs::read(project.root.join("unrelated.txt")).unwrap(),
        b"\0private baseline"
    );
}

#[test]
fn workspace_batch_admission_is_transactional_for_paths_and_baseline_failures() {
    let project = Project::new();
    std::fs::create_dir(project.root.join("existing")).unwrap();
    std::fs::write(project.root.join("existing/binary.txt"), b"\0").unwrap();
    let mut set =
        WriteSet::capture_workspace(&project.root, &project.data, &["excluded".into()]).unwrap();
    set.admit_paths(&["kept.txt".into()]).unwrap();
    let before = set.payload(&[]);
    for paths in [
        vec!["fresh/nested/new.txt", "../outside"],
        vec!["fresh/nested/new.txt", "own-data/file.txt"],
        vec!["fresh/nested/new.txt", "Excluded/file.txt"],
        vec!["fresh/nested/new.txt", ".env"],
        vec!["fresh/nested/new.txt", "existing/binary.txt"],
        vec!["fresh/nested/new.txt", "fresh/nested/new.txt"],
        vec!["fresh/nested/new.txt", "fresh"],
    ] {
        assert!(
            set.admit_paths(&paths.into_iter().map(PathBuf::from).collect::<Vec<_>>())
                .is_err()
        );
        assert_eq!(set.payload(&[]), before);
        assert_eq!(set.directories.len(), 1);
        assert!(set.owned_directories.is_empty());
        assert!(!project.root.join("fresh").exists());
        assert!(!project.root.join("own-data").exists());
    }
    let executable = std::env::current_exe().unwrap().canonicalize().unwrap();
    let mut executable_scope =
        WriteSet::capture_workspace(executable.parent().unwrap(), &project.data, &[]).unwrap();
    assert!(
        executable_scope
            .admit_paths(&[executable.file_name().unwrap().into()])
            .is_err()
    );
    assert!(executable_scope.targets.is_empty());
}

#[test]
fn exact_restrictions_and_workspace_eight_target_limit_do_not_expand() {
    let project = Project::new();
    let mut exact = project.capture(&["only.txt"]);
    exact.admit_paths(&["only.txt".into()]).unwrap();
    assert!(exact.admit_paths(&["unselected.txt".into()]).is_err());
    assert_eq!(exact.targets.len(), 1);
    let mut workspace = WriteSet::capture_workspace(&project.root, &project.data, &[]).unwrap();
    let paths = (0..8)
        .map(|index| PathBuf::from(format!("file-{index}.txt")))
        .collect::<Vec<_>>();
    workspace.admit_paths(&paths).unwrap();
    workspace.admit_paths(&paths).unwrap();
    assert!(workspace.admit_paths(&["ninth.txt".into()]).is_err());
    assert_eq!(workspace.targets.len(), 8);
    assert!(
        workspace
            .propose("ninth.txt".into(), String::new())
            .is_err()
    );
    assert_eq!(std::fs::read_dir(&project.root).unwrap().count(), 0);
}

#[test]
fn maximum_file_bytes_and_crlf_are_preserved_in_baselines_and_effects() {
    let project = Project::new();
    let original = "dirty user line\r\nold requirement\r\n";
    std::fs::write(project.root.join("dirty.txt"), original).unwrap();
    let mut set = WriteSet::capture_workspace(&project.root, &project.data, &[]).unwrap();
    set.admit_paths(&["dirty.txt".into(), "large.txt".into()])
        .unwrap();
    assert_eq!(set.payload(&[])[0]["working_content"], original);
    let content = "x".repeat(64 * 1024);
    let large = set.propose("large.txt".into(), content.clone()).unwrap();
    assert!(large.patch.contains(&format!("+{content}\n")));
    assert!(
        set.propose("large.txt".into(), "x".repeat(64 * 1024 + 1))
            .is_err()
    );
    let replacement = set
        .propose(
            "dirty.txt".into(),
            "dirty user line\r\nnew requirement\nno final newline".into(),
        )
        .unwrap();
    assert!(!replacement.patch.contains('\r'));
    assert!(replacement.patch.contains("baseline 2 CRLF, 0 LF"));
    assert!(replacement.patch.contains("proposed 1 CRLF, 1 LF"));
    assert!(replacement.patch.contains("-old requirement [CRLF]\n"));
    assert!(replacement.patch.contains("+new requirement [LF]\n"));
    assert!(replacement.patch.contains("\\ No newline at end of file\n"));
    set.apply(&large).unwrap();
    set.apply(&replacement).unwrap();
    assert_eq!(
        std::fs::read(project.root.join("large.txt")).unwrap(),
        content.as_bytes()
    );
    assert_eq!(
        std::fs::read(project.root.join("dirty.txt")).unwrap(),
        replacement.content.as_bytes()
    );
    for unsafe_content in [
        "bare\rreturn",
        "ends in CR\r",
        "\r\r\n",
        "\u{001b}[31m",
        "sk-secret123456",
    ] {
        assert!(
            set.propose("dirty.txt".into(), unsafe_content.into())
                .is_err()
        );
    }
}

#[test]
fn complete_review_line_bound_rejects_hidden_tail_before_consent() {
    let project = Project::new();
    let mut set = WriteSet::capture_workspace(&project.root, &project.data, &[]).unwrap();
    set.admit_paths(&["lines.txt".into()]).unwrap();
    let fitting = set
        .propose("lines.txt".into(), "\n".repeat(32_000))
        .unwrap();
    assert!(fitting.description().lines().count() <= REVIEW_LINES);
    assert!(
        set.propose("lines.txt".into(), "\n".repeat(33_000))
            .is_err()
    );
    set.apply(&fitting).unwrap();
    // Full replacement includes both baseline and proposed lines, not just new.
    assert!(set.propose("lines.txt".into(), "\n".repeat(1_000)).is_err());
    assert_eq!(
        std::fs::read(project.root.join("lines.txt")).unwrap().len(),
        32_000
    );
}

#[test]
fn workspace_never_recaptures_stale_existing_targets_or_absent_parents() {
    for existing in [false, true] {
        let project = Project::new();
        let path = if existing {
            "dirty.txt"
        } else {
            "missing/deeper/file.txt"
        };
        if existing {
            std::fs::write(project.root.join(path), "user dirty\r\n").unwrap();
        }
        let mut set = WriteSet::capture_workspace(&project.root, &project.data, &[]).unwrap();
        set.admit_paths(&[path.into()]).unwrap();
        let frozen = set.payload(&[]);
        let approved = set.propose(path.into(), "approved\r\n".into()).unwrap();
        if existing {
            std::fs::write(project.root.join(path), "newer user dirty\r\n").unwrap();
        } else {
            std::fs::create_dir(project.root.join("missing")).unwrap();
        }
        assert!(set.admit_paths(&[path.into()]).is_err());
        assert!(set.admit_paths(&["another.txt".into()]).is_err());
        assert_eq!(set.payload(&[]), frozen);
        assert!(!set.apply(&approved).unwrap_err().uncertain);
        assert!(!project.root.join("another.txt").exists());
        if existing {
            assert_eq!(
                std::fs::read_to_string(project.root.join(path)).unwrap(),
                "newer user dirty\r\n"
            );
        } else {
            assert!(!project.root.join("missing/deeper").exists());
        }
    }
}

#[test]
fn workspace_alias_batches_never_gain_partial_authority() {
    let project = Project::new();
    let mut set = WriteSet::capture_workspace(&project.root, &project.data, &[]).unwrap();
    let policy = set.directories.get(Path::new("")).unwrap().name_policy();
    if policy.compare(
        std::ffi::OsStr::new("parent"),
        std::ffi::OsStr::new("PARENT"),
    ) == NameRelation::Equivalent
    {
        assert!(
            set.admit_paths(&["parent/a.txt".into(), "PARENT/b.txt".into()])
                .is_err()
        );
        assert!(set.targets.is_empty());
        assert_eq!(set.directories.len(), 1);
    }
    #[cfg(unix)]
    {
        std::fs::create_dir(project.root.join("actual")).unwrap();
        std::os::unix::fs::symlink("actual", project.root.join("alias")).unwrap();
        assert!(
            set.admit_paths(&["fresh/a.txt".into(), "alias/b.txt".into()])
                .is_err()
        );
        assert!(set.targets.is_empty());
        assert!(!project.root.join("fresh").exists());
    }
}

#[test]
fn line_ending_review_suffixes_cannot_be_confused_with_literal_file_text() {
    let project = Project::new();
    let set = project.capture(&["endings.txt"]);
    let first = set
        .propose("endings.txt".into(), "a\r\nb [CRLF]\n".into())
        .unwrap();
    let second = set
        .propose("endings.txt".into(), "a [CRLF]\nb\r\n".into())
        .unwrap();
    assert_ne!(first.patch, second.patch);
    assert!(first.patch.contains("+a [CRLF]\n+b [CRLF] [LF]\n"));
    assert!(second.patch.contains("+a [CRLF] [LF]\n+b [CRLF]\n"));
    assert!(!first.description().contains('\r'));
    assert!(!second.description().contains('\r'));
}

fn read_tool(path: &str) -> FileTool {
    FileTool::Read {
        path: path.into(),
        start_line: 1,
        end_line: 0,
    }
}

#[test]
fn plain_project_discovery_read_generate_and_reviewed_edit_preserve_dirty_crlf() {
    let project = Project::new();
    std::fs::create_dir(project.root.join("src")).unwrap();
    let original = "user dirty header\r\npub fn value() -> i32 { 1 }\r\nunrelated tail\r\n";
    std::fs::write(project.root.join("src/lib.rs"), original).unwrap();
    let mut set = WriteSet::capture_workspace(&project.root, &project.data, &[]).unwrap();
    let names = set
        .observe_tool(&FileTool::List {
            query: "src/".into(),
            offset: 0,
        })
        .unwrap();
    assert_eq!(names["paths"], json!(["src/lib.rs"]));
    assert_eq!(names["truncated"], false);
    let source = set.observe_tool(&read_tool("src/lib.rs")).unwrap();
    assert_eq!(source["content"], original);
    assert_eq!(source["bytes"], original.len());
    assert_eq!(source["start_line"], 1);
    assert_eq!(source["end_line"], 3);
    assert_eq!(source["truncated"], false);
    assert!(set.targets.is_empty());
    let mut edit = FileTool::Edit {
        path: "src/lib.rs".into(),
        sha256: source["sha256"].as_str().unwrap().into(),
        old_text: "{ 1 }".into(),
        new_text: "{ 2 }".into(),
    };
    set.admit_paths(&[edit.path().into()]).unwrap();
    let proposal = set.propose_tool(&mut edit).unwrap();
    assert_eq!(
        std::fs::read_to_string(project.root.join("src/lib.rs")).unwrap(),
        original
    );
    assert!(proposal.description().contains("[CRLF]"));
    set.apply(&proposal).unwrap();
    assert_eq!(
        std::fs::read(project.root.join("src/lib.rs")).unwrap(),
        original.replace("{ 1 }", "{ 2 }").as_bytes()
    );
    // Derive a new file from the actual read and use the real approved executor.
    let mut generated = FileTool::Write {
        path: "generated/example.rs".into(),
        content: format!(
            "// generated from source\n{}",
            source["content"].as_str().unwrap()
        ),
    };
    set.admit_paths(&[generated.path().into()]).unwrap();
    let creation = set.propose_tool(&mut generated).unwrap();
    assert!(!project.root.join("generated").exists());
    set.apply(&creation).unwrap();
    assert_eq!(
        std::fs::read_to_string(project.root.join("generated/example.rs")).unwrap(),
        creation.content
    );
    assert_eq!(
        set.observe_tool(&read_tool("src/lib.rs")).unwrap()["content"],
        original.replace("{ 1 }", "{ 2 }")
    );
}

#[test]
fn source_reads_are_separate_from_selected_write_authority_and_eight_target_quota() {
    let project = Project::new();
    for index in 0..12 {
        std::fs::write(
            project.root.join(format!("source-{index}.txt")),
            format!("context {index}"),
        )
        .unwrap();
    }
    let mut set = WriteSet::capture_workspace(&project.root, &project.data, &[]).unwrap();
    for index in 0..12 {
        set.observe_tool(&read_tool(&format!("source-{index}.txt")))
            .unwrap();
    }
    assert!(set.targets.is_empty());
    set.admit_paths(
        &(0..MAX_FILES)
            .map(|index| format!("new-{index}.txt").into())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert_eq!(set.targets.len(), MAX_FILES);
    let mut selected = project.capture(&["selected.txt"]);
    selected.observe_tool(&read_tool("source-0.txt")).unwrap();
    assert_eq!(selected.targets.len(), 1);
    assert!(selected.admit_paths(&["source-0.txt".into()]).is_err());
}

#[test]
fn stale_and_ambiguous_edits_refuse_before_enrollment_or_effects() {
    let project = Project::new();
    std::fs::write(project.root.join("source.txt"), "same same\r\n").unwrap();
    let mut set = WriteSet::capture_workspace(&project.root, &project.data, &[]).unwrap();
    let read = set.observe_tool(&read_tool("source.txt")).unwrap();
    let mut candidate = set.clone();
    candidate.admit_paths(&["source.txt".into()]).unwrap();
    let mut edit = FileTool::Edit {
        path: "source.txt".into(),
        sha256: read["sha256"].as_str().unwrap().into(),
        old_text: "same".into(),
        new_text: "other".into(),
    };
    assert!(
        candidate
            .propose_tool(&mut edit)
            .unwrap_err()
            .contains("exactly once")
    );
    if let FileTool::Edit { old_text, .. } = &mut edit {
        *old_text = "same same".into();
    }
    let reviewed = candidate.propose_tool(&mut edit).unwrap();
    std::fs::write(project.root.join("source.txt"), "concurrent user bytes\r\n").unwrap();
    assert!(candidate.apply(&reviewed).is_err());
    assert!(
        set.admit_paths(&["source.txt".into()])
            .unwrap_err()
            .contains("Source changed since read")
    );
    assert!(set.targets.is_empty());
    assert_eq!(
        std::fs::read(project.root.join("source.txt")).unwrap(),
        b"concurrent user bytes\r\n"
    );
}

#[test]
fn source_ranges_limits_and_privacy_fail_closed() {
    let project = Project::new();
    std::fs::write(project.root.join("safe.txt"), "first\r\nsecond\r\nthird").unwrap();
    std::fs::write(project.root.join(".env"), "sensitive").unwrap();
    std::fs::create_dir(&project.data).unwrap();
    std::fs::write(project.data.join("private.txt"), "private").unwrap();
    std::fs::create_dir(project.root.join("excluded")).unwrap();
    std::fs::write(project.root.join("excluded/context.txt"), "excluded").unwrap();
    std::fs::write(project.root.join("large.txt"), "x".repeat(MAX_BYTES + 1)).unwrap();
    std::fs::write(project.root.join("token.txt"), "password=private-value").unwrap();
    let mut set =
        WriteSet::capture_readonly(&project.root, &project.data, &["excluded".into()]).unwrap();
    for path in [
        ".env",
        "own-data/private.txt",
        "excluded/context.txt",
        "../outside",
        "/outside",
        "large.txt",
        "token.txt",
        "./safe.txt",
    ] {
        assert!(
            set.observe_tool(&read_tool(path)).is_err(),
            "accepted {path}"
        );
    }
    let range = set
        .observe_tool(&FileTool::Read {
            path: "safe.txt".into(),
            start_line: 2,
            end_line: 2,
        })
        .unwrap();
    assert_eq!(range["content"], "second\r\n");
    assert_eq!(range["total_lines"], 3);
    assert_eq!(range["truncated"], true);
    assert!(
        set.observe_tool(&FileTool::Read {
            path: "safe.txt".into(),
            start_line: 1,
            end_line: 4
        })
        .is_err()
    );
    let names = set
        .observe_tool(&FileTool::List {
            query: "".into(),
            offset: 0,
        })
        .unwrap();
    let paths = names["paths"].as_array().unwrap();
    assert!(!paths.iter().any(|path| path == ".env"
        || path == "own-data/private.txt"
        || path == "excluded/context.txt"
        || path == "large.txt"));
}

#[cfg(unix)]
#[test]
fn source_tools_never_follow_file_directory_or_hard_links() {
    let project = Project::new();
    std::fs::write(project.root.join("original.txt"), "bytes").unwrap();
    std::fs::hard_link(
        project.root.join("original.txt"),
        project.root.join("hard.txt"),
    )
    .unwrap();
    std::os::unix::fs::symlink("original.txt", project.root.join("alias.txt")).unwrap();
    std::fs::create_dir(project.root.join("real")).unwrap();
    std::fs::write(project.root.join("real/file.txt"), "safe").unwrap();
    std::os::unix::fs::symlink("real", project.root.join("alias-dir")).unwrap();
    let mut set = WriteSet::capture_readonly(&project.root, &project.data, &[]).unwrap();
    for path in [
        "original.txt",
        "hard.txt",
        "alias.txt",
        "alias-dir/file.txt",
    ] {
        assert!(set.observe_tool(&read_tool(path)).is_err());
    }
    let names = set
        .observe_tool(&FileTool::List {
            query: "".into(),
            offset: 0,
        })
        .unwrap();
    assert_eq!(names["paths"], json!(["real/file.txt"]));
}

#[test]
fn normalized_privacy_aliases_never_enter_source_reads_or_discovery() {
    let project = Project::new();
    let protected = project.root.join("café");
    std::fs::create_dir(&protected).unwrap();
    std::fs::write(protected.join("private.txt"), "private project state").unwrap();
    std::fs::create_dir(project.root.join("éxcluded")).unwrap();
    std::fs::write(project.root.join("éxcluded/source.txt"), "excluded bytes").unwrap();
    std::fs::write(project.root.join("public.txt"), "visible bytes").unwrap();
    let mut set =
        WriteSet::capture_readonly(&project.root, &protected, &["éxcluded".into()]).unwrap();
    for path in [
        "café/private.txt",
        "cafe\u{301}/private.txt",
        "éxcluded/source.txt",
        "e\u{301}xcluded/source.txt",
    ] {
        assert!(
            set.observe_tool(&read_tool(path)).is_err(),
            "accepted {path}"
        );
    }
    let names = set
        .observe_tool(&FileTool::List {
            query: "".into(),
            offset: 0,
        })
        .unwrap();
    assert_eq!(names["paths"], json!(["public.txt"]));
    let context = set.project_context();
    assert_eq!(context["root"], json!(project.root));
    assert_eq!(context["filenames"]["paths"], json!(["public.txt"]));
    assert_eq!(context["filenames"]["complete"], true);
    assert!(!context.to_string().contains("visible bytes"));
    assert!(!context.to_string().contains("private.txt"));
    assert!(!context.to_string().contains("source.txt"));
}

#[test]
fn git_inventory_keeps_native_unicode_spelling_for_generated_sources() {
    let project = Project::new();
    for args in [
        &["init", "-q"][..],
        &["config", "core.precomposeunicode", "true"][..],
    ] {
        assert!(
            std::process::Command::new("git")
                .args(args)
                .current_dir(&project.root)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    let path = "generated/cafe\u{301}.py";
    std::fs::create_dir(project.root.join("generated")).unwrap();
    std::fs::write(project.root.join(path), "value = 1\r\n").unwrap();
    std::fs::write(project.root.join(".gitignore"), "ignored.txt\n").unwrap();
    std::fs::write(project.root.join("ignored.txt"), "ignored bytes").unwrap();
    let mut set = WriteSet::capture_readonly(&project.root, &project.data, &[]).unwrap();
    let read = set.observe_tool(&read_tool(path)).unwrap();
    assert_eq!(read["content"], "value = 1\r\n");
    let names = set
        .observe_tool(&FileTool::List {
            query: "generated/".into(),
            offset: 0,
        })
        .unwrap();
    assert!(
        names["paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == path)
    );
    assert!(set.observe_tool(&read_tool("ignored.txt")).is_err());
}

#[test]
fn project_observation_keeps_the_launch_subfolder_and_git_ignores() {
    let project = Project::new();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&project.root)
            .output()
            .unwrap()
            .status
            .success()
    );
    let launch = project.root.join("current");
    std::fs::create_dir(&launch).unwrap();
    std::fs::write(project.root.join("outside.txt"), "outside current root").unwrap();
    std::fs::write(project.root.join(".gitignore"), "ignored.txt\n").unwrap();
    std::fs::write(launch.join("ignored.txt"), "ignored").unwrap();
    std::fs::write(launch.join(".env"), "private").unwrap();
    std::fs::write(launch.join("visible.rs"), "source-body-canary").unwrap();
    let scope = WriteSet::capture_readonly(&launch, &project.data, &[]).unwrap();
    let context = scope.project_context();
    assert_eq!(context["root"], json!(launch));
    assert_eq!(context["filenames"]["paths"], json!(["visible.rs"]));
    assert_eq!(context["filenames"]["complete"], true);
    assert!(!context.to_string().contains("source-body-canary"));
    assert!(!scope.writing());
    assert!(scope.targets.is_empty() && scope.observed.is_empty());
}

#[test]
fn project_identity_refuses_credential_and_control_bearing_folder_names() {
    let project = Project::new();
    for name in ["password=private-root-canary", "unsafe\u{202e}root"] {
        let root = project.root.join(name);
        std::fs::create_dir(&root).unwrap();
        assert!(WriteSet::capture_readonly(&root, &project.data, &[]).is_err());
    }
}

#[test]
fn project_observation_bounds_names_and_bytes_without_changing_frozen_preview() {
    let project = Project::new();
    for index in 0..140 {
        std::fs::write(
            project
                .root
                .join(format!("{index:03}-{}.rs", "a".repeat(180))),
            "",
        )
        .unwrap();
    }
    let mut scope = WriteSet::capture_readonly(&project.root, &project.data, &[]).unwrap();
    assert_eq!(
        scope
            .observe_tool(&FileTool::List {
                query: String::new(),
                offset: 0
            })
            .unwrap()["paths"]
            .as_array()
            .unwrap()
            .len(),
        128
    );
    let frozen = scope.project_context().clone();
    let filenames = &frozen["filenames"];
    let names = filenames["paths"].as_array().unwrap();
    assert!(!names.is_empty() && names.len() < 128);
    assert!(
        names
            .iter()
            .map(|name| name.as_str().unwrap().len())
            .sum::<usize>()
            <= 16 * 1024
    );
    assert_eq!(filenames["total"], 140);
    assert_eq!(filenames["truncated"], true);
    assert_eq!(filenames["complete"], false);
    assert_eq!(filenames["next_offset"], names.len());
    std::fs::write(project.root.join("later.rs"), "new bytes").unwrap();
    assert_eq!(scope.project_context(), &frozen);
    assert!(std::ptr::eq(
        scope.project_context(),
        scope.clone().project_context()
    ));
    assert_eq!(
        scope
            .observe_tool(&FileTool::List {
                query: "later".into(),
                offset: 0
            })
            .unwrap()["paths"],
        json!(["later.rs"])
    );
    let next = WriteSet::capture_readonly(&project.root, &project.data, &[]).unwrap();
    assert_eq!(next.project_context()["filenames"]["total"], 141);
}

#[test]
fn unavailable_project_observation_is_not_reported_as_empty_or_complete() {
    let project = Project::new();
    let mut nested = project.root.clone();
    for _ in 0..66 {
        nested.push("d");
        std::fs::create_dir(&nested).unwrap();
    }
    std::fs::write(nested.join("hidden-by-bound.rs"), "not uploaded").unwrap();
    let scope = WriteSet::capture_readonly(&project.root, &project.data, &[]).unwrap();
    let context = scope.project_context();
    assert_eq!(context["filenames"]["status"], "unavailable");
    assert_eq!(context["filenames"]["complete"], false);
    assert!(context["filenames"]["truncated"].is_null());
    assert!(context["filenames"]["total"].is_null());
    assert_eq!(context["filenames"]["paths"], json!([]));
}

#[test]
fn readonly_scope_cannot_enroll_propose_or_apply_even_a_valid_writer_proposal() {
    let project = Project::new();
    std::fs::write(project.root.join("actual.txt"), "frozen dirty bytes\r\n").unwrap();
    let writer = project.capture(&["actual.txt"]);
    let proposal = writer
        .propose("actual.txt".into(), "unapproved replacement".into())
        .unwrap();
    let mut scope = WriteSet::capture_readonly(&project.root, &project.data, &[]).unwrap();
    assert!(!scope.writing());
    assert_eq!(
        scope.observe_tool(&read_tool("actual.txt")).unwrap()["content"],
        "frozen dirty bytes\r\n"
    );
    assert!(
        scope
            .admit_paths(&["actual.txt".into(), "new/child.txt".into()])
            .unwrap_err()
            .contains("Read-only")
    );
    assert!(
        scope
            .propose("actual.txt".into(), "unapproved".into())
            .unwrap_err()
            .contains("Read-only")
    );
    let failed = scope.apply(&proposal).unwrap_err();
    assert!(!failed.uncertain);
    assert!(failed.message.contains("Read-only"));
    assert!(scope.targets.is_empty());
    assert!(scope.owned_directories.is_empty());
    assert_eq!(scope.directories.len(), 1);
    assert!(!project.root.join("new").exists());
    assert_eq!(
        std::fs::read_to_string(project.root.join("actual.txt")).unwrap(),
        "frozen dirty bytes\r\n"
    );
}

#[cfg(unix)]
#[test]
fn readonly_root_capture_and_source_tools_need_no_mutation_permissions() {
    use std::os::unix::fs::PermissionsExt;
    struct Restore(PathBuf, std::fs::Permissions);
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, self.1.clone());
        }
    }
    let project = Project::new();
    let path = project.root.join("source.txt");
    std::fs::write(&path, "readonly actual bytes\r\n").unwrap();
    let _restore = Restore(
        project.root.clone(),
        std::fs::metadata(&project.root).unwrap().permissions(),
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
    std::fs::set_permissions(&project.root, std::fs::Permissions::from_mode(0o555)).unwrap();
    let mut scope = WriteSet::capture_readonly(&project.root, &project.data, &[]).unwrap();
    assert_eq!(
        scope.observe_tool(&read_tool("source.txt")).unwrap()["content"],
        "readonly actual bytes\r\n"
    );
    assert_eq!(
        scope
            .observe_tool(&FileTool::List {
                query: String::new(),
                offset: 0
            })
            .unwrap()["paths"],
        json!(["source.txt"])
    );
    assert!(scope.targets.is_empty());
}

#[cfg(unix)]
#[test]
fn readonly_scope_refuses_rebound_source_root_before_observation_or_mutation() {
    let project = Project::new();
    let mut scope = WriteSet::capture_readonly(&project.root, &project.data, &[]).unwrap();
    let moved = project.root.with_extension("moved-readonly");
    std::fs::rename(&project.root, &moved).unwrap();
    std::fs::create_dir(&project.root).unwrap();
    std::fs::write(project.root.join("substituted.txt"), "must not read").unwrap();
    assert!(scope.verify().is_err());
    assert!(scope.observe_tool(&read_tool("substituted.txt")).is_err());
    assert!(scope.admit_paths(&["unapproved.txt".into()]).is_err());
    std::fs::remove_dir_all(&project.root).unwrap();
    std::fs::rename(moved, &project.root).unwrap();
}
