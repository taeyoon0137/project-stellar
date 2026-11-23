//! 옛 배치 변환·작업 트리 이동 시험 (ADR-0131).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use stellar_core_types::ids::StableId;
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::context::fixtures::SceneBuilder;
use stellar_document::model::DocumentModel;
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::chunk::kinds::KIND_COMPONENT_LIBRARY;
use stellar_storage::chunk::planner::LogicalChunkState;
use stellar_storage::generation::current::read_current_pointer;
use stellar_storage::generation::loader::load_current_generation;
use stellar_storage::save::cancel::{run_save, CancelToken};
use stellar_storage::save::commit::SavedGenerationState;
use stellar_storage::save::plan::build_save_plan;

use crate::cache::{
    converted_marker, migrate_working_tree, migrate_working_tree_with, project_id,
    rebuild_from_text, CacheError, LegacyLayout, Rebuild,
};
use crate::document_io::serialize::serialize_document;
use crate::document_io::session::{
    commit_files, decode_extra_documents, decode_source, observe, read_source, read_stamp,
    OpenPlan, SourceBase,
};
use crate::document_io::source_v4::documents;

const PROJECT: &str = "0195c530-3acd-7bc1-8414-c213f61cb969";

struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Temp {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("stellar-legacy-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            path.join("stellar.yaml"),
            format!(
                "schema_version: 4\nproject:\n  id: \"{PROJECT}\"\n  name: \"Legacy\"\ntarget_profiles: []\ndependencies: {{}}\n"
            ),
        )
        .unwrap();
        Temp(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn sid(n: u64) -> StableId {
    StableId::new_v7(n, 0x0abc, 0x0123_4567_89ab)
}

fn document(base: u64) -> DocumentModel {
    SceneBuilder::new(sid(base + 1), sid(base + 2), sid(base + 3), sid(base + 10))
        .frame(sid(base + 11), sid(base + 10))
        .build()
}

fn library() -> ComponentLibrary {
    crate::document_io::source_v4::freeze_tests::interface_library(sid)
}

/// 옛 배치 프로젝트: 문서들과(있으면) component library chunk를 저장한 generation을 `design/` 옛 위치에 둡니다.
fn write_legacy(root: &Path, documents: &[&DocumentModel], library: Option<&ComponentLibrary>) {
    let mut states = Vec::new();
    for document in documents {
        states.extend(serialize_document(document).unwrap().to_dirty_states());
    }
    if let Some(library) = library {
        states.push(LogicalChunkState::Dirty {
            chunk_id: ChunkId::from_stable_id(sid(999)),
            kind: KIND_COMPONENT_LIBRARY,
            schema_version: 1,
            payload: library.encode(),
        });
    }
    let project = project_id(root).unwrap();
    let plan = build_save_plan(
        None,
        project,
        crate::migration::PRODUCT_SCHEMA_VERSION,
        &states,
        documents.iter().map(|d| d.document_id()).collect(),
    )
    .unwrap();
    run_save(
        root,
        &plan,
        &mut SavedGenerationState::new(None),
        &CancelToken::new(),
    )
    .unwrap();
    let cache = crate::cache::cache_dir(root);
    std::fs::create_dir_all(root.join("design")).unwrap();
    for name in ["current", "generations", "objects"] {
        std::fs::rename(cache.join(name), root.join("design").join(name)).unwrap();
    }
    std::fs::remove_dir_all(root.join(".stellar")).unwrap();
}

fn same(a: &DocumentModel, b: &DocumentModel) -> bool {
    documents::semantic(a).unwrap() == documents::semantic(b).unwrap()
}

/// root 문서 둘과 Property·Slot·상태 머신을 가진 component library의 옛 배치 프로젝트를 열기 관찰이 텍스트 원본으로
/// 바꿉니다. 의미가 같고, 옛 배치는 Git에 들어갈 때까지 작업 트리에 남으며, 다시 관찰해도 바꾸지 않고, cache를
/// 지워도 원본에서 두 문서를 다시 만듭니다.
#[test]
fn legacy_project_converts_to_a_text_source_on_open() {
    let temp = Temp::new("convert");
    let root = &temp.0;
    let (first, second, components) = (document(0), document(100), library());
    write_legacy(root, &[&first, &second], Some(&components));
    let legacy_pointer = std::fs::read(root.join("design/current")).unwrap();

    let disk = observe(root).unwrap();
    assert_eq!(disk.plan, OpenPlan::Cache, "변환한 원본과 옮긴 cache가 짝");
    let files = disk.text.expect("원본이 생김");
    assert!(files.files.contains_key("documents.stellar.yaml"));
    assert!(files
        .files
        .keys()
        .any(|path| path.starts_with("interfaces/Toggle-")));
    assert!(files
        .files
        .keys()
        .any(|path| path.starts_with("components/Toggle-")));
    let project = decode_source(&files).unwrap();
    assert!(same(&project.document, &first));
    assert_eq!(project.components.encode(), components.encode());
    let extras = decode_extra_documents(&files, first.document_id()).unwrap();
    assert_eq!(extras.len(), 1);
    assert!(same(&extras[0], &second));
    // 옛 배치는 그대로(Git 원본), cache에는 같은 pointer가 있습니다.
    assert_eq!(
        std::fs::read(root.join("design/current")).unwrap(),
        legacy_pointer
    );
    assert!(converted_marker(root).is_some());
    assert_eq!(
        Some(read_current_pointer(root).unwrap()),
        read_stamp(root).map(|s| s.manifest)
    );

    // 다시 관찰해도 바꾸지 않습니다.
    let again = migrate_working_tree(root).unwrap();
    assert!(again.held && again.converted.is_none());
    assert_eq!(observe(root).unwrap().plan, OpenPlan::Cache);

    // chunk cache와 stamp를 지우면 원본에서 두 문서를 다시 만듭니다(변환 표시는 남아 옛 배치를 계속 둠).
    std::fs::remove_dir_all(crate::cache::cache_dir(root)).unwrap();
    std::fs::remove_file(root.join(".stellar/cache/text-source.stamp")).unwrap();
    let Rebuild::Rebuilt { .. } = rebuild_from_text(root).unwrap() else {
        panic!("원본에서 다시 만들어야 함");
    };
    let loaded = load_current_generation(root, project_id(root).unwrap()).unwrap();
    assert_eq!(
        loaded.manifest.root_document_ids,
        vec![first.document_id(), second.document_id()]
    );
    assert_eq!(
        std::fs::read(root.join("design/current")).unwrap(),
        legacy_pointer
    );

    // 변환 뒤 Git에 넣기 전에 옛 pointer가 바뀌면(다른 commit checkout) 옮기지 않고 알립니다.
    std::fs::write(root.join("design/current"), b"other\n").unwrap();
    assert!(matches!(
        migrate_working_tree(root),
        Err(CacheError::LegacyChanged)
    ));
    assert_eq!(observe(root).unwrap().plan, OpenPlan::Cache, "원본으로 엶");
    std::fs::write(root.join("design/current"), &legacy_pointer).unwrap();

    // 변환이 Git에 들어간 뒤(표시가 지워진 뒤) 관찰은 옛 배치를 옮기되 옛 pointer는 버립니다.
    crate::cache::clear_converted_marker(root).unwrap();
    let pointer = read_current_pointer(root).unwrap();
    let moved = migrate_working_tree(root).unwrap();
    assert!(moved.moved.current_dropped);
    assert!(!root.join("design/current").exists());
    assert_eq!(read_current_pointer(root).unwrap(), pointer);
    assert_eq!(observe(root).unwrap().plan, OpenPlan::Cache);
}

/// 변환을 고르지 않으면(Refuse) 거부하고 아무것도 쓰지 않습니다. 텍스트 원본이 담지 못하는 chunk가 있는 옛 배치는
/// 변환도 거부하고 작업 트리를 바꾸지 않습니다.
#[test]
fn unconvertible_legacy_projects_are_left_untouched() {
    let temp = Temp::new("refuse");
    let root = &temp.0;
    let first = document(0);
    write_legacy(root, &[&first], None);
    assert!(matches!(
        migrate_working_tree_with(root, LegacyLayout::Refuse),
        Err(CacheError::LegacyWithoutSource)
    ));
    assert!(read_source(root).unwrap().is_none());

    let temp = Temp::new("unconvertible");
    let root = &temp.0;
    let mut states = serialize_document(&first).unwrap().to_dirty_states();
    states.push(LogicalChunkState::Dirty {
        chunk_id: ChunkId::from_stable_id(sid(998)),
        kind: stellar_storage::chunk::kinds::KIND_TOKEN_SET,
        schema_version: 1,
        payload: b"unknown".to_vec(),
    });
    let plan = build_save_plan(
        None,
        project_id(root).unwrap(),
        crate::migration::PRODUCT_SCHEMA_VERSION,
        &states,
        vec![first.document_id()],
    )
    .unwrap();
    run_save(
        root,
        &plan,
        &mut SavedGenerationState::new(None),
        &CancelToken::new(),
    )
    .unwrap();
    let cache = crate::cache::cache_dir(root);
    std::fs::create_dir_all(root.join("design")).unwrap();
    for name in ["current", "generations", "objects"] {
        std::fs::rename(cache.join(name), root.join("design").join(name)).unwrap();
    }
    std::fs::remove_dir_all(root.join(".stellar")).unwrap();
    let error = migrate_working_tree(root).unwrap_err();
    assert!(
        matches!(error, CacheError::LegacyUnconvertible(_)),
        "{error}"
    );
    assert!(observe(root).is_err(), "열기도 거부");
    assert!(read_source(root).unwrap().is_none(), "원본을 쓰지 않음");
    assert!(root.join("design/current").is_file());
    assert!(!crate::cache::cache_dir(root).join("current").exists());
}

/// 텍스트 원본 프로젝트에서 원본 이후 cache에만 쓴 편집(Forward)이 있을 때 Git이 옛 `design/current`를 되살려도
/// 관찰은 cache pointer를 바꾸지 않습니다(ADR-0131 결정 4).
#[test]
fn restored_legacy_pointer_keeps_forward_edits() {
    let temp = Temp::new("forward");
    let root = &temp.0;
    // 원본 프로젝트를 만들고 cache를 원본에서 만듭니다.
    let first = document(0);
    let encoded = crate::document_io::source_v4::project::encode(
        &crate::document_io::session::SourceLibrary::default()
            .join(documents::copy_model(&first).unwrap()),
    )
    .unwrap();
    commit_files(root, &SourceBase::default(), &encoded.files).unwrap();
    assert!(matches!(
        rebuild_from_text(root).unwrap(),
        Rebuild::Rebuilt { .. }
    ));
    let old_pointer = std::fs::read(crate::cache::cache_dir(root).join("current")).unwrap();
    // CLI·MCP처럼 cache만 새 세대로 씁니다(Forward).
    let project = project_id(root).unwrap();
    let base = load_current_generation(root, project).unwrap();
    let edited = SceneBuilder::new(sid(1), sid(2), sid(3), sid(10))
        .frame(sid(11), sid(10))
        .frame(sid(12), sid(10))
        .build();
    let plan = build_save_plan(
        Some(&base),
        project,
        crate::migration::PRODUCT_SCHEMA_VERSION,
        &serialize_document(&edited).unwrap().to_dirty_states(),
        vec![edited.document_id()],
    )
    .unwrap();
    run_save(
        root,
        &plan,
        &mut SavedGenerationState::new(Some((base.manifest.generation, base.manifest_hash))),
        &CancelToken::new(),
    )
    .unwrap();
    let forward = read_current_pointer(root).unwrap();
    assert_eq!(observe(root).unwrap().plan, OpenPlan::Forward);
    // Git이 옛 pointer를 되살림(예: `git restore design/current`).
    std::fs::write(root.join("design/current"), &old_pointer).unwrap();
    let disk = observe(root).unwrap();
    assert_eq!(disk.plan, OpenPlan::Forward, "cache 편집이 남음");
    assert_eq!(read_current_pointer(root).unwrap(), forward);
    assert!(!root.join("design/current").exists());
}

/// 첫 문서만 다루는 원본 저장(파일 집합에 추가 root 문서가 없음)은 추가 root 문서 파일을 지우지 않습니다.
#[test]
fn primary_only_saves_keep_extra_root_documents() {
    let temp = Temp::new("extras");
    let root = &temp.0;
    let (first, second) = (document(0), document(100));
    let primary = crate::document_io::source_v4::project::encode(
        &crate::document_io::session::SourceLibrary::default()
            .join(documents::copy_model(&first).unwrap()),
    )
    .unwrap()
    .files;
    let mut all = primary.clone();
    all.extend(documents::encode(first.document_id(), &[second]).unwrap());
    commit_files(root, &SourceBase::default(), &all).unwrap();
    let read = read_source(root).unwrap().unwrap();
    let commit = commit_files(root, &read.base, &primary).unwrap();
    assert_eq!(commit.deleted, 0);
    let after = read_source(root).unwrap().unwrap();
    assert_eq!(after.files, all);
    assert_eq!(
        decode_extra_documents(&after, first.document_id())
            .unwrap()
            .len(),
        1
    );
}

fn hermetic_git() -> Option<stellar_git_porcelain::trust::GatedGitRunner> {
    use std::ffi::OsString;
    use stellar_git_porcelain::discovery::{discover_git, GitDiscoveryConfig};
    use stellar_git_porcelain::runner::GitRunner;
    let git = discover_git(&GitDiscoveryConfig::default())?;
    let runner = GitRunner::new(
        git,
        vec![OsString::from("PATH"), OsString::from("HOME")],
        std::time::Duration::from_secs(60),
    )
    .with_env_overrides(vec![
        (
            OsString::from("GIT_CONFIG_GLOBAL"),
            OsString::from("/dev/null"),
        ),
        (
            OsString::from("GIT_CONFIG_SYSTEM"),
            OsString::from("/dev/null"),
        ),
        (OsString::from("GIT_TERMINAL_PROMPT"), OsString::from("0")),
    ]);
    Some(stellar_git_porcelain::trust::GatedGitRunner::new(
        runner,
        stellar_git_porcelain::trust::GitTrust::Trusted,
    ))
}

fn run_git(
    gate: &stellar_git_porcelain::trust::GatedGitRunner,
    root: &Path,
    args: &[&str],
) -> String {
    let mut all = vec![
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@example.invalid",
        "-c",
        "commit.gpgsign=false",
    ];
    all.extend_from_slice(args);
    let output = gate
        .run(&stellar_git_porcelain::runner::argv(all), root)
        .unwrap();
    assert!(
        output.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Git이 추적하는 옛 chunk 프로젝트: 열기가 원본으로 바꾼 뒤 `commit -a`를 해도 옛 배치(유일한 Git 원본)는 HEAD에
/// 남습니다. index 정리(`untrack_legacy`)가 새 원본을 넣고 옛 경로를 빼면, 다음 commit은 원본만 담고 작업 트리는
/// 깨끗하며, 그 commit을 새로 꺼내 열면 같은 의미입니다.
#[test]
fn converted_legacy_reaches_git_as_text_source_in_one_commit() {
    let Some(gate) = hermetic_git() else {
        return;
    };
    let temp = Temp::new("convert-git");
    let root = &temp.0;
    let (first, second, components) = (document(0), document(100), library());
    write_legacy(root, &[&first, &second], Some(&components));
    run_git(&gate, root, &["init", "-q", "-b", "main"]);
    run_git(&gate, root, &["add", "-A"]);
    run_git(&gate, root, &["commit", "-q", "-m", "legacy"]);

    assert_eq!(observe(root).unwrap().plan, OpenPlan::Cache);
    // 변환한 원본은 아직 추적하지 않으므로 `commit -a`는 옛 배치를 지우지 않습니다.
    let status = run_git(&gate, root, &["status", "--porcelain"]);
    assert!(status.lines().all(|l| l.starts_with("??")), "{status}");
    run_git(&gate, root, &["commit", "-q", "-a", "--allow-empty", "-m", "noop"]);
    let tracked = run_git(&gate, root, &["ls-files"]);
    assert!(tracked.lines().any(|l| l == "design/current"), "{tracked}");

    let untracked = crate::cache::untrack_legacy(&gate, root).unwrap();
    assert!(
        untracked.iter().any(|p| p == "design/current"),
        "{untracked:?}"
    );
    assert!(converted_marker(root).is_none());
    assert!(!root.join("design/current").exists(), "옛 배치를 cache로 옮김");
    run_git(&gate, root, &["commit", "-q", "-m", "text source"]);
    let tracked = run_git(&gate, root, &["ls-files"]);
    assert!(
        tracked
            .lines()
            .all(|l| !crate::cache::is_legacy_cache_path(l.as_bytes())),
        "{tracked}"
    );
    assert!(tracked.lines().any(|l| l == "design/documents.stellar.yaml"));
    assert!(tracked.lines().any(|l| l.starts_with("design/interfaces/")));
    assert_eq!(
        run_git(
            &gate,
            root,
            &["status", "--porcelain", "--untracked-files=all"]
        ),
        ""
    );
    // commit을 새 폴더로 꺼내면(clone처럼) 원본만으로 같은 의미를 엽니다.
    let fresh = Temp::new("convert-git-clone");
    run_git(
        &gate,
        &fresh.0,
        &["clone", "-q", root.to_str().unwrap(), "work"],
    );
    let work = fresh.0.join("work");
    let disk = observe(&work).unwrap();
    assert_eq!(disk.plan, OpenPlan::FromText);
    let files = disk.text.unwrap();
    let project = decode_source(&files).unwrap();
    assert!(same(&project.document, &first));
    assert_eq!(project.components.encode(), components.encode());
    assert!(same(
        &decode_extra_documents(&files, first.document_id()).unwrap()[0],
        &second
    ));
}
