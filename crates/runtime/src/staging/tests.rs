use super::*;
use std::collections::BTreeMap;
use std::path::PathBuf;
use stellar_document::model::DocumentModel;
use stellar_git_porcelain::fixture::GitFixture;
use stellar_git_porcelain::object::FileModeKind;
use stellar_git_porcelain::snapshot::materialize_snapshot;
use stellar_git_porcelain::snapshot::read_index_snapshot;
use stellar_git_porcelain::trust::GatedGitRunner;
use stellar_ops::candidate::{saved_semantic_snapshot, CandidateStorageError, SemanticDiff};
use stellar_ops::candidate::{ChangeId, StageSelectionRequest};
use stellar_ops::test_support::{
    file_digest, large_document, save_text_document, scaffold_project,
};
use stellar_project::saved_project::SavedProjectLoader;

/// materialize한 후보·index tree를 원본으로 만든 cache로 엽니다(Git tree에는 cache가 없음, ADR-0122).
fn open_tree(
    root: &std::path::Path,
) -> Result<
    stellar_project::saved_project::SavedProject,
    stellar_project::saved_project::SavedProjectError,
> {
    let _ = stellar_project::cache::prepare_snapshot(root);
    SavedProjectLoader::for_materialized_snapshot().open(root)
}

#[test]
fn selected_property_round_trips_without_changing_working_bytes_or_real_index() {
    // chunk cache는 Git 밖이라(ADR-0122) 후보 index에는 텍스트 원본만 바뀌어 들어갑니다.
    let fx = GitFixture::with_system_git("candidate-storage")
        .unwrap()
        .expect("system Git required");
    scaffold_project(fx.work());
    let model = large_document(4);
    save_text_document(fx.work(), &model);
    save_text_document(fx.work(), &model);
    fx.write("stellar.lock.yaml", b"opaque-lock: keep-exactly\n")
        .unwrap();
    let manifest = std::fs::read(fx.work().join("stellar.yaml")).unwrap();
    let mut opaque_manifest = manifest.clone();
    opaque_manifest.extend_from_slice(b"\n# opaque metadata spelling must survive\n");
    fx.write("stellar.yaml", &opaque_manifest).unwrap();
    fx.commit_all("fixture baseline").unwrap();
    let tracked = String::from_utf8(fx.git(["ls-files"]).unwrap().stdout).unwrap();
    assert!(
        !tracked.lines().any(|l| l.starts_with(".stellar/")),
        "{tracked}"
    );
    let view_file = tracked
        .lines()
        .find(|l| l.starts_with("design/pages/") && l.ends_with(".stellar.yaml"))
        .unwrap()
        .to_string();
    // HEAD 이후 이미 stage된 non-Stellar bytes와 원본 파일 mode도 그대로 보존합니다.
    fx.write("README.txt", b"already staged bytes\n").unwrap();
    fx.git(["add", "README.txt"]).unwrap();
    fx.git(["update-index", "--chmod=+x", view_file.as_str()])
        .unwrap();
    let mut records = model.to_records().unwrap();
    let node = records
        .nodes
        .iter_mut()
        .find(|n| n.parent.is_none())
        .unwrap();
    let id = node.id;
    node.display_name = "selected A".into();
    let expected = records.clone();
    records
        .nodes
        .iter_mut()
        .find(|n| n.id == id)
        .unwrap()
        .locked = true;
    let (working, _) = DocumentModel::from_records(records).unwrap();
    save_text_document(fx.work(), &working);
    fx.write("stellar.lock.yaml", b"unselected working lock\n")
        .unwrap();
    fx.write("README.txt", b"unstaged non-Stellar\n").unwrap();
    let before = file_digest(fx.work());
    let index_bytes = std::fs::read(fx.work().join(".git/index")).unwrap();
    let index = read_index_snapshot(&fx.gated(), fx.work()).unwrap();
    let parent = fx.root().join("candidates");
    std::fs::create_dir(&parent).unwrap();
    let request = StageSelectionRequest::stage([ChangeId::property(
        stellar_core_types::property::PropertyPath::with_keys(
            id,
            vec![stellar_ops::candidate::DISPLAY_NAME],
        ),
    )]);
    let candidate = prepare_stage_candidate(&fx.gated(), fx.work(), &parent, &request).unwrap();
    // 후보 tree(Git 파일)에는 cache 경로가 없습니다. 열 때는 원본에서 cache를 만듭니다.
    assert!(!candidate
        .tree
        .files()
        .iter()
        .any(|f| f.path.as_bytes().starts_with(b".stellar/")));
    let reopened = open_tree(candidate.tree.root()).unwrap();
    let mut actual = reopened
        .load_document(model.document_id())
        .unwrap()
        .model
        .to_records()
        .unwrap();
    actual.sort_canonical();
    let mut expected = expected;
    expected.sort_canonical();
    assert_eq!(actual, expected);
    assert_eq!(
        std::fs::read(candidate.tree.root().join("stellar.yaml")).unwrap(),
        opaque_manifest
    );
    assert_eq!(
        std::fs::read(candidate.tree.root().join("stellar.lock.yaml")).unwrap(),
        b"opaque-lock: keep-exactly\n"
    );
    assert_eq!(
        std::fs::read(candidate.tree.root().join("README.txt")).unwrap(),
        b"already staged bytes\n"
    );
    let repeated = prepare_stage_candidate(&fx.gated(), fx.work(), &parent, &request).unwrap();
    assert_eq!(
        file_digest(repeated.tree.root())
            .into_iter()
            .filter(|(path, _)| !path.starts_with(".stellar"))
            .collect::<std::collections::BTreeMap<_, _>>(),
        file_digest(candidate.tree.root())
            .into_iter()
            .filter(|(path, _)| !path.starts_with(".stellar"))
            .collect::<std::collections::BTreeMap<_, _>>()
    );
    let delta = stellar_git_porcelain::candidate_delta::compute_candidate_delta(
        &fx.gated(),
        fx.work(),
        &repeated.tree,
    )
    .unwrap();
    // A가 든 View 원본 파일만 바뀌며 다른 파일·기존 staged 파일·cache는 제외됩니다.
    assert_eq!(delta.changes().len(), 1, "{:?}", delta.changes());
    assert!(delta.changes().iter().all(|change| {
        matches!(
            change,
            stellar_git_porcelain::candidate_delta::FileDelta::Upsert { .. }
        ) && change.path().as_bytes() == view_file.as_bytes()
    }));
    assert_eq!(delta.base_index(), &index);
    assert_eq!(file_digest(fx.work()), before);
    let gate = fx.gated();
    let before_write = gate.spawn_count();
    let written =
        stellar_git_porcelain::blob_write::write_candidate_blobs(&gate, fx.work(), &delta).unwrap();
    assert_eq!(gate.spawn_count() - before_write, 1);
    assert_eq!(written.base_index(), &index);
    for (file, stored) in delta.changes().iter().zip(written.changes()) {
        let stellar_git_porcelain::candidate_delta::FileDelta::Upsert { path, mode, bytes } = file
        else {
            panic!("unexpected deletion");
        };
        let stellar_git_porcelain::blob_write::WrittenFileDelta::Upsert {
            path: actual_path,
            mode: actual_mode,
            oid,
        } = stored
        else {
            panic!("unexpected deletion");
        };
        assert_eq!(path, actual_path);
        assert_eq!(mode, actual_mode);
        assert_eq!(
            fx.git(["cat-file", "blob", oid.as_str()]).unwrap().stdout,
            *bytes
        );
    }
    let after_blobs = file_digest(fx.work());
    for (path, hash) in &before {
        assert_eq!(after_blobs.get(path), Some(hash), "existing bytes: {path}");
    }
    assert!(after_blobs
        .keys()
        .filter(|path| !before.contains_key(*path))
        .all(|path| path.starts_with(".git/objects/")));
    assert_eq!(
        std::fs::read(fx.work().join(".git/index")).unwrap(),
        index_bytes
    );
    assert_eq!(read_index_snapshot(&fx.gated(), fx.work()).unwrap(), index);
    assert!(!candidate.tree.root().join(".git").exists());
    assert_eq!(
        repeated
            .tree
            .files()
            .iter()
            .find(|f| f.path.as_bytes() == view_file.as_bytes())
            .unwrap()
            .kind,
        FileModeKind::Executable
    );
    drop(candidate);
    drop(repeated);
    assert_eq!(std::fs::read_dir(parent).unwrap().count(), 0);
}

#[test]
fn no_op_uses_index_generation_and_rejects_worktree_temp_parent_and_untrusted_git() {
    use stellar_git_porcelain::trust::GitTrust;
    let fx = GitFixture::with_system_git("candidate-noop")
        .unwrap()
        .expect("system Git required");
    scaffold_project(fx.work());
    let model = large_document(3);
    save_text_document(fx.work(), &model);
    fx.commit_all("fixture baseline").unwrap();
    let mut changed = model.to_records().unwrap();
    changed.nodes[0].display_name = "already staged".into();
    let (changed, _) = DocumentModel::from_records(changed).unwrap();
    save_text_document(fx.work(), &changed);
    fx.git(["add", "."]).unwrap();
    let index_files = file_digest(fx.work());
    save_text_document(fx.work(), &model);
    let before = file_digest(fx.work());
    let request = StageSelectionRequest::stage([]);
    let parent = fx.root().join("candidates");
    std::fs::create_dir(&parent).unwrap();
    let candidate = prepare_stage_candidate(&fx.gated(), fx.work(), &parent, &request).unwrap();
    let reopened = open_tree(candidate.tree.root()).unwrap();
    // 후보 tree의 cache는 원본에서 새로 만든 1세대입니다(ADR-0122).
    assert_eq!(reopened.generation_number(), 1);
    assert_eq!(
        reopened
            .load_document(model.document_id())
            .unwrap()
            .model
            .to_records()
            .unwrap(),
        changed.to_records().unwrap()
    );
    let candidate_files = file_digest(candidate.tree.root());
    for (path, hash) in candidate_files
        .iter()
        .filter(|(p, _)| !p.starts_with(".stellar"))
    {
        assert_eq!(index_files.get(path), Some(hash), "{path}");
    }
    let delta = stellar_git_porcelain::candidate_delta::compute_candidate_delta(
        &fx.gated(),
        fx.work(),
        &candidate.tree,
    )
    .unwrap();
    assert!(delta.changes().is_empty());
    let gate = fx.gated();
    let before_write = gate.spawn_count();
    assert!(
        stellar_git_porcelain::blob_write::write_candidate_blobs(&gate, fx.work(), &delta)
            .unwrap()
            .changes()
            .is_empty()
    );
    assert_eq!(gate.spawn_count(), before_write);
    drop(candidate);
    assert!(matches!(
        prepare_stage_candidate(&fx.gated(), fx.work(), fx.work(), &request),
        Err(PrepareStageError::Storage(
            CandidateStorageError::Unsupported(_)
        ))
    ));
    let mut git = fx.gated();
    git.set_trust(GitTrust::NotTrusted);
    let spawns = git.spawn_count();
    assert!(matches!(
        prepare_stage_candidate(&git, fx.work(), &parent, &request),
        Err(PrepareStageError::Git(_))
    ));
    assert_eq!(git.spawn_count(), spawns);
    assert_eq!(file_digest(fx.work()), before);
    assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 0);
}

#[test]
fn stage_preview_rejects_changed_name_with_same_id_and_accepts_independent_edit() {
    let fx = GitFixture::with_system_git("stage-preview")
        .unwrap()
        .expect("system Git required");
    scaffold_project(fx.work());
    let doc = large_document(4).to_records().unwrap();
    let root = doc.nodes.iter().find(|n| n.parent.is_none()).unwrap().id;
    let save = |name: &str, locked| {
        let mut records = doc.clone();
        let node = records.nodes.iter_mut().find(|n| n.id == root).unwrap();
        node.display_name = name.into();
        node.locked = locked;
        let (model, _) = DocumentModel::from_records(records).unwrap();
        save_text_document(fx.work(), &model);
    };
    save("A", false);
    fx.commit_all("baseline A").unwrap();
    let parent = fx.root().join("candidates");
    std::fs::create_dir(&parent).unwrap();
    let request = StageSelectionRequest::stage([ChangeId::property(
        stellar_core_types::property::PropertyPath::with_keys(
            root,
            vec![stellar_ops::candidate::DISPLAY_NAME],
        ),
    )]);
    let git = fx.gated();
    save("B", false);
    let preview = prepare_stage_candidate(&git, fx.work(), &parent, &request)
        .unwrap()
        .semantic
        .applied()
        .clone();

    save("C", false);
    let before = file_digest(fx.work());
    assert!(matches!(
        prepare_stage_candidate_with_preview(&git, fx.work(), &parent, &request, Some(&preview)),
        Err(PrepareStageError::StalePreview)
    ));
    assert_eq!(file_digest(fx.work()), before);
    assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 0);

    let refreshed = prepare_stage_candidate(&git, fx.work(), &parent, &request)
        .unwrap()
        .semantic
        .applied()
        .clone();
    assert_ne!(preview, refreshed);
    // 사용자가 C를 본 뒤 같은 청크의 다른 속성만 바뀌었습니다.
    save("C", true);
    let prepared =
        prepare_stage_candidate_with_preview(&git, fx.work(), &parent, &request, Some(&refreshed))
            .unwrap();
    assert_eq!(prepared.semantic.applied(), &refreshed);
}

#[cfg(target_os = "macos")]
#[test]
fn actual_partial_stage_reopens_index_and_preserves_working_bytes_and_memory_revision() {
    use stellar_commands::state::DocumentState;
    use stellar_core_types::revision::DocumentRevision;
    use stellar_git_porcelain::cancel::GitCancelToken;
    let fx = GitFixture::with_system_git("actual-semantic-stage")
        .unwrap()
        .expect("system Git required");
    scaffold_project(fx.work());
    let model = large_document(4);
    save_text_document(fx.work(), &model);
    fx.commit_all("baseline").unwrap();
    fx.write("README.txt", b"unrelated staged").unwrap();
    fx.git(["add", "README.txt"]).unwrap();
    let mut records = model.to_records().unwrap();
    let root = records
        .nodes
        .iter_mut()
        .find(|n| n.parent.is_none())
        .unwrap();
    let id = root.id;
    root.display_name = "selected A".into();
    root.locked = true; // unselected B in the same chunk
    let (working, _) = DocumentModel::from_records(records).unwrap();
    save_text_document(fx.work(), &working);
    fx.write("README.txt", b"unrelated working").unwrap();
    let memory = DocumentState::with_revision(working, DocumentRevision(7));
    let memory_before = memory.model().to_records().unwrap();
    let working_before: std::collections::BTreeMap<_, _> = file_digest(fx.work())
        .into_iter()
        .filter(|(p, _)| !p.starts_with(".git/"))
        .collect();
    let parent = fx.root().join("candidates");
    std::fs::create_dir(&parent).unwrap();
    let request = StageSelectionRequest::stage([ChangeId::property(
        stellar_core_types::property::PropertyPath::with_keys(
            id,
            vec![stellar_ops::candidate::DISPLAY_NAME],
        ),
    )]);
    let preview = prepare_stage_candidate(&fx.gated(), fx.work(), &parent, &request)
        .unwrap()
        .semantic
        .applied()
        .clone();
    let prepared = prepare_stage_candidate_with_preview(
        &fx.gated(),
        fx.work(),
        &parent,
        &request,
        Some(&preview),
    )
    .unwrap();
    let actual = apply_stage_candidate(
        &fx.gated(),
        fx.work(),
        &parent,
        &prepared,
        &GitCancelToken::new(),
    )
    .unwrap();
    assert_eq!(actual, read_index_snapshot(&fx.gated(), fx.work()).unwrap());
    let actual_tree =
        materialize_snapshot(&fx.gated(), fx.work(), &actual.entries, &parent).unwrap();
    let loaded = open_tree(actual_tree.root()).unwrap();
    assert_eq!(
        &saved_semantic_snapshot(&loaded).unwrap(),
        prepared.semantic.snapshot()
    );
    let actual_model = loaded
        .load_document(model.document_id())
        .unwrap()
        .model
        .to_records()
        .unwrap();
    let selected = actual_model.nodes.iter().find(|n| n.id == id).unwrap();
    assert_eq!(selected.display_name, "selected A");
    assert!(!selected.locked);
    assert_eq!(
        std::fs::read(actual_tree.root().join("README.txt")).unwrap(),
        b"unrelated staged"
    );
    let working_after: std::collections::BTreeMap<_, _> = file_digest(fx.work())
        .into_iter()
        .filter(|(p, _)| !p.starts_with(".git/"))
        .collect();
    assert_eq!(working_after, working_before);
    assert_eq!(memory.revision(), DocumentRevision(7));
    assert_eq!(memory.model().to_records().unwrap(), memory_before);
}

#[cfg(target_os = "macos")]
#[test]
fn unstage_only_the_selected_name_from_index_even_when_working_is_different() {
    use stellar_git_porcelain::cancel::GitCancelToken;

    let fx = GitFixture::with_system_git("actual-semantic-unstage")
        .unwrap()
        .unwrap();
    scaffold_project(fx.work());
    let model = large_document(4);
    save_text_document(fx.work(), &model);
    fx.commit_all("baseline").unwrap();

    let mut records = model.to_records().unwrap();
    let root = records
        .nodes
        .iter_mut()
        .find(|r| r.parent.is_none())
        .unwrap();
    let id = root.id;
    let original_name = root.display_name.clone();
    root.display_name = "staged name".into();
    root.locked = true;
    let (staged, _) = DocumentModel::from_records(records).unwrap();
    save_text_document(fx.work(), &staged);
    fx.write("notes.txt", b"staged notes").unwrap();
    fx.git(["add", "."]).unwrap();

    // Working Tree를 재열 수도 없는 상태에서 index의 두 속성 중 이름만 되돌립니다.
    fx.write("design/current", b"concurrent incomplete working save")
        .unwrap();
    fx.write("notes.txt", b"working notes").unwrap();
    let working_files = || {
        file_digest(fx.work())
            .into_iter()
            .filter(|(p, _)| !p.starts_with(".git/"))
            .collect::<BTreeMap<_, _>>()
    };
    let before = working_files();
    let parent = fx.root().join("candidate");
    std::fs::create_dir(&parent).unwrap();
    let change = ChangeId::property(stellar_core_types::property::PropertyPath::with_keys(
        id,
        vec![stellar_ops::candidate::DISPLAY_NAME],
    ));
    let request = StageSelectionRequest::unstage([change]);
    let git = fx.gated();
    let preview = prepare_unstage_candidate(&git, fx.work(), &parent, &request, None)
        .unwrap()
        .semantic
        .applied()
        .inverse();
    let index_before = std::fs::read(fx.work().join(".git/index")).unwrap();
    assert!(matches!(
        prepare_unstage_candidate(
            &git,
            fx.work(),
            &parent,
            &request,
            Some(&SemanticDiff::default())
        ),
        Err(PrepareStageError::StalePreview)
    ));
    let prepare =
        || prepare_unstage_candidate(&git, fx.work(), &parent, &request, Some(&preview)).unwrap();
    let expired = prepare();
    // 같은 tree인 빈 commit도 이미 준비한 HEAD 기준을 무효화합니다.
    let original_head = fx.git(["rev-parse", "HEAD"]).unwrap();
    let tree_oid = fx.git(["rev-parse", "HEAD^{tree}"]).unwrap();
    let new_head = fx
        .git([
            "commit-tree",
            std::str::from_utf8(&tree_oid.stdout).unwrap().trim(),
            "-p",
            "HEAD",
            "-m",
            "advancing ref",
        ])
        .unwrap();
    fx.git([
        "update-ref",
        "HEAD",
        std::str::from_utf8(&new_head.stdout).unwrap().trim(),
        std::str::from_utf8(&original_head.stdout).unwrap().trim(),
    ])
    .unwrap();
    assert!(matches!(
        apply_stage_candidate(&git, fx.work(), &parent, &expired, &GitCancelToken::new()),
        Err(ApplyStageError::Preparation(PrepareStageError::HeadChanged))
    ));
    assert_eq!(
        std::fs::read(fx.work().join(".git/index")).unwrap(),
        index_before
    );

    let prepared = prepare();
    let actual =
        apply_stage_candidate(&git, fx.work(), &parent, &prepared, &GitCancelToken::new()).unwrap();
    let tree = materialize_snapshot(&git, fx.work(), &actual.entries, &parent).unwrap();
    let loaded = open_tree(tree.root()).unwrap();
    let reopened = loaded
        .load_document(model.document_id())
        .unwrap()
        .model
        .to_records()
        .unwrap();
    let root = reopened.nodes.iter().find(|r| r.id == id).unwrap();
    assert_eq!(root.display_name, original_name);
    assert!(root.locked);
    assert_eq!(
        std::fs::read(tree.root().join("notes.txt")).unwrap(),
        b"staged notes"
    );
    assert_eq!(working_files(), before);
}

#[cfg(target_os = "macos")]
fn apply_fixture(
    tag: &str,
) -> (
    GitFixture,
    PathBuf,
    PreparedStageCandidate,
    PreparedStageCandidate,
) {
    let fx = GitFixture::with_system_git(tag)
        .unwrap()
        .expect("system Git required");
    scaffold_project(fx.work());
    let model = large_document(4);
    save_text_document(fx.work(), &model);
    fx.commit_all("baseline").unwrap();
    let mut records = model.to_records().unwrap();
    let root = records
        .nodes
        .iter_mut()
        .find(|n| n.parent.is_none())
        .unwrap();
    let id = root.id;
    root.display_name = "selected".into();
    let (working, _) = DocumentModel::from_records(records).unwrap();
    save_text_document(fx.work(), &working);
    let parent = fx.root().join("candidate");
    std::fs::create_dir(&parent).unwrap();
    let request = StageSelectionRequest::stage([ChangeId::property(
        stellar_core_types::property::PropertyPath::with_keys(
            id,
            vec![stellar_ops::candidate::DISPLAY_NAME],
        ),
    )]);
    let selected = prepare_stage_candidate(&fx.gated(), fx.work(), &parent, &request).unwrap();
    let unchanged = prepare_stage_candidate(
        &fx.gated(),
        fx.work(),
        &parent,
        &StageSelectionRequest::stage([]),
    )
    .unwrap();
    (fx, parent, selected, unchanged)
}

#[cfg(target_os = "macos")]
#[test]
fn actual_index_semantic_mismatch_restores_original_tree() {
    use stellar_git_porcelain::cancel::GitCancelToken;
    let (fx, parent, selected, unchanged) = apply_fixture("semantic-restore");
    let before = read_index_snapshot(&fx.gated(), fx.work()).unwrap();
    let working_before: std::collections::BTreeMap<_, _> = file_digest(fx.work())
        .into_iter()
        .filter(|(p, _)| !p.starts_with(".git/"))
        .collect();
    // 검증 후 내부 후보 대응이 잘못된 경우를 주입해 실제 publish 이후 비교·복원까지 실행합니다.
    let wrong = selected.with_semantic_for_test(unchanged.semantic);
    let result = apply_stage_candidate(
        &fx.gated(),
        fx.work(),
        &parent,
        &wrong,
        &GitCancelToken::new(),
    );
    assert!(
        matches!(result, Err(ApplyStageError::Verification { cause, rollback: Ok(()) }) if matches!(*cause, PrepareStageError::ReopenMismatch))
    );
    assert_eq!(read_index_snapshot(&fx.gated(), fx.work()).unwrap(), before);
    let working_after: std::collections::BTreeMap<_, _> = file_digest(fx.work())
        .into_iter()
        .filter(|(p, _)| !p.starts_with(".git/"))
        .collect();
    assert_eq!(working_after, working_before);
    assert_eq!(std::fs::read_dir(parent).unwrap().count(), 2); // 두 준비 후보 외 검증 temp는 정리됨
}

#[cfg(target_os = "macos")]
#[test]
fn writer_during_semantic_verification_returns_restore_conflict_and_preserves_writer() {
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };
    use stellar_git_porcelain::{
        blob_write::write_blob,
        cancel::GitCancelToken,
        index_publish::PublishError,
        object::ObjectId,
        runner::argv,
        trace::{GitCommandKind, GitCommandTrace, GitTraceExit, GitTraceSink},
        trust::GitTrust,
    };
    struct Writer {
        gate: GatedGitRunner,
        root: PathBuf,
        oid: ObjectId,
        armed: AtomicBool,
        reads: AtomicUsize,
    }
    impl GitTraceSink for Writer {
        fn record(&self, trace: &GitCommandTrace) {
            if trace.kind == GitCommandKind::UpdateIndex && trace.exit == GitTraceExit::Code(0) {
                self.armed.store(true, Ordering::SeqCst);
            }
            if trace.kind == GitCommandKind::LsFiles
                && self.armed.load(Ordering::SeqCst)
                && self.reads.fetch_add(1, Ordering::SeqCst) == 1
            {
                assert!(self
                    .gate
                    .run_with_input(
                        &argv(["update-index", "--index-info"]),
                        &self.root,
                        format!("100644 {}\tREADME.external\n", self.oid).as_bytes()
                    )
                    .unwrap()
                    .success());
            }
        }
    }
    let (fx, parent, prepared, _unchanged) = apply_fixture("semantic-restore-conflict");
    let external = write_blob(&fx.gated(), fx.work(), b"external during verification").unwrap();
    let writer = Arc::new(Writer {
        gate: fx.gated(),
        root: fx.work().to_path_buf(),
        oid: external.clone(),
        armed: AtomicBool::new(false),
        reads: AtomicUsize::new(0),
    });
    let gate = GatedGitRunner::new(
        fx.runner().clone().with_trace_sink(writer.clone()),
        GitTrust::Trusted,
    );
    let result =
        apply_stage_candidate(&gate, fx.work(), &parent, &prepared, &GitCancelToken::new());
    assert!(
        matches!(result, Err(ApplyStageError::Verification { cause, rollback:Err(PublishError::StaleBase) }) if matches!(*cause, PrepareStageError::IndexChanged))
    );
    assert!(writer.reads.load(Ordering::SeqCst) >= 2);
    let actual = read_index_snapshot(&fx.gated(), fx.work()).unwrap();
    assert_eq!(
        actual
            .entries
            .iter()
            .find(|e| e.path.as_bytes() == b"README.external")
            .unwrap()
            .oid,
        external
    );
    assert!(!fx.work().join("README.external").exists());
}

#[cfg(target_os = "macos")]
#[test]
fn rejected_cancelled_and_restored_stage_preserve_working_tree_and_document_revision() {
    use stellar_commands::state::DocumentState;
    use stellar_core_types::revision::DocumentRevision;
    use stellar_git_porcelain::{
        blob_write::write_blob, cancel::GitCancelToken, runner::argv, trust::GitTrust,
    };
    for scenario in [
        "cancel",
        "untrusted",
        "stale",
        "bad-temp",
        "nested-root",
        "mismatch",
    ] {
        let (fx, parent, mut prepared, unchanged) = apply_fixture("stage-invariants");
        let loaded = SavedProjectLoader::new().open(fx.work()).unwrap();
        let document_id = loaded.document_ids().unwrap()[0];
        let memory = DocumentState::with_revision(
            loaded.load_document(document_id).unwrap().model,
            DocumentRevision(19),
        );
        let memory_before = memory.model().to_records().unwrap();
        let mut gate = fx.gated();
        let cancel = GitCancelToken::new();
        let mut worktree = fx.work().to_path_buf();
        let mut temp_parent = parent.clone();
        match scenario {
            "cancel" => cancel.cancel(),
            "untrusted" => gate.set_trust(GitTrust::NotTrusted),
            "stale" => {
                let oid = write_blob(&fx.gated(), fx.work(), b"external staged").unwrap();
                assert!(fx
                    .gated()
                    .run_with_input(
                        &argv(["update-index", "--index-info"]),
                        fx.work(),
                        format!("100644 {oid}\tREADME.external\n").as_bytes()
                    )
                    .unwrap()
                    .success());
            }
            "bad-temp" => temp_parent = fx.work().to_path_buf(),
            "nested-root" => {
                worktree.push("nested");
                std::fs::create_dir(&worktree).unwrap();
            }
            "mismatch" => prepared = prepared.with_semantic_for_test(unchanged.semantic),
            _ => unreachable!(),
        }
        let index_before = read_index_snapshot(&fx.gated(), fx.work()).unwrap();
        let index_bytes = std::fs::read(fx.work().join(".git/index")).unwrap();
        let working_before: std::collections::BTreeMap<_, _> = file_digest(fx.work())
            .into_iter()
            .filter(|(p, _)| !p.starts_with(".git/"))
            .collect();
        let spawns = gate.spawn_count();
        let result = apply_stage_candidate(&gate, &worktree, &temp_parent, &prepared, &cancel);
        assert!(result.is_err(), "{scenario}");
        if matches!(scenario, "cancel" | "untrusted") {
            assert_eq!(gate.spawn_count(), spawns, "{scenario}");
        }
        if scenario == "mismatch" {
            assert!(matches!(
                result,
                Err(ApplyStageError::Verification {
                    rollback: Ok(()),
                    ..
                })
            ));
        } else {
            assert_eq!(
                std::fs::read(fx.work().join(".git/index")).unwrap(),
                index_bytes,
                "{scenario}"
            );
        }
        assert_eq!(
            read_index_snapshot(&fx.gated(), fx.work()).unwrap(),
            index_before,
            "{scenario}"
        );
        let working_after: std::collections::BTreeMap<_, _> = file_digest(fx.work())
            .into_iter()
            .filter(|(p, _)| !p.starts_with(".git/"))
            .collect();
        assert_eq!(working_after, working_before, "{scenario}");
        assert_eq!(memory.revision(), DocumentRevision(19), "{scenario}");
        assert_eq!(
            memory.model().to_records().unwrap(),
            memory_before,
            "{scenario}"
        );
        assert_eq!(std::fs::read_dir(parent).unwrap().count(), 2, "{scenario}");
        assert!(!fx.work().join(".git/index.lock").exists(), "{scenario}");
    }
}
