//! CLI·MCP 읽기·쓰기의 텍스트 원본 경로(ADR-0113 결정 1~3) 시험.

use std::path::{Path, PathBuf};

use stellar_project::document_io::session::{
    commit, convert_saved, observe, read_stamp, write_stamp, OpenPlan, SourceBase, Stamp,
};
use stellar_project::document_io::source_v4::project::DOCUMENT_PATH;

use super::save::SourceWrite;
use super::*;
use crate::apply::{apply, ApplyError, ApplyMode, ApplyRequest, CommandInput};
use crate::lock::LockPolicy;
use crate::test_support::{file_digest, large_document, save_documents, scaffold_project, TempDir};

/// chunk 프로젝트를 만들고 앱 창 열기와 같은 검증된 변환으로 텍스트 원본과 stamp를 씁니다(Cache 짝).
pub(crate) fn text_project(tag: &str) -> (TempDir, PathBuf, DocumentModel) {
    let tmp = TempDir::new(tag);
    let root = tmp.path.join("project");
    scaffold_project(&root);
    let model = large_document(12);
    save_documents(&root, &[&model]);
    let lazy =
        LazyGeneration::open(&root, crate::test_support::project_id(), &mut NoopRecorder).unwrap();
    let (base, library) = convert_saved(&root, &lazy, &model, None).unwrap();
    let written = commit(&root, &base, &library.join(copy(&model))).unwrap();
    write_stamp(
        &root,
        &Stamp {
            source: written.base.digest(),
            manifest: stellar_storage::generation::current::read_current_pointer(&root).unwrap(),
        },
    )
    .unwrap();
    (tmp, root, model)
}

fn copy(model: &DocumentModel) -> DocumentModel {
    DocumentModel::from_records(model.to_records().unwrap())
        .unwrap()
        .0
}

fn locks(tmp: &TempDir) -> LockPolicy {
    LockPolicy::Required {
        dir: tmp.path.join("locks"),
    }
}

fn view_file(root: &Path) -> PathBuf {
    let design = root.join("design");
    let files = stellar_project::document_io::session::read_source_dir(&design).unwrap();
    let view = files
        .keys()
        .find(|path| path.starts_with("pages/"))
        .expect("View 파일");
    design.join(view)
}

fn child(model: &DocumentModel) -> StableId {
    let records = model.to_records().unwrap();
    records
        .nodes
        .iter()
        .find(|node| node.parent.is_some())
        .unwrap()
        .id
}

fn rename(snapshot: &ProjectSnapshot, node: StableId, name: &str) -> ApplyRequest {
    ApplyRequest {
        base_revision: snapshot.revision(),
        document: snapshot.documents()[0].id,
        commands: vec![CommandInput::Rename {
            node,
            name: name.into(),
        }],
    }
}

fn name_of(snapshot: &ProjectSnapshot, node: StableId) -> String {
    snapshot
        .document_records(snapshot.documents()[0].id)
        .unwrap()
        .nodes
        .iter()
        .find(|n| n.id == node)
        .unwrap()
        .display_name
        .clone()
}

#[test]
fn a_paired_source_opens_from_the_cache_and_apply_writes_source_cache_and_stamp() {
    let (tmp, root, model) = text_project("text-apply");
    let snapshot = open_snapshot(&root).unwrap();
    assert_eq!(snapshot.open_plan(), OpenPlan::Cache);
    assert!(!snapshot.reads_from_text());
    assert_eq!(snapshot.revision().hash(), snapshot.current_manifest());
    let node = child(&model);
    let view = view_file(&root);
    let before = std::fs::read_to_string(&view).unwrap();

    let outcome = apply(
        &root,
        &rename(&snapshot, node, "from-cli"),
        ApplyMode::Write,
        &locks(&tmp),
    )
    .unwrap();
    assert!(
        matches!(
            outcome.source,
            Some(SourceWrite::Text {
                written: 1,
                deleted: 0
            })
        ),
        "{:?}",
        outcome.source
    );
    assert!(!outcome.cache_stale);
    let after = std::fs::read_to_string(&view).unwrap();
    assert_ne!(before, after);
    assert!(after.contains("from-cli"), "원본 View 파일에 이름 줄");

    // 원본·cache·stamp가 다시 짝이고, 다시 연 revision이 쓰기 결과와 같습니다.
    let disk = observe(&root).unwrap();
    assert_eq!(disk.plan, OpenPlan::Cache);
    assert_eq!(
        read_stamp(&root),
        Some(Stamp {
            source: disk.text.as_ref().unwrap().base.digest(),
            manifest: disk.current.unwrap(),
        })
    );
    let reopened = open_snapshot(&root).unwrap();
    assert_eq!(reopened.revision(), outcome.revision);
    assert_eq!(name_of(&reopened, node), "from-cli");
}

#[test]
fn a_source_edit_is_read_from_text_changes_the_revision_and_the_next_write_rebuilds_the_cache() {
    let (tmp, root, model) = text_project("text-pull");
    let node = child(&model);
    let cached = open_snapshot(&root).unwrap();
    let old = cached.revision();
    // Git pull·편집기처럼 원본만 바뀝니다(cache·stamp는 그대로).
    let first = open_snapshot(&root).unwrap();
    let edit = apply(
        &root,
        &rename(&first, node, "pulled"),
        ApplyMode::DryRun,
        &LockPolicy::None,
    )
    .unwrap();
    assert_eq!(edit.revision, old, "dry-run은 쓰지 않음");
    let view = view_file(&root);
    let text = std::fs::read_to_string(&view).unwrap();
    let empty = r#"display_name: """#;
    let at = text.rfind(empty).expect("빈 이름 줄");
    let renamed = format!(
        "{}display_name: pulled{}",
        &text[..at],
        &text[at + empty.len()..]
    );
    assert_ne!(text, renamed, "fixture에 빈 이름 줄이 있어야 함");
    std::fs::write(&view, renamed).unwrap();

    let digest = file_digest(&root);
    let snapshot = open_snapshot(&root).unwrap();
    assert_eq!(snapshot.open_plan(), OpenPlan::FromText);
    assert!(snapshot.reads_from_text());
    assert_ne!(snapshot.revision(), old, "원본이 바뀌면 revision이 바뀜");
    assert_eq!(snapshot.revision().generation(), old.generation() + 1);
    assert!(!cached.disk_unchanged(), "MCP 재사용 판정이 원본 변경을 봄");
    assert!(snapshot.disk_unchanged());
    assert_eq!(
        file_digest(&root),
        digest,
        "읽기는 cache를 다시 만들지 않음"
    );
    let names: Vec<String> = snapshot
        .document_records(snapshot.documents()[0].id)
        .unwrap()
        .nodes
        .iter()
        .map(|n| n.display_name.clone())
        .collect();
    assert!(names.iter().any(|n| n == "pulled"), "{names:?}");

    // 원본 변경 전 revision으로 쓰면 아무것도 쓰지 않고 거부합니다.
    let stale = apply(
        &root,
        &rename(&cached, node, "lost"),
        ApplyMode::Write,
        &locks(&tmp),
    )
    .unwrap_err();
    assert!(
        matches!(stale, ApplyError::BaseRevisionMismatch { .. }),
        "{stale:?}"
    );
    assert_eq!(file_digest(&root), digest);

    // 새 revision으로 쓰면 원본 위에 쓰고 cache를 다시 맞춥니다.
    let outcome = apply(
        &root,
        &rename(&snapshot, node, "after-pull"),
        ApplyMode::Write,
        &locks(&tmp),
    )
    .unwrap();
    let reopened = open_snapshot(&root).unwrap();
    assert_eq!(reopened.open_plan(), OpenPlan::Cache);
    assert_eq!(reopened.revision(), outcome.revision);
    assert_eq!(name_of(&reopened, node), "after-pull");
    let names: Vec<String> = reopened
        .document_records(reopened.documents()[0].id)
        .unwrap()
        .nodes
        .iter()
        .map(|n| n.display_name.clone())
        .collect();
    assert!(
        names.iter().any(|n| n == "pulled"),
        "원본에서 온 편집이 cache에도"
    );
}

#[test]
fn a_cache_only_write_is_read_forward_and_the_next_write_moves_it_into_the_source() {
    let (tmp, root, model) = text_project("text-forward");
    let node = child(&model);
    // 원본을 모르는 옛 writer가 generation만 씁니다.
    let mut edited = copy(&model);
    edited.set_display_name(node, "cache-only".into()).unwrap();
    save_documents(&root, &[&edited]);

    let snapshot = open_snapshot(&root).unwrap();
    assert_eq!(snapshot.open_plan(), OpenPlan::Forward);
    assert_eq!(name_of(&snapshot, node), "cache-only");
    let other = child_after(&model, node);
    apply(
        &root,
        &rename(&snapshot, other, "second"),
        ApplyMode::Write,
        &locks(&tmp),
    )
    .unwrap();
    let text = std::fs::read_to_string(view_file(&root)).unwrap();
    assert!(
        text.contains("cache-only") && text.contains("second"),
        "{text}"
    );
    assert_eq!(open_snapshot(&root).unwrap().open_plan(), OpenPlan::Cache);
}

fn child_after(model: &DocumentModel, not: StableId) -> StableId {
    model
        .to_records()
        .unwrap()
        .nodes
        .iter()
        .find(|node| node.parent.is_some() && node.id != not)
        .unwrap()
        .id
}

#[test]
fn a_broken_source_opens_but_refuses_document_reads_and_writes_with_the_file_name() {
    let (tmp, root, model) = text_project("text-broken");
    std::fs::write(view_file(&root), "format: [\n").unwrap();
    let snapshot = open_snapshot(&root).unwrap();
    assert!(snapshot.reads_from_text());
    let diagnostic = snapshot.source_diagnostic().expect("진단").to_string();
    assert!(diagnostic.contains("pages/"), "{diagnostic}");
    let error = snapshot
        .document_records(model.document_id())
        .unwrap_err()
        .to_string();
    assert!(error.contains("pages/"), "{error}");
    let digest = file_digest(&root);
    let request = rename(&snapshot, child(&model), "x");
    assert!(apply(&root, &request, ApplyMode::Write, &locks(&tmp)).is_err());
    assert_eq!(file_digest(&root), digest);
}

#[test]
fn a_text_only_clone_without_cache_is_read_from_text_and_the_first_write_creates_the_cache() {
    let (tmp, root, model) = text_project("text-clone");
    // Git clone처럼 원본만 있고 로컬 cache·stamp가 없습니다.
    // chunk cache와 stamp는 모두 `.stellar/` 아래 로컬 데이터입니다(ADR-0122).
    std::fs::remove_dir_all(root.join(".stellar")).unwrap();
    assert!(root.join("design").join(DOCUMENT_PATH).is_file());
    let snapshot = open_snapshot(&root).unwrap();
    assert_eq!(snapshot.open_plan(), OpenPlan::FromText);
    assert!(snapshot.generation().is_none());
    assert_eq!(snapshot.revision().generation(), 1);
    assert_eq!(snapshot.documents()[0].id, model.document_id());
    let node = child(&model);
    let outcome = apply(
        &root,
        &rename(&snapshot, node, "first"),
        ApplyMode::Write,
        &locks(&tmp),
    )
    .unwrap();
    let reopened = open_snapshot(&root).unwrap();
    assert_eq!(reopened.open_plan(), OpenPlan::Cache);
    assert_eq!(reopened.revision(), outcome.revision);
    assert_eq!(reopened.revision().generation(), 1);
    assert_eq!(name_of(&reopened, node), "first");
}

#[test]
fn a_chunk_project_without_source_stays_chunk_only_on_write() {
    let tmp = TempDir::new("text-chunk-only");
    let root = tmp.path.join("project");
    scaffold_project(&root);
    let model = large_document(4);
    save_documents(&root, &[&model]);
    let snapshot = open_snapshot(&root).unwrap();
    assert_eq!(snapshot.open_plan(), OpenPlan::Migrate);
    let outcome = apply(
        &root,
        &rename(&snapshot, child(&model), "chunk"),
        ApplyMode::Write,
        &locks(&tmp),
    )
    .unwrap();
    assert!(matches!(
        outcome.source,
        Some(SourceWrite::ChunkOnly { .. })
    ));
    assert!(!root.join("design").join(DOCUMENT_PATH).exists());
    assert_eq!(read_stamp(&root), None);
    let _ = SourceBase::default();
}

#[test]
fn a_write_over_an_unreadable_source_is_refused_instead_of_falling_back_to_the_cache() {
    // 원본이 있는데 읽을 수 없으면 cache만 쓰지 않습니다(Git 원본과 cache가 갈라지지 않게 함, ADR-0113 결정 3).
    let (_tmp, root, model) = text_project("text-broken-save");
    std::fs::write(view_file(&root), "format: [\n").unwrap();
    let snapshot = open_snapshot(&root).unwrap();
    assert!(snapshot.source_diagnostic().is_some());
    let digest = file_digest(&root);
    let error =
        super::save::save_documents(&snapshot, save::root_order(&snapshot), vec![copy(&model)])
            .unwrap_err();
    assert!(
        matches!(&error, super::save::SaveError::Source(message) if message.contains("pages/")),
        "{error:?}"
    );
    assert_eq!(file_digest(&root), digest);
}

fn set_style_defaults(snapshot: &ProjectSnapshot, name: &str) -> ApplyRequest {
    let command = stellar_commands::command::Command::SetStyleDefaults(
        stellar_commands::style_defaults::SetStyleDefaults { name: name.into() },
    );
    ApplyRequest {
        base_revision: snapshot.revision(),
        document: snapshot.documents()[0].id,
        commands: vec![CommandInput::Canonical {
            kind: command.kind().name().to_string(),
            bytes: stellar_commands::codec::encode_command(&command),
        }],
    }
}

/// 두 digest에서 내용이 다른 파일(추가·삭제 포함).
fn changed_files(
    before: &std::collections::BTreeMap<String, String>,
    after: &std::collections::BTreeMap<String, String>,
) -> Vec<String> {
    before
        .keys()
        .chain(after.keys())
        .filter(|name| before.get(*name) != after.get(*name))
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[test]
fn a_set_only_write_on_a_text_source_project_touches_only_the_manifest_line() {
    // ADR-0128 결정 1·2: 텍스트 원본 프로젝트(원본·cache 짝, 원본만 바뀐 FromText)에서도 세트만 바꾸는 쓰기는
    // 원본·cache·stamp를 쓰지 않고 `stellar.yaml`만 바꿉니다. 연 방식(open plan)이 그대로이고, 다시 연 revision이
    // 쓰기 결과와 같습니다.
    let (tmp, root, model) = text_project("text-set-only");
    let snapshot = open_snapshot(&root).unwrap();
    assert_eq!(snapshot.open_plan(), OpenPlan::Cache);
    let before = file_digest(&root);
    let outcome = apply(
        &root,
        &set_style_defaults(&snapshot, "react-native"),
        ApplyMode::Write,
        &locks(&tmp),
    )
    .unwrap();
    assert!(outcome.source.is_none());
    assert_eq!(outcome.written_chunks, 0);
    assert_eq!(
        changed_files(&before, &file_digest(&root)),
        ["stellar.yaml"]
    );
    let reopened = open_snapshot(&root).unwrap();
    assert_eq!(reopened.open_plan(), OpenPlan::Cache);
    assert_eq!(reopened.storage_revision(), snapshot.storage_revision());
    assert_eq!(reopened.revision(), outcome.revision);
    assert_ne!(reopened.revision(), snapshot.revision());

    // 원본만 바뀐 상태(FromText)에서도 같습니다. cache를 다시 만들지 않고 다음 문서 쓰기가 맞춥니다.
    let view = view_file(&root);
    let text = std::fs::read_to_string(&view).unwrap();
    let empty = r#"display_name: """#;
    let at = text.rfind(empty).expect("빈 이름 줄");
    std::fs::write(
        &view,
        format!(
            "{}display_name: pulled{}",
            &text[..at],
            &text[at + empty.len()..]
        ),
    )
    .unwrap();
    let pulled = open_snapshot(&root).unwrap();
    assert_eq!(pulled.open_plan(), OpenPlan::FromText);
    let before = file_digest(&root);
    let back = apply(
        &root,
        &set_style_defaults(&pulled, "css"),
        ApplyMode::Write,
        &locks(&tmp),
    )
    .unwrap();
    assert!(back.source.is_none());
    assert_eq!(
        changed_files(&before, &file_digest(&root)),
        ["stellar.yaml"]
    );
    let reopened = open_snapshot(&root).unwrap();
    assert_eq!(reopened.open_plan(), OpenPlan::FromText);
    assert_eq!(reopened.revision(), back.revision);
    assert_eq!(back.revision, pulled.storage_revision());
    let node = child(&model);
    let edit = apply(
        &root,
        &rename(&reopened, node, "after-set"),
        ApplyMode::Write,
        &locks(&tmp),
    )
    .unwrap();
    assert_eq!(open_snapshot(&root).unwrap().revision(), edit.revision);
    assert_eq!(open_snapshot(&root).unwrap().open_plan(), OpenPlan::Cache);
}
