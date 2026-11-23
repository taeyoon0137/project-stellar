//! 제품 열기 경로의 외부 library 부분 열기 진단 종단 시험 (XV.5.5·XXV.2.1, ADR-0127 결정 1·2).
//!
//! fixture(`library_common::product`): 사용 프로젝트가 ds·chips를 lock하고, 제품 OS 기본 library cache에는 ds만 있습니다.
//! 창 session 열기(`ProjectWindowSession::load`)와 CLI `stellar diagnose`·MCP `document.diagnostics`가 같이 쓰는 진단 수집
//! (`diagnose_snapshot`)이 Git·network 없이 ds를 로드하고 chips를 library resource unavailable 진단 하나로 진단 index에
//! 싣는지, 프로젝트 문서는 그대로 열리는지 확인합니다.

#![cfg(unix)] // Windows 경로는 보류 상태라 검증하지 않습니다(ADR-0006).

mod e2e_common;
mod library_common;

use e2e_common::TempDir;
use library_common::product::*;
use library_common::*;
use stellar_dependencies::library_load::UnavailableReason;
use stellar_ops::integrity::diagnose_snapshot;
use stellar_ops::integrity::diagnostic::{DiagnosticKind, Severity};
use stellar_ops::snapshot::open_snapshot;

#[test]
fn window_and_headless_open_report_the_same_missing_library_diagnostic() {
    let dir = TempDir::new("library-product-open");
    let Some(project) = product_project(&dir) else {
        eprintln!("시스템 Git 없음: 건너뜀");
        return;
    };

    // CLI·MCP 진단 수집: 프로젝트 문서는 열리고 library 진단은 chips 하나입니다.
    let snapshot = open_snapshot(&project).unwrap();
    let collected = diagnose_snapshot(&snapshot);
    assert_eq!(
        collected.documents_checked, 1,
        "프로젝트 문서는 그대로 열림"
    );
    assert!(collected.unreadable.is_empty());
    let page = collected
        .engine
        .index()
        .page(&Default::default(), None, 100)
        .unwrap();
    // 문서의 ds Card instance는 stellar.yaml에 참조를 저장하지 않은 호환 읽기라 문서 단위 진단이 하나 더 있습니다
    // (origin `library-refs`, ADR-0137 결정 2). 프로젝트 단위 진단(origin `libraries`)만 비교합니다.
    let (library, refs): (Vec<_>, Vec<_>) = page
        .items
        .into_iter()
        .filter(|d| d.kind() == DiagnosticKind::MissingLibrary)
        .partition(|d| d.key.origin == "libraries");
    let ref_codes: Vec<&str> = refs.iter().map(|d| d.code()).collect();
    assert_eq!(ref_codes, ["library-ref/undeclared"], "{refs:?}");
    assert_eq!(library.len(), 1, "{library:?}");
    assert_eq!(library[0].code(), "library-unavailable/missing-cache-entry");
    assert_eq!(library[0].severity, Severity::Warning);
    assert_eq!(library[0].document(), None, "프로젝트 단위 진단");
    assert_eq!(library[0].key.origin, "libraries");
    assert!(library[0]
        .key
        .subject
        .contains(&library_id(CHIPS_ID).to_string()));
    assert!(
        !library[0].message.contains("file://"),
        "진단에 URL 원문 없음"
    );
    assert_eq!(
        collected
            .engine
            .index()
            .summary()
            .kind(DiagnosticKind::MissingLibrary),
        2,
        "진단 index 집계 수(프로젝트 단위 1 + 문서 참조 1)"
    );

    // 창 session 열기: ds만 로드하고 같은 진단을 냅니다. 문서는 열립니다.
    let session = open_window(&project);
    assert!(session.document().is_some(), "문서는 열림");
    let libraries = session.libraries();
    assert!(!libraries.state().is_complete());
    assert!(libraries.state().lock_problem().is_none());
    assert_eq!(libraries.instance().snapshots().count(), 1, "ds만 로드");
    let loaded = libraries.state().libraries().unwrap().loaded().unwrap();
    let missing: Vec<_> = loaded.unavailable().collect();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].reason, UnavailableReason::MissingCacheEntry);
    assert_eq!(missing[0].node.library_id, library_id(CHIPS_ID));
    assert_eq!(libraries.diagnostics(), library, "창과 CLI·MCP가 같은 진단");

    // lock이 손상되면 library는 하나도 로드하지 않고 lock 오류 진단만 남깁니다. 문서는 여전히 열립니다.
    std::fs::write(project.join("stellar.lock.yaml"), "schema_version: [\n").unwrap();
    let snapshot = open_snapshot(&project).unwrap();
    let collected = diagnose_snapshot(&snapshot);
    assert_eq!(collected.documents_checked, 1);
    let page = collected
        .engine
        .index()
        .page(&Default::default(), None, 100)
        .unwrap();
    let codes: Vec<&str> = page
        .items
        .iter()
        .filter(|d| d.kind() == DiagnosticKind::MissingLibrary)
        .map(|d| d.code())
        .collect();
    assert_eq!(codes, ["library-lock/invalid"]);
    let session = open_window(&project);
    assert!(session.document().is_some());
    assert_eq!(session.libraries().instance().snapshots().count(), 0);
    assert_eq!(session.libraries().diagnostics().len(), 1);
}
