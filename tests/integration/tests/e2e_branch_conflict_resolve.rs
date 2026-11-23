//! XVI.11.3 종단 시나리오: 두 branch가 같은 속성을 다르게 바꿔 `git merge` conflict → Semantic Conflict 해결 →
//! 검증한 결과를 쓰고 `git add`까지.
//!
//! 판정 기준(계획 XVI.11.3): unresolved entry 0, merge 상태 유지(MERGE_HEAD가 남고 commit·continue는 하지 않음).
//!
//! 경로(ADR-0111):
//!
//! - 새 프로젝트는 앱 runtime 생성 경로, 편집은 `stellar apply`·`stellar component-source resize`와 같은 ops 함수,
//!   commit·branch·merge는 사용자가 터미널에서 쓰는 시스템 Git입니다(merge driver는 등록하지 않아 텍스트 conflict가 남음).
//! - 해결은 `stellar merge-driver resolve`와 같은 conflict 세션(`ConflictSession` + ops Git 포트 `GatedMergeGit`)과
//!   적용(`apply_session`: 저장 journal 한 거래로 쓰고 `git add`)입니다.
//!
//! conflict 단위는 둘입니다: generation 문서(`design/current`)의 Frame `width`, 컴포넌트 원본 파일의 root 크기.
//! 서로 다른 속성 변경(한쪽 `background-color`, 다른 쪽 이름)은 자동 병합되어야 합니다.
//!
//! 텍스트 원본 View 파일(`design/pages/**`)의 conflict는 conflict 세션이 의미 병합하지 않는 경로입니다(ADR-0081 단점).
//! 그래서 이 시나리오는 앱 창으로 열지 않은(텍스트 원본이 없는) 프로젝트를 씁니다(ADR-0111 결정 5).

mod e2e_common;

use std::path::Path;

use e2e_common::{git, git_session, trusted_git, App, TempDir};
use stellar_core_types::ids::StableId;
use stellar_core_types::value::Size2;
use stellar_document::records::NodePayload;
use stellar_ops::apply::{apply, ApplyMode, ApplyRequest, CommandInput};
use stellar_ops::candidate::apply::apply_session;
use stellar_ops::candidate::git_port::{GatedMergeGit, GitOperation, MergeGit};
use stellar_ops::candidate::session::{ConflictSession, UnitKind};
use stellar_ops::lock::{default_lock_dir, LockPolicy};
use stellar_ops::query::{project_structure, property_path_label};
use stellar_ops::snapshot::open_snapshot;
use stellar_ops::validate::validate_project;
use stellar_semantic_merge::candidate::Resolution;
use stellar_semantic_merge::model::ConflictTarget;

/// `stellar apply` 한 번(쓰기 모드, 사용자 잠금 폴더).
fn cli_apply(root: &Path, document: StableId, commands: Vec<CommandInput>) {
    let snapshot = open_snapshot(root).unwrap();
    apply(
        root,
        &ApplyRequest {
            base_revision: snapshot.revision(),
            document,
            commands,
        },
        ApplyMode::Write,
        &LockPolicy::Required {
            dir: default_lock_dir().unwrap(),
        },
    )
    .expect("stellar apply");
}

fn set(node: StableId, property: &str, value: &str) -> CommandInput {
    CommandInput::SetProperty {
        node,
        property: property.into(),
        value: value.into(),
    }
}

/// 컴포넌트 원본 root 크기 변경(`stellar component-source resize`).
fn resize_component(file: &Path, width: f32, height: f32) {
    let info = stellar_ops::component_source::check(file.parent().unwrap())
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    stellar_ops::component_source::resize(
        file,
        &info.hash.to_string(),
        Size2::new(width, height).unwrap(),
    )
    .expect("컴포넌트 크기 변경");
}

fn commit_all(gate: &stellar_git_porcelain::trust::GatedGitRunner, root: &Path, message: &str) {
    git(gate, root, &["add", "-A"]);
    git(gate, root, &["commit", "-q", "-m", message]);
}

fn frame_size(root: &Path, document: StableId, frame: StableId) -> (f32, f32) {
    let records = open_snapshot(root)
        .unwrap()
        .document_records(document)
        .unwrap();
    let node = records.nodes.iter().find(|n| n.id == frame).unwrap();
    match &node.payload {
        NodePayload::Frame(f) => (f.size.width(), f.size.height()),
        other => panic!("Frame이 아님: {other:?}"),
    }
}

#[test]
fn two_branch_conflict_is_resolved_semantically_and_added() {
    let Some(gate) = trusted_git() else {
        eprintln!("시스템 Git 없음: 건너뜀");
        return;
    };
    let dir = TempDir::new("branch-conflict");
    let root = dir.path.join("병합 프로젝트");
    let app = App::start();
    assert_eq!(app.create_project(&root), 0);
    app.shutdown();
    let structure = project_structure(&open_snapshot(&root).unwrap());
    let document = structure.documents[0].id;
    let frame = structure.documents[0].pages[0].views[0].root_frame;

    // base: 컴포넌트 하나를 더해 commit.
    let components = root.join("design/components");
    std::fs::create_dir_all(&components).unwrap();
    let ids = stellar_project::new_ids::new_ids::<2>().unwrap();
    let info =
        stellar_ops::component_source::create(&components, "Chip".into(), ids[0], ids[1]).unwrap();
    let component_file = components.join(info.file.file_name().unwrap());
    commit_all(&gate, &root, "base: 컴포넌트");

    // feature: width 1280, 이름 변경, 컴포넌트 200×40.
    git(&gate, &root, &["checkout", "-q", "-b", "feature"]);
    cli_apply(
        &root,
        document,
        vec![
            set(frame, "width", "1280px"),
            CommandInput::Rename {
                node: frame,
                name: "기능 화면".into(),
            },
        ],
    );
    resize_component(&component_file, 200.0, 40.0);
    commit_all(&gate, &root, "feature: 너비 1280");

    // main: width 1024, 배경색, 컴포넌트 240×40.
    git(&gate, &root, &["checkout", "-q", "main"]);
    cli_apply(
        &root,
        document,
        vec![
            set(frame, "width", "1024px"),
            set(frame, "background-color", "#204080"),
        ],
    );
    resize_component(&component_file, 240.0, 40.0);
    commit_all(&gate, &root, "main: 너비 1024");
    let ours_head = git(&gate, &root, &["rev-parse", "HEAD"]).trim().to_string();
    let theirs_head = git(&gate, &root, &["rev-parse", "feature"])
        .trim()
        .to_string();

    // ── git merge: 텍스트 conflict가 납니다(종료 코드 1).
    let merge = gate
        .run(
            &stellar_git_porcelain::runner::argv(["merge", "--no-edit", "feature"]),
            &root,
        )
        .unwrap();
    assert!(!merge.success(), "conflict가 나야 함");
    let port = GatedMergeGit::new(gate.clone());
    let unmerged: Vec<_> = port
        .unmerged(&root)
        .unwrap()
        .into_iter()
        .map(|e| e.path)
        .collect();
    // 앱이 만든 프로젝트는 텍스트 원본이 Git 원본입니다(ADR-0113): View 원본·로컬 cache pointer·컴포넌트 원본.
    assert_eq!(
        unmerged.len(),
        3,
        "View 원본·design/current·컴포넌트 원본: {unmerged:?}"
    );
    assert!(unmerged.contains(&Path::new("design/current").to_path_buf()));
    let view_path = unmerged
        .iter()
        .find(|p| p.starts_with("design/pages"))
        .expect("View 원본 conflict")
        .clone();

    // ── conflict 세션: 단위 두 개, raw·제외 경로 없음.
    let mut session = ConflictSession::open(&port, &root).expect("conflict 세션");
    assert_eq!(session.operation(), GitOperation::Merge);
    assert!(
        session.raw_conflicts().is_empty(),
        "{:?}",
        session.raw_conflicts()
    );
    assert!(
        session.unsupported().is_empty(),
        "{:?}",
        session.unsupported()
    );
    let kinds: Vec<_> = session.units().iter().map(|u| u.kind()).collect();
    assert_eq!(kinds.len(), 2, "{kinds:?}");
    assert!(kinds.contains(&UnitKind::TextSource) && kinds.contains(&UnitKind::ComponentSource));

    // semantic conflict는 같은 속성을 다르게 바꾼 두 곳뿐입니다. 이름·배경색은 conflict가 아닙니다.
    let conflicts: Vec<(usize, UnitKind, StableId, String)> = session
        .conflicts()
        .iter()
        .map(|c| {
            let label = match &c.conflict.target {
                ConflictTarget::Property(path) => property_path_label(path),
                other => format!("{other:?}"),
            };
            (c.index, c.unit.kind(), c.conflict.object, label)
        })
        .collect();
    assert_eq!(conflicts.len(), 2, "{conflicts:?}");
    let doc_conflict = conflicts
        .iter()
        .find(|c| c.1 == UnitKind::TextSource)
        .expect("문서 conflict");
    assert_eq!((doc_conflict.2, doc_conflict.3.as_str()), (frame, "width"));
    let component_conflict = conflicts
        .iter()
        .find(|c| c.1 == UnitKind::ComponentSource)
        .expect("컴포넌트 conflict");
    assert_eq!(session.paths_for(frame), vec![view_path.as_path()]);
    for c in session.conflicts() {
        assert!(
            c.allowed().contains(&"theirs") && c.allowed().contains(&"ours"),
            "{:?}",
            c.allowed()
        );
        assert_eq!(c.resolution(), None);
    }

    // 미해결이면 아무것도 쓰지 않습니다.
    let before = e2e_common::tree(&root);
    assert!(apply_session(&port, &session).is_err());
    assert_eq!(e2e_common::tree(&root), before);

    // ── 해결: 문서 width는 theirs(1280), 컴포넌트 크기는 ours(240).
    session
        .resolve(doc_conflict.0, Resolution::ChooseTheirs)
        .unwrap();
    session
        .resolve(component_conflict.0, Resolution::ChooseOurs)
        .unwrap();
    assert!(session.units().iter().all(|u| u.status().writable()));
    let report = apply_session(&port, &session).expect("해결 적용");
    assert_eq!(
        report.operation,
        Some(GitOperation::Merge),
        "merge 진행 중 그대로"
    );
    assert_eq!(report.remaining, Some(Vec::new()), "unresolved entry 0");
    assert!(report
        .added
        .contains(&Path::new("design/current").to_path_buf()));
    assert!(report.added.contains(&view_path));
    assert!(report
        .added
        .iter()
        .any(|p| p.starts_with("design/components")));

    // ── Git 상태: unmerged 0, MERGE_HEAD 유지, HEAD는 merge 전 ours, commit하지 않음.
    assert!(port.unmerged(&root).unwrap().is_empty());
    assert_eq!(git(&gate, &root, &["ls-files", "-u"]), "");
    assert_eq!(
        git(&gate, &root, &["rev-parse", "MERGE_HEAD"]).trim(),
        theirs_head
    );
    assert_eq!(git(&gate, &root, &["rev-parse", "HEAD"]).trim(), ours_head);
    assert_eq!(port.operation(&root).unwrap(), GitOperation::Merge);

    // ── 결과 의미: width = theirs, 배경색(ours)과 이름(theirs)은 자동 병합, 컴포넌트 = ours.
    assert_eq!(frame_size(&root, document, frame).0, 1280.0);
    let records = open_snapshot(&root)
        .unwrap()
        .document_records(document)
        .unwrap();
    let node = records.nodes.iter().find(|n| n.id == frame).unwrap();
    assert_eq!(node.display_name, "기능 화면");
    match &node.payload {
        NodePayload::Frame(f) => assert!(f.background.is_some(), "main의 배경색 유지"),
        _ => unreachable!(),
    }
    let merged_component = stellar_ops::component_source::check(&components).unwrap();
    assert_eq!(merged_component.len(), 1);
    assert_eq!(merged_component[0].size, Size2::new(240.0, 40.0).unwrap());
    // 병합 결과는 전체 validator와 staged 재검증을 통과합니다(사용자가 이어서 commit할 수 있음).
    assert_eq!(validate_project(&root).unwrap().error_count(), 0);
    let staged = git_session(&gate, &root)
        .validate_staged()
        .expect("staged 재검증");
    assert!(staged.is_valid(), "{:?}", staged.findings);
    // 작업 트리와 index가 같습니다(해결 결과가 모두 add됨).
    assert_eq!(git(&gate, &root, &["diff", "--name-only"]), "");
}
