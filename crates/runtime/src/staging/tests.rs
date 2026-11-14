use super::*;
use stellar_document::model::DocumentModel;
use stellar_git_porcelain::fixture::GitFixture;
use stellar_ops::candidate::{ChangeId, StageSelectionRequest};
use stellar_ops::test_support::{
    file_digest, large_document, random_valid_document, save_documents, scaffold_project,
};

#[test]
fn selected_property_round_trips_without_changing_working_bytes_or_real_index() {
    let fx = GitFixture::with_system_git("candidate-storage")
        .unwrap()
        .expect("system Git required");
    scaffold_project(fx.work());
    let model = large_document(4);
    let other = random_valid_document(42);
    save_documents(fx.work(), &[&model, &other]);
    save_documents(fx.work(), &[&model, &other]);
    fx.write("stellar.lock.yaml", b"opaque-lock: keep-exactly\n")
        .unwrap();
    let manifest = std::fs::read(fx.work().join("stellar.yaml")).unwrap();
    let mut opaque_manifest = manifest.clone();
    opaque_manifest.extend_from_slice(b"\n# opaque metadata spelling must survive\n");
    fx.write("stellar.yaml", &opaque_manifest).unwrap();
    fx.commit_all("fixture baseline").unwrap();
    // HEAD 이후 이미 stage된 non-Stellar bytes도 그대로 보존합니다.
    fx.write("README.txt", b"already staged bytes\n").unwrap();
    fx.git(["add", "README.txt"]).unwrap();
    fx.git(["update-index", "--chmod=+x", "design/current"])
        .unwrap();
    let index_files = file_digest(fx.work());
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
    save_documents(fx.work(), &[&working, &other]);
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
    let reopened = SavedProjectLoader::for_materialized_snapshot()
        .open(candidate.tree.root())
        .unwrap();
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
        reopened
            .load_document(other.document_id())
            .unwrap()
            .model
            .to_records()
            .unwrap(),
        other.to_records().unwrap()
    );
    assert_eq!(reopened.generation_number(), 3);
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
    let candidate_files = file_digest(candidate.tree.root());
    for (path, hash) in &index_files {
        if path.starts_with("design/") && path != "design/current" {
            assert_eq!(
                candidate_files.get(path),
                Some(hash),
                "old generation/object {path}"
            );
        }
    }
    let repeated = prepare_stage_candidate(&fx.gated(), fx.work(), &parent, &request).unwrap();
    assert_eq!(candidate_files, file_digest(repeated.tree.root()));
    let delta = stellar_git_porcelain::candidate_delta::compute_candidate_delta(
        &fx.gated(),
        fx.work(),
        &candidate.tree,
    )
    .unwrap();
    // A가 든 chunk와 새 generation, current만 바뀌며 다른 문서·기존 staged 파일은 제외됩니다.
    assert_eq!(delta.changes().len(), 3);
    assert!(delta.changes().iter().all(|change| {
        matches!(
            change,
            stellar_git_porcelain::candidate_delta::FileDelta::Upsert { .. }
        ) && change.path().as_bytes().starts_with(b"design/")
    }));
    assert_eq!(delta.base_index(), &index);
    assert_eq!(file_digest(fx.work()), before);
    let gate = fx.gated();
    let before_write = gate.spawn_count();
    let written =
        stellar_git_porcelain::blob_write::write_candidate_blobs(&gate, fx.work(), &delta).unwrap();
    assert_eq!(gate.spawn_count() - before_write, 3);
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
        candidate
            .tree
            .files()
            .iter()
            .find(|f| f.path.as_bytes() == b"design/current")
            .unwrap()
            .kind,
        FileModeKind::Executable
    );
    // 현재 manifest가 참조하지 않는 opaque 파일도 후보 생성 때문에 덮어쓰면 안 됩니다.
    let collision = candidate
        .tree
        .files()
        .iter()
        .find(|f| {
            f.path.as_bytes().starts_with(b"design/objects/")
                && matches!(
                    f.origin,
                    stellar_git_porcelain::candidate_tree::CandidateOrigin::Bytes
                )
        })
        .unwrap()
        .path
        .clone();
    let relative = collision.to_path_buf().unwrap();
    std::fs::create_dir_all(fx.work().join(&relative).parent().unwrap()).unwrap();
    std::fs::write(
        fx.work().join(&relative),
        b"opaque orphan: preserve these bytes",
    )
    .unwrap();
    fx.git([std::ffi::OsStr::new("add"), relative.as_os_str()])
        .unwrap();
    let before_collision = file_digest(fx.work());
    let result = prepare_stage_candidate(&fx.gated(), fx.work(), &parent, &request);
    assert!(matches!(
        result,
        Err(PrepareStageError::Storage(
            CandidateStorageError::Unsupported(_)
        ))
    ));
    assert_eq!(file_digest(fx.work()), before_collision);
    drop(candidate);
    drop(repeated);
    assert_eq!(std::fs::read_dir(parent).unwrap().count(), 0);
}

#[test]
fn unsupported_chunks_and_metadata_preserve_every_byte_and_leave_no_candidate() {
    use stellar_ops::test_support::project_id;
    use stellar_storage::chunk::envelope::ChunkKindCode;
    use stellar_storage::chunk::id::ChunkId;
    use stellar_storage::chunk::pointer::ChunkPointer;
    use stellar_storage::chunk::store::ChunkStore;
    use stellar_storage::generation::current::write_current_pointer;
    use stellar_storage::generation::loader::{load_current_generation, write_manifest_file};
    use stellar_storage::hash::ContentHash;

    let fx = GitFixture::with_system_git("candidate-unsupported")
        .unwrap()
        .expect("system Git required");
    scaffold_project(fx.work());
    save_documents(fx.work(), &[&large_document(4)]);
    let original = load_current_generation(fx.work(), project_id()).unwrap();
    let parent = fx.root().join("candidates");
    std::fs::create_dir(&parent).unwrap();
    for case in 0..5 {
        let mut manifest = original.manifest.clone();
        match case {
            0 => {
                let kind = ChunkKindCode(999);
                let payload = b"opaque optional chunk with future fields";
                ChunkStore::new(fx.work())
                    .write_chunk(kind, payload)
                    .unwrap();
                manifest.chunk_pointers.push(ChunkPointer {
                    chunk_id: ChunkId::from_stable_id(stellar_core_types::ids::StableId::new_v7(
                        99, 0, 99,
                    )),
                    content_hash: ContentHash::compute(payload),
                    kind,
                    schema_version: 1,
                });
            }
            1 => manifest.chunk_pointers[0].schema_version += 1,
            2 => manifest.root_document_ids.clear(),
            3 => manifest.schema_version += 1,
            4 => {
                manifest
                    .chunk_pointers
                    .iter_mut()
                    .find(|p| p.kind == stellar_storage::chunk::kinds::KIND_DOCUMENT_MANIFEST)
                    .unwrap()
                    .chunk_id =
                    ChunkId::from_stable_id(stellar_core_types::ids::StableId::new_v7(99, 0, 99))
            }
            _ => unreachable!(),
        }
        manifest.chunk_pointers.sort();
        let hash = write_manifest_file(fx.work(), &manifest).unwrap();
        write_current_pointer(fx.work(), &hash).unwrap();
        fx.git(["add", "."]).unwrap();
        let before = file_digest(fx.work());
        let result = prepare_stage_candidate(
            &fx.gated(),
            fx.work(),
            &parent,
            &StageSelectionRequest::stage([]),
        );
        assert!(
            matches!(
                result,
                Err(PrepareStageError::Storage(
                    CandidateStorageError::Unsupported(_)
                ))
            ),
            "case {case}: {result:?}"
        );
        assert_eq!(file_digest(fx.work()), before, "case {case}");
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 0);
    }
}

#[test]
fn no_op_uses_index_generation_and_rejects_worktree_temp_parent_and_untrusted_git() {
    use stellar_git_porcelain::trust::GitTrust;
    let fx = GitFixture::with_system_git("candidate-noop")
        .unwrap()
        .expect("system Git required");
    scaffold_project(fx.work());
    let model = large_document(3);
    save_documents(fx.work(), &[&model]);
    fx.commit_all("fixture baseline").unwrap();
    let mut changed = model.to_records().unwrap();
    changed.nodes[0].display_name = "already staged".into();
    let (changed, _) = DocumentModel::from_records(changed).unwrap();
    save_documents(fx.work(), &[&changed]);
    fx.git(["add", "."]).unwrap();
    let index_files = file_digest(fx.work());
    save_documents(fx.work(), &[&model]);
    let before = file_digest(fx.work());
    let request = StageSelectionRequest::stage([]);
    let parent = fx.root().join("candidates");
    std::fs::create_dir(&parent).unwrap();
    let candidate = prepare_stage_candidate(&fx.gated(), fx.work(), &parent, &request).unwrap();
    let reopened = SavedProjectLoader::for_materialized_snapshot()
        .open(candidate.tree.root())
        .unwrap();
    assert_eq!(reopened.generation_number(), 2);
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
    for (path, hash) in &candidate_files {
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
