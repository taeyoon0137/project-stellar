use super::*;
use crate::blob_write::write_blob;
use crate::fixture::GitFixture;
use crate::object::ObjectId;
use crate::snapshot::read_index_snapshot;

fn fixture(tag: &str) -> GitFixture {
    if let Some(git) = std::env::var_os("STELLAR_INDEX_PUBLISH_TEST_GIT") {
        GitFixture::new(git.into(), tag).unwrap()
    } else {
        GitFixture::with_system_git(tag)
            .unwrap()
            .expect("system Git required")
    }
}
fn path(s: &str) -> GitPath {
    GitPath::from_bytes(s.as_bytes())
}
fn upsert(fx: &GitFixture, p: &str, mode: FileMode, bytes: &[u8]) -> WrittenFileDelta {
    WrittenFileDelta::Upsert {
        path: path(p),
        mode,
        oid: write_blob(&fx.gated(), fx.work(), bytes).unwrap(),
    }
}
fn candidate(fx: &GitFixture, changes: Vec<WrittenFileDelta>) -> WrittenCandidate {
    WrittenCandidate::for_test(
        read_index_snapshot(&fx.gated(), fx.work()).unwrap(),
        changes,
    )
}
fn set(fx: &GitFixture, p: &str, oid: &ObjectId) {
    let output = fx
        .gated()
        .run_with_input(
            &argv(["update-index", "--index-info"]),
            fx.work(),
            format!("100644 {oid}\t{p}\n").as_bytes(),
        )
        .unwrap();
    assert!(output.success(), "{output:?}");
}
fn publish(fx: &GitFixture, c: &WrittenCandidate) -> Result<IndexSnapshot, PublishError> {
    publish_candidate(&fx.gated(), fx.work(), c, &GitCancelToken::new())
}

#[test]
fn cold_start_and_many_paths_preserve_working_bytes() {
    let fx = fixture("publish-cold");
    assert!(!fx.work().join(".git/index").exists());
    fx.write("selected", b"working A+B").unwrap();
    let c = candidate(
        &fx,
        vec![
            upsert(&fx, "selected", FileMode::REGULAR, b"A only"),
            upsert(&fx, "new\t\n한글", FileMode::EXECUTABLE, b"exec"),
        ],
    );
    let actual = publish(&fx, &c).unwrap();
    assert_eq!(actual, expected_index(&c).unwrap());
    assert_eq!(read_index_snapshot(&fx.gated(), fx.work()).unwrap(), actual);
    assert_eq!(
        std::fs::read(fx.work().join("selected")).unwrap(),
        b"working A+B"
    );
    assert!(!fx.work().join("new\t\n한글").exists());
    assert!(!fx.work().join(".git/index.lock").exists());
}

#[test]
fn stale_same_path_and_both_directory_file_races_preserve_external_writer() {
    for (target, external) in [("selected", "selected"), ("d/a", "d"), ("d", "d/a")] {
        let fx = fixture("publish-stale");
        let c = candidate(
            &fx,
            vec![upsert(&fx, target, FileMode::REGULAR, b"candidate")],
        );
        let external_oid = write_blob(&fx.gated(), fx.work(), b"external").unwrap();
        set(&fx, external, &external_oid);
        let before = index_bytes(&fx.work().join(".git/index")).unwrap();
        assert_eq!(publish(&fx, &c), Err(PublishError::StaleBase));
        assert_eq!(index_bytes(&fx.work().join(".git/index")).unwrap(), before);
        assert!(!fx.work().join(".git/index.lock").exists());
    }
}

#[test]
fn modes_deletion_and_valid_directory_file_transitions() {
    for (old, new) in [("d/a", "d"), ("d", "d/a")] {
        let fx = fixture("publish-modes");
        fx.write(old, b"working").unwrap();
        fx.git(["add", old]).unwrap();
        let c = candidate(
            &fx,
            vec![
                WrittenFileDelta::Remove { path: path(old) },
                upsert(&fx, new, FileMode::SYMLINK, b"../target"),
                upsert(&fx, "exec", FileMode::EXECUTABLE, b"exec"),
            ],
        );
        assert_eq!(publish(&fx, &c).unwrap(), expected_index(&c).unwrap());
        assert_eq!(std::fs::read(fx.work().join(old)).unwrap(), b"working");
    }
}

#[test]
fn unrelated_flags_and_resolve_undo_survive_selected_update() {
    let fx = fixture("publish-metadata");
    for p in ["selected", "assume", "skip", "resolved"] {
        fx.write(p, p.as_bytes()).unwrap()
    }
    fx.git(["add", "-A"]).unwrap();
    fx.write("ita", b"not staged").unwrap();
    fx.git(["add", "-N", "ita"]).unwrap();
    fx.git(["update-index", "--assume-unchanged", "assume"])
        .unwrap();
    fx.git(["update-index", "--skip-worktree", "skip"]).unwrap();
    let oid = write_blob(&fx.gated(), fx.work(), b"conflict").unwrap();
    let info = format!("0 {}\tresolved\n100644 {oid} 1\tresolved\n100644 {oid} 2\tresolved\n100644 {oid} 3\tresolved\n", "0".repeat(40));
    assert!(fx
        .gated()
        .run_with_input(
            &argv(["update-index", "--index-info"]),
            fx.work(),
            info.as_bytes()
        )
        .unwrap()
        .success());
    set(&fx, "resolved", &oid);
    let reuc = fx.git(["ls-files", "--resolve-undo", "-z"]).unwrap().stdout;
    assert!(!reuc.is_empty());
    let flags = fx
        .git(["ls-files", "--debug", "-z", "--", "assume", "skip", "ita"])
        .unwrap()
        .stdout;
    let c = candidate(
        &fx,
        vec![upsert(&fx, "selected", FileMode::REGULAR, b"candidate")],
    );
    let receipt = publish_with_receipt(&fx.gated(), fx.work(), &c, &GitCancelToken::new()).unwrap();
    assert_eq!(
        fx.git(["ls-files", "--resolve-undo", "-z"]).unwrap().stdout,
        reuc
    );
    assert_eq!(
        fx.git(["ls-files", "--debug", "-z", "--", "assume", "skip", "ita"])
            .unwrap()
            .stdout,
        flags
    );
    assert_eq!(
        restore_published(&fx.gated(), receipt, &GitCancelToken::new()).unwrap(),
        *c.base_index()
    );
    assert_eq!(
        fx.git(["ls-files", "--resolve-undo", "-z"]).unwrap().stdout,
        reuc
    );
    assert_eq!(
        fx.git(["ls-files", "--debug", "-z", "--", "assume", "skip", "ita"])
            .unwrap()
            .stdout,
        flags
    );
}

#[test]
fn selected_flags_and_split_index_are_diagnosed_without_mutation() {
    for option in ["--assume-unchanged", "--skip-worktree", "--split-index"] {
        let fx = fixture("publish-flags");
        fx.write("selected", b"base").unwrap();
        fx.git(["add", "selected"]).unwrap();
        if option == "--split-index" {
            fx.git(["update-index", option]).unwrap();
        } else {
            fx.git(["update-index", option, "selected"]).unwrap();
        }
        let c = candidate(
            &fx,
            vec![upsert(&fx, "selected", FileMode::REGULAR, b"candidate")],
        );
        let before = index_bytes(&fx.work().join(".git/index")).unwrap();
        assert!(matches!(
            publish(&fx, &c),
            Err(PublishError::Unsupported(_))
        ));
        assert_eq!(index_bytes(&fx.work().join(".git/index")).unwrap(), before);
        assert!(!fx.work().join(".git/index.lock").exists());
    }
}

#[test]
fn a_foreign_git_lock_is_never_mistaken_for_our_childs_lock() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    let fx = fixture("publish-foreign-lock");
    let c = candidate(
        &fx,
        vec![upsert(&fx, "selected", FileMode::REGULAR, b"candidate")],
    );
    let external = write_blob(&fx.gated(), fx.work(), b"external").unwrap();
    let external_input = format!("100644 {external}\tselected\n");
    let release = AtomicBool::new(false);
    let (ready_tx, ready_rx) = mpsc::channel();
    std::thread::scope(|scope| {
        let writer = scope.spawn(|| {
            let mut announced = false;
            protocol_runner(&fx.gated())
                .unwrap()
                .run_with_checked_input(
                    &argv(["update-index", "--index-info"]),
                    fx.work(),
                    external_input.as_bytes(),
                    &GitCancelToken::new(),
                    &mut |pid, _, stderr| {
                        if !index_read_ready(pid, stderr) {
                            return Ok(false);
                        }
                        if !ownership::owns_lock(pid, &fx.work().join(".git/index.lock")).unwrap() {
                            return Ok(false);
                        }
                        if !announced {
                            ready_tx.send(pid).unwrap();
                            announced = true;
                        }
                        Ok(release.load(Ordering::SeqCst))
                    },
                )
                .unwrap()
        });
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let gate = GatedGitRunner::new(
            fx.runner()
                .clone()
                .with_timeout(std::time::Duration::from_millis(200)),
            GitTrust::Trusted,
        );
        let result = publish_candidate(&gate, fx.work(), &c, &GitCancelToken::new());
        // Always release the external writer, including on assertion failure.
        release.store(true, Ordering::SeqCst);
        assert_eq!(result, Err(PublishError::LockOwnershipUnconfirmed));
        assert!(writer.join().unwrap().success());
    });
    let actual = read_index_snapshot(&fx.gated(), fx.work()).unwrap();
    assert_eq!(actual.entries[0].oid, external);
    assert!(!fx.work().join(".git/index.lock").exists());
}

#[test]
fn locked_validation_excludes_external_writer_and_git_rolls_back_invalid_batch() {
    let fx = fixture("publish-invalid-batch");
    fx.write("selected", b"base").unwrap();
    fx.git(["add", "selected"]).unwrap();
    let before = index_bytes(&fx.work().join(".git/index")).unwrap();
    let oid = write_blob(&fx.gated(), fx.work(), b"candidate").unwrap();
    let input = format!("100644 {oid}\tselected\ninvalid final record\n");
    let mut checked = false;
    let result = protocol_runner(&fx.gated())
        .unwrap()
        .run_with_checked_input(
            &argv(["update-index", "--index-info"]),
            fx.work(),
            input.as_bytes(),
            &GitCancelToken::new(),
            &mut |pid, _, stderr| {
                if !index_read_ready(pid, stderr) {
                    return Ok(false);
                }
                if !ownership::owns_lock(pid, &fx.work().join(".git/index.lock")).unwrap() {
                    return Ok(false);
                }
                let other = fx
                    .gated()
                    .run_with_input(
                        &argv(["update-index", "--index-info"]),
                        fx.work(),
                        format!("100644 {oid}\texternal\n").as_bytes(),
                    )
                    .unwrap();
                assert!(!other.success());
                assert_eq!(index_bytes(&fx.work().join(".git/index")).unwrap(), before);
                checked = true;
                Ok(true)
            },
        )
        .unwrap();
    assert!(checked);
    assert!(!result.success());
    assert_eq!(index_bytes(&fx.work().join(".git/index")).unwrap(), before);
    assert!(!fx.work().join(".git/index.lock").exists());
}

#[test]
fn withheld_input_timeout_cancel_and_rejection_leave_index_exactly_unchanged() {
    for action in ["timeout", "cancel", "reject"] {
        let fx = fixture("publish-abort");
        fx.write("selected", b"base").unwrap();
        fx.git(["add", "selected"]).unwrap();
        let before = index_bytes(&fx.work().join(".git/index")).unwrap();
        let oid = write_blob(&fx.gated(), fx.work(), b"candidate").unwrap();
        let gate = GatedGitRunner::new(
            fx.runner()
                .clone()
                .with_timeout(std::time::Duration::from_millis(200)),
            GitTrust::Trusted,
        );
        let token = GitCancelToken::new();
        let mut seen = false;
        let result = protocol_runner(&gate).unwrap().run_with_checked_input(
            &argv(["update-index", "--index-info"]),
            fx.work(),
            format!("100644 {oid}\tselected\n").as_bytes(),
            &token,
            &mut |pid, _, stderr| {
                if !index_read_ready(pid, stderr) {
                    return Ok(false);
                }
                if ownership::owns_lock(pid, &fx.work().join(".git/index.lock")).unwrap() {
                    seen = true;
                    if action == "cancel" {
                        token.cancel();
                    }
                    if action == "reject" {
                        return Err(GitRunError::Cancelled);
                    }
                }
                Ok(false)
            },
        );
        assert!(seen);
        assert_eq!(
            result,
            Err(GatedRunError::Run(if action == "timeout" {
                GitRunError::Timeout
            } else {
                GitRunError::Cancelled
            }))
        );
        assert_eq!(index_bytes(&fx.work().join(".git/index")).unwrap(), before);
        assert!(!fx.work().join(".git/index.lock").exists());
    }
}

#[test]
fn untrusted_and_precancelled_publish_spawn_nothing() {
    let fx = fixture("publish-trust");
    let c = candidate(
        &fx,
        vec![upsert(&fx, "selected", FileMode::REGULAR, b"candidate")],
    );
    let gate = GatedGitRunner::new(fx.runner().clone(), GitTrust::NotTrusted);
    let count = gate.spawn_count();
    assert_eq!(
        publish_candidate(&gate, fx.work(), &c, &GitCancelToken::new()),
        Err(PublishError::Git(GatedRunError::ProjectNotTrusted.into()))
    );
    let cancelled = GitCancelToken::new();
    cancelled.cancel();
    assert_eq!(
        publish_candidate(&fx.gated(), fx.work(), &c, &cancelled),
        Err(PublishError::Git(
            GatedRunError::Run(GitRunError::Cancelled).into()
        ))
    );
    assert_eq!(gate.spawn_count(), count);
}

#[test]
fn writer_after_git_commit_is_diagnosed_and_never_rolled_back_over() {
    use crate::trace::{GitCommandKind, GitCommandTrace, GitTraceExit, GitTraceSink};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    struct Writer {
        gate: GatedGitRunner,
        root: PathBuf,
        oid: ObjectId,
        ran: AtomicBool,
    }
    impl GitTraceSink for Writer {
        fn record(&self, trace: &GitCommandTrace) {
            if trace.kind == GitCommandKind::UpdateIndex
                && trace.exit == GitTraceExit::Code(0)
                && !self.ran.swap(true, Ordering::SeqCst)
            {
                assert!(self
                    .gate
                    .run_with_input(
                        &argv(["update-index", "--index-info"]),
                        &self.root,
                        format!("100644 {}\tselected\n", self.oid).as_bytes()
                    )
                    .unwrap()
                    .success());
            }
        }
    }
    let fx = fixture("publish-after-commit-race");
    let c = candidate(
        &fx,
        vec![upsert(&fx, "selected", FileMode::REGULAR, b"candidate")],
    );
    let external = write_blob(&fx.gated(), fx.work(), b"external after commit").unwrap();
    let writer = Arc::new(Writer {
        gate: fx.gated(),
        root: fx.work().to_path_buf(),
        oid: external.clone(),
        ran: AtomicBool::new(false),
    });
    let gate = GatedGitRunner::new(
        fx.runner().clone().with_trace_sink(writer.clone()),
        GitTrust::Trusted,
    );
    assert_eq!(
        publish_candidate(&gate, fx.work(), &c, &GitCancelToken::new()),
        Err(PublishError::OutcomeUnknown { cause: None })
    );
    assert!(writer.ran.load(Ordering::SeqCst));
    assert_eq!(
        read_index_snapshot(&fx.gated(), fx.work()).unwrap().entries[0].oid,
        external
    );
}

#[test]
fn sha256_delete_and_index_versions_three_and_four() {
    let fx = fixture("publish-formats");
    let repo = fx.root().join("sha256");
    std::fs::create_dir(&repo).unwrap();
    assert!(fx
        .gated()
        .run(&argv(["init", "-q", "--object-format=sha256"]), &repo)
        .unwrap()
        .success());
    let oid = write_blob(&fx.gated(), &repo, b"base").unwrap();
    assert!(fx
        .gated()
        .run_with_input(
            &argv(["update-index", "--index-info"]),
            &repo,
            format!("100644 {oid}\told\n").as_bytes()
        )
        .unwrap()
        .success());
    let c = WrittenCandidate::for_test(
        read_index_snapshot(&fx.gated(), &repo).unwrap(),
        vec![WrittenFileDelta::Remove { path: path("old") }],
    );
    assert!(
        publish_candidate(&fx.gated(), &repo, &c, &GitCancelToken::new())
            .unwrap()
            .entries
            .is_empty()
    );
    for version in ["3", "4"] {
        let oid = write_blob(&fx.gated(), fx.work(), b"base").unwrap();
        set(&fx, "selected", &oid);
        fx.git(["update-index", "--index-version", version])
            .unwrap();
        let c = candidate(
            &fx,
            vec![upsert(&fx, "selected", FileMode::EXECUTABLE, b"candidate")],
        );
        assert_eq!(publish(&fx, &c).unwrap(), expected_index(&c).unwrap());
    }
}

#[test]
fn linked_worktree_uses_only_its_own_index_even_from_subdirectory() {
    let fx = fixture("publish-linked");
    fx.write("selected", b"base").unwrap();
    fx.commit_all("baseline").unwrap();
    let linked = fx.root().join("linked");
    fx.git([
        std::ffi::OsStr::new("worktree"),
        std::ffi::OsStr::new("add"),
        std::ffi::OsStr::new("--detach"),
        linked.as_os_str(),
    ])
    .unwrap();
    let main_index = index_bytes(&fx.work().join(".git/index")).unwrap();
    let c = WrittenCandidate::for_test(
        read_index_snapshot(&fx.gated(), &linked).unwrap(),
        vec![upsert(
            &fx,
            "selected",
            FileMode::REGULAR,
            b"linked candidate",
        )],
    );
    let subdir = linked.join("subdir");
    std::fs::create_dir(&subdir).unwrap();
    assert_eq!(
        publish_candidate(&fx.gated(), &subdir, &c, &GitCancelToken::new()).unwrap(),
        expected_index(&c).unwrap()
    );
    assert_eq!(
        index_bytes(&fx.work().join(".git/index")).unwrap(),
        main_index
    );
    assert_eq!(std::fs::read(linked.join("selected")).unwrap(), b"base");
}

#[test]
fn sparse_index_and_selected_intent_to_add_are_diagnosed() {
    let fx = fixture("publish-sparse");
    for p in ["selected", "keep/a", "hide/a"] {
        fx.write(p, b"base").unwrap();
    }
    fx.commit_all("baseline").unwrap();
    fx.git(["sparse-checkout", "init", "--cone", "--sparse-index"])
        .unwrap();
    fx.git(["sparse-checkout", "set", "keep"]).unwrap();
    let c = candidate(
        &fx,
        vec![upsert(&fx, "selected", FileMode::REGULAR, b"candidate")],
    );
    let before = index_bytes(&fx.work().join(".git/index")).unwrap();
    assert_eq!(
        publish(&fx, &c),
        Err(PublishError::Unsupported("sparse index"))
    );
    assert_eq!(index_bytes(&fx.work().join(".git/index")).unwrap(), before);
    fx.git(["sparse-checkout", "disable"]).unwrap();
    fx.write("ita", b"working").unwrap();
    fx.git(["add", "-N", "ita"]).unwrap();
    let c = candidate(
        &fx,
        vec![upsert(&fx, "ita", FileMode::REGULAR, b"candidate")],
    );
    let before = index_bytes(&fx.work().join(".git/index")).unwrap();
    assert!(matches!(
        publish(&fx, &c),
        Err(PublishError::Unsupported(_))
    ));
    assert_eq!(index_bytes(&fx.work().join(".git/index")).unwrap(), before);
}

#[test]
fn change_between_snapshot_and_lock_is_rejected_including_flags_only() {
    use std::os::unix::fs::PermissionsExt;
    for (target, external) in [
        ("selected", "selected"),
        ("d/a", "d"),
        ("d", "d/a"),
        ("selected", "flags"),
    ] {
        let fx = fixture("publish-before-lock-race");
        fx.write("selected", b"base").unwrap();
        fx.git(["add", "selected"]).unwrap();
        let c = candidate(
            &fx,
            vec![upsert(&fx, target, FileMode::REGULAR, b"candidate")],
        );
        let oid = write_blob(&fx.gated(), fx.work(), b"external").unwrap();
        let ready = fx.root().join("ready");
        let release = fx.root().join("release");
        let script = fx.root().join("delayed-git");
        let real_git = std::env::var_os("STELLAR_INDEX_PUBLISH_TEST_GIT")
            .map(PathBuf::from)
            .unwrap_or_else(|| crate::test_support::system_git().unwrap());
        let quote = |p: &Path| format!("'{}'", p.to_str().unwrap().replace('\'', "'\\''"));
        std::fs::write(&script, format!("#!/bin/sh\ncase \"$*\" in *--index-info*)\n: > {}\nwhile [ ! -e {} ]; do sleep 0.005; done;;\nesac\nexec {} \"$@\"\n", quote(&ready), quote(&release), quote(&real_git))).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let gate = GatedGitRunner::new(
            fx.runner().clone().with_test_executable(script),
            GitTrust::Trusted,
        );
        std::thread::scope(|scope| {
            let task =
                scope.spawn(|| publish_candidate(&gate, fx.work(), &c, &GitCancelToken::new()));
            let deadline = Instant::now() + std::time::Duration::from_secs(5);
            while !ready.exists() && Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            if !ready.exists() {
                std::fs::write(&release, b"").unwrap();
                panic!("child did not reach pre-lock barrier");
            }
            if external == "flags" {
                fx.git(["update-index", "--assume-unchanged", "selected"])
                    .unwrap();
            } else {
                set(&fx, external, &oid);
            }
            let before = index_bytes(&fx.work().join(".git/index")).unwrap();
            std::fs::write(&release, b"").unwrap();
            assert_eq!(task.join().unwrap(), Err(PublishError::StaleBase));
            assert_eq!(index_bytes(&fx.work().join(".git/index")).unwrap(), before);
        });
        assert!(!fx.work().join(".git/index.lock").exists());
    }
}

#[test]
fn mode_only_changes_and_deletions_keep_gitlinks_and_working_types() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let fx = fixture("publish-mode-only");
    fx.write("script", b"unchanged bytes").unwrap();
    let outside = fx.root().join("outside");
    std::fs::write(&outside, b"sentinel").unwrap();
    symlink(&outside, fx.work().join("link")).unwrap();
    fx.commit_all("baseline").unwrap();
    let head = fx.git(["rev-parse", "HEAD"]).unwrap().stdout;
    let head = std::str::from_utf8(&head).unwrap().trim();
    assert!(fx
        .gated()
        .run_with_input(
            &argv(["update-index", "--index-info"]),
            fx.work(),
            format!("160000 {head}\tmodule\n").as_bytes()
        )
        .unwrap()
        .success());
    let working_mode = std::fs::metadata(fx.work().join("script"))
        .unwrap()
        .permissions()
        .mode();
    let before = read_index_snapshot(&fx.gated(), fx.work()).unwrap();
    let script_oid = before
        .entries
        .iter()
        .find(|e| e.path == path("script"))
        .unwrap()
        .oid
        .clone();
    let gitlink = before
        .entries
        .iter()
        .find(|e| e.path == path("module"))
        .unwrap()
        .clone();
    let c = candidate(
        &fx,
        vec![
            upsert(&fx, "script", FileMode::EXECUTABLE, b"unchanged bytes"),
            upsert(&fx, "link", FileMode::REGULAR, b"index regular file"),
        ],
    );
    let staged = publish(&fx, &c).unwrap();
    assert_eq!(
        staged
            .entries
            .iter()
            .find(|e| e.path == path("script"))
            .unwrap()
            .oid,
        script_oid
    );
    assert_eq!(
        staged.entries.iter().find(|e| e.path == path("module")),
        Some(&gitlink)
    );
    let remove = candidate(
        &fx,
        vec![
            WrittenFileDelta::Remove {
                path: path("script"),
            },
            WrittenFileDelta::Remove { path: path("link") },
        ],
    );
    assert_eq!(publish(&fx, &remove).unwrap().entries, vec![gitlink]);
    assert_eq!(
        std::fs::read(fx.work().join("script")).unwrap(),
        b"unchanged bytes"
    );
    assert_eq!(
        std::fs::metadata(fx.work().join("script"))
            .unwrap()
            .permissions()
            .mode(),
        working_mode
    );
    assert_eq!(std::fs::read_link(fx.work().join("link")).unwrap(), outside);
    assert_eq!(std::fs::read(outside).unwrap(), b"sentinel");
    let unknown_flags =
        b"module\0  ctime: 0:0\n  mtime: 0:0\n  dev: 0\n  uid: 0\n  size: 0\tflags: 200000\n";
    assert_eq!(
        check_flags(unknown_flags, &c),
        Err(PublishError::Unsupported("검증되지 않은 index flags"))
    );
}

#[test]
fn receipt_restores_multiple_paths_modes_and_deletions_as_one_operation() {
    let fx = fixture("publish-restore");
    fx.write("d/a", b"original").unwrap();
    fx.write("other", b"keep").unwrap();
    fx.git(["add", "-A"]).unwrap();
    let c = candidate(
        &fx,
        vec![
            WrittenFileDelta::Remove { path: path("d/a") },
            upsert(&fx, "d", FileMode::SYMLINK, b"../target"),
            upsert(&fx, "exec", FileMode::EXECUTABLE, b"new"),
        ],
    );
    let receipt = publish_with_receipt(&fx.gated(), fx.work(), &c, &GitCancelToken::new()).unwrap();
    assert_eq!(receipt.snapshot(), &expected_index(&c).unwrap());
    assert_eq!(
        restore_published(&fx.gated(), receipt, &GitCancelToken::new()).unwrap(),
        *c.base_index()
    );
    assert_eq!(
        read_index_snapshot(&fx.gated(), fx.work()).unwrap(),
        *c.base_index()
    );
    assert_eq!(std::fs::read(fx.work().join("d/a")).unwrap(), b"original");
    assert!(!fx.work().join("exec").exists());
}

#[test]
fn receipt_rejects_external_content_and_flags_without_restoring_over_them() {
    for kind in ["selected", "unrelated", "flags"] {
        let fx = fixture("publish-restore-conflict");
        fx.write("selected", b"base").unwrap();
        fx.write("unrelated", b"base").unwrap();
        fx.git(["add", "-A"]).unwrap();
        let c = candidate(
            &fx,
            vec![upsert(&fx, "selected", FileMode::REGULAR, b"candidate")],
        );
        let receipt =
            publish_with_receipt(&fx.gated(), fx.work(), &c, &GitCancelToken::new()).unwrap();
        if kind == "flags" {
            fx.git(["update-index", "--assume-unchanged", "unrelated"])
                .unwrap();
        } else {
            let external = write_blob(&fx.gated(), fx.work(), b"external").unwrap();
            set(&fx, kind, &external);
        }
        let before = index_bytes(&fx.work().join(".git/index")).unwrap();
        assert_eq!(
            restore_published(&fx.gated(), receipt, &GitCancelToken::new()),
            Err(PublishError::StaleBase)
        );
        assert_eq!(index_bytes(&fx.work().join(".git/index")).unwrap(), before);
    }
}

#[test]
fn timeout_and_cancel_after_git_commit_issue_no_restore_receipt() {
    use std::os::unix::fs::PermissionsExt;
    for action in ["timeout", "cancel"] {
        let fx = fixture("publish-after-commit-cancel");
        fx.write("selected", b"working").unwrap();
        fx.git(["add", "selected"]).unwrap();
        let c = candidate(
            &fx,
            vec![upsert(&fx, "selected", FileMode::REGULAR, b"candidate")],
        );
        let hooks = fx.root().join("hooks");
        std::fs::create_dir(&hooks).unwrap();
        let ready = fx.root().join("committed");
        let hook = hooks.join("post-index-change");
        std::fs::write(
            &hook,
            format!("#!/bin/sh\n: > '{}'\nsleep 30\n", ready.display()),
        )
        .unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        fx.git([
            std::ffi::OsStr::new("config"),
            std::ffi::OsStr::new("core.hooksPath"),
            hooks.as_os_str(),
        ])
        .unwrap();
        let gate = GatedGitRunner::new(
            fx.runner()
                .clone()
                .with_timeout(std::time::Duration::from_secs(2)),
            GitTrust::Trusted,
        );
        let token = GitCancelToken::new();
        let result = std::thread::scope(|scope| {
            let job = scope.spawn(|| publish_with_receipt(&gate, fx.work(), &c, &token));
            let deadline = Instant::now() + std::time::Duration::from_secs(5);
            while !ready.exists() && Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            if action == "cancel" || !ready.exists() {
                token.cancel();
            }
            job.join().unwrap()
        });
        assert!(
            ready.exists(),
            "post-index-change hook did not run ({action}): {result:?}"
        );
        let Err(PublishError::OutcomeUnknown {
            cause: Some(GitOpError::Gate(GatedRunError::Run(error))),
        }) = result
        else {
            panic!("expected unconfirmed outcome: {result:?}")
        };
        assert_eq!(
            error,
            if action == "cancel" {
                GitRunError::Cancelled
            } else {
                GitRunError::Timeout
            }
        );
        assert_eq!(
            read_index_snapshot(&fx.gated(), fx.work()).unwrap(),
            expected_index(&c).unwrap()
        );
        assert_eq!(
            std::fs::read(fx.work().join("selected")).unwrap(),
            b"working"
        );
        assert!(!fx.work().join(".git/index.lock").exists());
    }
}

#[test]
fn configured_split_index_is_diagnosed_before_publish() {
    let fx = fixture("publish-split-config");
    fx.git(["config", "core.splitIndex", "true"]).unwrap();
    let before = index_bytes(&fx.work().join(".git/index")).unwrap();
    let c = candidate(
        &fx,
        vec![upsert(&fx, "selected", FileMode::REGULAR, b"candidate")],
    );
    assert!(matches!(
        publish_with_receipt(&fx.gated(), fx.work(), &c, &GitCancelToken::new()),
        Err(PublishError::Unsupported("split index"))
    ));
    assert_eq!(index_bytes(&fx.work().join(".git/index")).unwrap(), before);
    assert!(!fx.work().join(".git/index.lock").exists());
}

#[test]
fn configured_sparse_conversion_is_diagnosed_before_publish() {
    let fx = fixture("publish-sparse-config");
    for p in ["selected", "keep/a", "hide/a"] {
        fx.write(p, b"base").unwrap();
    }
    fx.commit_all("baseline").unwrap();
    fx.git(["sparse-checkout", "init", "--cone", "--no-sparse-index"])
        .unwrap();
    fx.git(["sparse-checkout", "set", "keep"]).unwrap();
    fx.git(["config", "--worktree", "index.sparse", "true"])
        .unwrap();
    let c = candidate(
        &fx,
        vec![upsert(&fx, "selected", FileMode::REGULAR, b"candidate")],
    );
    let before = index_bytes(&fx.work().join(".git/index")).unwrap();
    assert!(matches!(
        publish_with_receipt(&fx.gated(), fx.work(), &c, &GitCancelToken::new()),
        Err(PublishError::Unsupported("sparse index"))
    ));
    assert_eq!(index_bytes(&fx.work().join(".git/index")).unwrap(), before);
    assert!(!fx.work().join(".git/index.lock").exists());
    assert_eq!(
        fx.git(["config", "--get", "index.sparse"]).unwrap().stdout,
        b"true\n"
    );
}

#[test]
fn atomic_pipe_budget_covers_forward_and_inverse_before_any_index_update() {
    let fx = fixture("publish-pipe-budget");
    let oid = write_blob(&fx.gated(), fx.work(), b"blob").unwrap();
    let changes = |count| {
        (0..count)
            .map(|i| WrittenFileDelta::Upsert {
                path: path(&format!("file{i:04}")),
                mode: FileMode::REGULAR,
                oid: oid.clone(),
            })
            .collect()
    };
    let small = candidate(&fx, changes(8));
    assert_eq!(input(&small).len(), 456);
    let receipt =
        publish_with_receipt(&fx.gated(), fx.work(), &small, &GitCancelToken::new()).unwrap();
    assert_eq!(
        restore_published(&fx.gated(), receipt, &GitCancelToken::new()).unwrap(),
        *small.base_index()
    );
    let large = candidate(&fx, changes(9));
    assert_eq!(input(&large).len(), 513);
    let before = index_bytes(&fx.work().join(".git/index")).unwrap();
    assert!(matches!(
        publish(&fx, &large),
        Err(PublishError::Unsupported(_))
    ));
    assert_eq!(index_bytes(&fx.work().join(".git/index")).unwrap(), before);

    // 삭제 입력은 작아도 역방향 upsert가 한도를 넘으면 시작하지 않습니다.
    for i in 0..9 {
        fx.write(&format!("file{i:04}"), b"base").unwrap();
    }
    fx.git(["add", "-A"]).unwrap();
    let removal = candidate(
        &fx,
        (0..9)
            .map(|i| WrittenFileDelta::Remove {
                path: path(&format!("file{i:04}")),
            })
            .collect(),
    );
    assert!(input(&removal).len() < 512);
    let before = index_bytes(&fx.work().join(".git/index")).unwrap();
    assert!(matches!(
        publish(&fx, &removal),
        Err(PublishError::Unsupported(_))
    ));
    assert_eq!(index_bytes(&fx.work().join(".git/index")).unwrap(), before);
    assert!(!fx.work().join(".git/index.lock").exists());
}
