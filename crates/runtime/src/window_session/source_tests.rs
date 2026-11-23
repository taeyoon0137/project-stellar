//! 텍스트 원본 기본 열기·저장·외부 변경 시험(V.10.6·V.10.7, ADR-0088).
//!
//! 기존 chunk 프로젝트를 열면 원본으로 옮기고, 저장은 원본(journal)과 cache generation·stamp를 함께 맞추며, 원본·
//! cache 어느 쪽이 따로 움직여도 다음 읽기가 계획([`OpenPlan`])대로 맞춥니다.

use super::test_fixtures::*;
use super::*;
use std::path::Path;
use stellar_commands::document::test_support::semantic_snapshot::model_snapshot_json;
use stellar_ops::apply::{apply, ApplyMode, ApplyRequest, CommandInput};
use stellar_ops::lock::LockPolicy;
use stellar_ops::snapshot::open_snapshot;
use stellar_project::document_io::session::{read_source, read_stamp};
use stellar_project::external_change::source::TextSourceSession;

fn locks_of(locks: &TempDir) -> LockDir {
    let dir = locks.path().to_path_buf();
    LockDir::new(move || Ok(dir.clone()))
}

fn open(project: &TempDir, locks: &TempDir, n: u64) -> ProjectWindowSession {
    ProjectWindowSession::load_with_lock_dir(identity(project.path()), window(n), locks_of(locks))
        .unwrap()
}

fn json(session: &ProjectWindowSession) -> String {
    model_snapshot_json(session.document().unwrap().model()).unwrap()
}

fn name_of(session: &ProjectWindowSession, node: u64) -> String {
    session
        .document()
        .unwrap()
        .model()
        .record(sid(node))
        .unwrap()
        .display_name
}

fn command(n: u64) -> CommandId {
    CommandId::from_stable_id(StableId::new_v7(80_000 + n, 0x0f0f, n))
}

/// 원본 파일 digest.
fn source_digest(root: &Path) -> ContentHash {
    read_source(root).unwrap().unwrap().base.digest()
}

/// 원본과 cache가 stamp 짝인지.
fn stamped(root: &Path) -> bool {
    let current = stellar_storage::generation::current::read_current_pointer(root).unwrap();
    read_stamp(root)
        == Some(Stamp {
            source: source_digest(root),
            manifest: current,
        })
}

/// 원본만 바꾸는 외부 writer(Git pull·편집기·엔진 session): 텍스트 원본 session으로 이름을 바꿔 저장합니다.
fn text_rename(project: &TempDir, node: u64, name: &str) {
    let mut text = TextSourceSession::open(project.path(), window(77)).unwrap();
    let tx = rename(text.document(), 900 + node, node, name);
    text.apply(&tx).unwrap();
    text.save().unwrap();
}

/// cache만 바꾸는 외부 writer(`stellar apply`, 아직 generation에 씀).
fn cli_rename(project: &TempDir, locks: &TempDir, node: u64, name: &str) {
    let snapshot = open_snapshot(project.path()).unwrap();
    let request = ApplyRequest {
        base_revision: snapshot.revision(),
        document: sid(1),
        commands: vec![CommandInput::Rename {
            node: sid(node),
            name: name.to_string(),
        }],
    };
    apply(
        project.path(),
        &request,
        ApplyMode::Write,
        &LockPolicy::Required {
            dir: locks.path().to_path_buf(),
        },
    )
    .unwrap();
}

#[test]
fn opening_a_chunk_project_migrates_it_to_the_text_source_once() {
    let project = saved_project("source-migrate");
    let locks = TempDir::new("source-migrate-locks");
    assert!(
        read_source(project.path()).unwrap().is_none(),
        "처음에는 원본 없음"
    );
    let a = open(&project, &locks, 1);
    assert_eq!(a.open_plan(), OpenPlan::Migrate);
    assert_eq!(a.chunk_only_reason(), None);
    assert_eq!(a.generation_number(), 1, "옮기기는 cache를 새로 쓰지 않음");
    let files = read_source(project.path()).unwrap().unwrap();
    assert!(files.files.contains_key("document.stellar.yaml"));
    assert_eq!(a.source_base(), Some(&files.base));
    assert!(stamped(project.path()));
    assert_eq!(
        read_stamp(project.path()).map(|_| ()),
        Some(()),
        "stamp는 .stellar/cache에 있음"
    );
    assert_eq!(
        std::fs::read(project.path().join(".stellar/cache/.gitignore")).unwrap(),
        b"*\n"
    );

    // 다시 열면 원본과 cache가 짝이라 cache에서 엽니다(빠른 열기). 의미는 같습니다.
    let b = open(&project, &locks, 2);
    assert_eq!(b.open_plan(), OpenPlan::Cache);
    assert_eq!(json(&a), json(&b));
    // 원본만으로 읽은 의미도 같습니다.
    let text = TextSourceSession::open(project.path(), window(3)).unwrap();
    assert_eq!(
        model_snapshot_json(text.document().model()).unwrap(),
        json(&a)
    );
}

#[test]
fn save_writes_the_changed_source_file_then_the_cache_and_the_stamp() {
    let project = saved_project("source-save");
    let locks = TempDir::new("source-save-locks");
    let mut a = open(&project, &locks, 1);
    let before = read_source(project.path()).unwrap().unwrap();
    a.apply(&rename(a.document().unwrap(), 10, 11, "원본 저장"), 0)
        .unwrap();
    let outcome = a.save_now().unwrap();
    assert!(outcome.written);
    assert_eq!(outcome.source_files, 1, "바뀐 View 파일 하나만");
    assert!(!outcome.cache_stale);
    assert_eq!(a.generation_number(), 2);
    let after = read_source(project.path()).unwrap().unwrap();
    assert_eq!(before.base.changed_paths(&after.base).len(), 1);
    assert!(stamped(project.path()));
    assert!(!a.has_unsaved_changes());
    assert!(
        a.stale_state().unwrap().is_none(),
        "자기 저장은 stale이 아님"
    );

    let b = open(&project, &locks, 2);
    assert_eq!(b.open_plan(), OpenPlan::Cache);
    assert_eq!(name_of(&b, 11), "원본 저장");
    // 바뀐 것이 없으면 아무것도 쓰지 않습니다.
    let mut b = b;
    assert!(!b.save_now().unwrap().wrote_anything());
}

#[test]
fn a_source_edit_while_closed_reopens_from_the_text_and_rebuilds_the_cache() {
    let project = saved_project("source-from-text");
    let locks = TempDir::new("source-from-text-locks");
    drop(open(&project, &locks, 1));
    text_rename(&project, 11, "Git pull");
    assert!(!stamped(project.path()), "원본이 stamp와 다름");
    let a = open(&project, &locks, 2);
    assert_eq!(a.open_plan(), OpenPlan::FromText);
    assert_eq!(name_of(&a, 11), "Git pull");
    assert_eq!(a.generation_number(), 2, "cache를 원본에서 다시 만듦");
    assert!(stamped(project.path()));
    // cache도 원본과 같은 의미입니다(CLI가 읽는 generation).
    let snapshot = open_snapshot(project.path()).unwrap();
    let model = snapshot.document_model(sid(1)).unwrap();
    assert_eq!(model.record(sid(11)).unwrap().display_name, "Git pull");
    assert!(!a.has_unsaved_changes());
}

#[test]
fn a_cache_only_write_moves_forward_into_the_source() {
    let project = saved_project("source-forward");
    let locks = TempDir::new("source-forward-locks");
    drop(open(&project, &locks, 1));
    cli_rename(&project, &locks, 12, "CLI 편집");
    let a = open(&project, &locks, 2);
    assert_eq!(a.open_plan(), OpenPlan::Forward);
    assert_eq!(name_of(&a, 12), "CLI 편집");
    assert!(stamped(project.path()), "원본을 cache에 맞춰 씀");
    let text = TextSourceSession::open(project.path(), window(3)).unwrap();
    assert_eq!(
        text.document()
            .model()
            .record(sid(12))
            .unwrap()
            .display_name,
        "CLI 편집"
    );
}

#[test]
fn a_clean_window_applies_an_external_source_edit_as_one_undo_and_rebuilds_the_cache() {
    let project = saved_project("source-sync-clean");
    let locks = TempDir::new("source-sync-clean-locks");
    let mut a = open(&project, &locks, 1);
    text_rename(&project, 11, "외부 원본");
    assert_eq!(
        a.stale_state().unwrap(),
        Some((
            WriterProvenance::External,
            StaleResolution::ReconciliationPending
        ))
    );
    match a.sync_external(command(1)) {
        ExternalSync::Applied { changes, .. } => assert!(changes.is_some()),
        other => panic!("{other:?}"),
    }
    assert_eq!(name_of(&a, 11), "외부 원본");
    assert!(!a.has_unsaved_changes());
    // cache 맞춤 쓰기는 document thread가 아니라 다음 저장이 합니다.
    assert!(a.reconcile_pending() && a.needs_write());
    assert!(!stamped(project.path()));
    assert!(a.save_now().unwrap().written);
    assert!(!a.reconcile_pending());
    assert!(stamped(project.path()), "cache를 원본에 맞춤");
    assert_eq!(a.generation_number(), 2);
    assert!(matches!(
        a.sync_external(command(2)),
        ExternalSync::Unchanged
    ));
    // Undo 한 번이 외부 변경을 되돌리고(디스크와 달라 dirty), 저장하면 원본에 되돌린 이름이 씁니다.
    assert!(a.undo().unwrap());
    assert_eq!(name_of(&a, 11), "n11");
    assert!(a.has_unsaved_changes());
    assert!(a.save_now().unwrap().source_files > 0);
    let text = TextSourceSession::open(project.path(), window(5)).unwrap();
    assert_eq!(
        text.document()
            .model()
            .record(sid(11))
            .unwrap()
            .display_name,
        "n11"
    );
}

#[test]
fn a_clean_window_applies_a_cache_only_write_and_moves_it_into_the_source() {
    let project = saved_project("source-sync-forward");
    let locks = TempDir::new("source-sync-forward-locks");
    let mut a = open(&project, &locks, 1);
    cli_rename(&project, &locks, 12, "CLI 반영");
    assert!(matches!(
        a.sync_external(command(1)),
        ExternalSync::Applied { .. }
    ));
    assert_eq!(name_of(&a, 12), "CLI 반영");
    assert!(a.reconcile_pending());
    let outcome = a.save_now().unwrap();
    assert!(!outcome.written, "cache는 이미 최신");
    assert_eq!(outcome.source_files, 1, "원본을 cache에 맞춤");
    assert!(stamped(project.path()));
    let text = TextSourceSession::open(project.path(), window(3)).unwrap();
    assert_eq!(
        text.document()
            .model()
            .record(sid(12))
            .unwrap()
            .display_name,
        "CLI 반영"
    );
}

#[test]
fn a_dirty_window_keeps_its_edits_and_does_not_overwrite_an_external_source_edit() {
    let project = saved_project("source-sync-dirty");
    let locks = TempDir::new("source-sync-dirty-locks");
    let mut a = open(&project, &locks, 1);
    a.apply(&rename(a.document().unwrap(), 10, 12, "창 편집"), 0)
        .unwrap();
    text_rename(&project, 11, "외부 원본");
    let disk = read_source(project.path()).unwrap().unwrap().base;
    match a.sync_external(command(1)) {
        ExternalSync::Stale { writer, resolution } => {
            assert_eq!(writer, WriterProvenance::External);
            assert_eq!(resolution, StaleResolution::ReconciliationPending);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(name_of(&a, 12), "창 편집", "편집 보존");
    let error = a.save_now().unwrap_err();
    assert!(error.is_stale(), "{error}");
    assert_eq!(
        read_source(project.path()).unwrap().unwrap().base,
        disk,
        "원본을 덮어쓰지 않음"
    );
    assert!(matches!(
        a.resolve_stale(StaleSaveChoice::UseCurrentWindow),
        Err(StaleResolveError::ReconciliationRequired)
    ));
}

#[test]
fn two_app_windows_resolve_a_source_stale_with_the_two_choices() {
    let project = saved_project("source-two-windows");
    let locks = TempDir::new("source-two-windows-locks");
    let mut a = open(&project, &locks, 1);
    let mut b = open(&project, &locks, 2);
    a.apply(&rename(a.document().unwrap(), 1, 11, "A 편집"), 0)
        .unwrap();
    b.apply(&rename(b.document().unwrap(), 2, 12, "B 편집"), 0)
        .unwrap();
    assert!(a.save_now().unwrap().written);
    assert_eq!(
        b.stale_state().unwrap(),
        Some((WriterProvenance::AppWindow, StaleResolution::Choose))
    );
    assert!(b.save_now().unwrap_err().is_stale());
    match b.resolve_stale(StaleSaveChoice::UseCurrentWindow).unwrap() {
        StaleResolved::Saved(outcome) => assert!(outcome.wrote_anything()),
        other => panic!("{other:?}"),
    }
    assert!(stamped(project.path()));
    let c = open(&project, &locks, 3);
    assert_eq!(c.open_plan(), OpenPlan::Cache);
    assert_eq!(name_of(&c, 11), "n11", "B 창의 버전");
    assert_eq!(name_of(&c, 12), "B 편집");
    let text = TextSourceSession::open(project.path(), window(4)).unwrap();
    assert_eq!(
        model_snapshot_json(text.document().model()).unwrap(),
        json(&c),
        "원본과 cache가 같은 의미"
    );

    // UseLatestDisk: A가 다시 편집하고 B가 저장하면 A는 디스크로 돌아갑니다.
    a.apply(&rename(a.document().unwrap(), 3, 20, "A 버림"), 0)
        .unwrap();
    b.apply(&rename(b.document().unwrap(), 4, 20, "B 둘째"), 0)
        .unwrap();
    assert!(b.save_now().unwrap().wrote_anything());
    match a.resolve_stale(StaleSaveChoice::UseLatestDisk).unwrap() {
        StaleResolved::Reloaded { .. } => {}
        other => panic!("{other:?}"),
    }
    assert_eq!(name_of(&a, 20), "B 둘째");
    assert!(!a.has_unsaved_changes());
    assert!(!a.reconcile_pending(), "앱 창 저장은 원본·cache가 짝");
    assert_eq!(a.source_base(), b.source_base());
}

#[test]
fn a_broken_source_is_not_opened_and_names_the_file_and_line() {
    let project = saved_project("source-broken");
    let locks = TempDir::new("source-broken-locks");
    drop(open(&project, &locks, 1));
    let document = project.path().join("design/document.stellar.yaml");
    let mut text = std::fs::read_to_string(&document).unwrap();
    text.push_str("  : [깨짐\n");
    std::fs::write(&document, text).unwrap();
    match ProjectWindowSession::load_with_lock_dir(
        identity(project.path()),
        window(2),
        locks_of(&locks),
    ) {
        Err(WindowSessionLoadError::Source(diagnostic)) => {
            assert_eq!(diagnostic.path.as_deref(), Some("document.stellar.yaml"));
            assert!(diagnostic.line.is_some(), "{diagnostic}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_generation_the_text_source_cannot_hold_stays_chunk_only() {
    let project = saved_project_with(
        "source-chunk-only",
        &[&sample_document(), &other_document()],
    );
    let locks = TempDir::new("source-chunk-only-locks");
    let mut a = open(&project, &locks, 1);
    assert!(a.chunk_only_reason().is_some());
    assert_eq!(a.source_base(), None);
    assert!(
        read_source(project.path()).unwrap().is_none(),
        "원본을 만들지 않음"
    );
    a.apply(&rename(a.document().unwrap(), 1, 11, "chunk"), 0)
        .unwrap();
    let outcome = a.save_now().unwrap();
    assert!(outcome.written);
    assert_eq!(outcome.source_files, 0);
    assert!(read_source(project.path()).unwrap().is_none());
}

#[test]
fn the_active_style_defaults_set_is_mirrored_on_open_and_written_back_on_save() {
    let project = saved_project("source-style-defaults");
    let locks = TempDir::new("source-style-defaults-locks");
    let mut a = open(&project, &locks, 1);
    assert_eq!(
        a.document().unwrap().model().style_defaults(),
        stellar_document::css::style_defaults::CSS
    );
    let tx = CommandTransaction::new(command(5), stellar_commands::id::CommandOrigin::User)
        .with_command(
            a.revision(),
            stellar_commands::command::Command::SetStyleDefaults(
                stellar_commands::style_defaults::SetStyleDefaults {
                    name: stellar_document::css::style_defaults::REACT_NATIVE.into(),
                },
            ),
        );
    a.apply(&tx, 0).unwrap();
    assert!(a.has_unsaved_changes());
    a.save_now().unwrap();
    assert_eq!(
        stellar_project::document_io::session::active_style_defaults(project.path()).unwrap(),
        stellar_document::css::style_defaults::REACT_NATIVE
    );
    let b = open(&project, &locks, 2);
    assert_eq!(
        b.document().unwrap().model().style_defaults(),
        stellar_document::css::style_defaults::REACT_NATIVE
    );
    // 바꾸지 않은 창의 저장은 다른 쓰기가 바꾼 stellar.yaml을 되돌리지 않습니다.
    let mut c = open(&project, &locks, 3);
    stellar_project::manifest_style_defaults::set_active(
        project.path(),
        stellar_document::css::style_defaults::CSS,
    )
    .unwrap();
    c.apply(&rename(c.document().unwrap(), 6, 11, "다른 편집"), 0)
        .unwrap();
    c.save_now().unwrap();
    assert_eq!(
        stellar_project::document_io::session::active_style_defaults(project.path()).unwrap(),
        stellar_document::css::style_defaults::CSS
    );
}

#[test]
fn a_save_that_fails_on_the_source_does_not_leave_the_style_defaults_choice_written() {
    let project = saved_project("source-style-rollback");
    let locks = TempDir::new("source-style-rollback-locks");
    let mut a = open(&project, &locks, 1);
    let tx = CommandTransaction::new(command(6), stellar_commands::id::CommandOrigin::User)
        .with_command(
            a.revision(),
            stellar_commands::command::Command::SetStyleDefaults(
                stellar_commands::style_defaults::SetStyleDefaults {
                    name: stellar_document::css::style_defaults::REACT_NATIVE.into(),
                },
            ),
        );
    a.apply(&tx, 0).unwrap();
    a.apply(&rename(a.document().unwrap(), 7, 11, "창 편집"), 0)
        .unwrap();
    // 같은 문서 파일을 외부(Git pull·편집기)가 먼저 바꿉니다. 창 저장은 원본 journal에서 stale입니다.
    text_rename(&project, 11, "외부 원본");
    let manifest_before = std::fs::read(project.path().join("stellar.yaml")).unwrap();
    let error = a.save_now().unwrap_err();
    assert!(error.is_stale(), "{error}");
    // 실패한 저장은 아무것도 쓰지 않았다는 뜻이므로 먼저 되쓴 stellar.yaml도 되돌립니다.
    assert_eq!(
        stellar_project::document_io::session::active_style_defaults(project.path()).unwrap(),
        stellar_document::css::style_defaults::CSS
    );
    assert_eq!(
        std::fs::read(project.path().join("stellar.yaml")).unwrap(),
        manifest_before
    );
    assert!(a.has_unsaved_changes(), "창 편집은 그대로 dirty");
}

#[test]
fn opening_recovers_an_interrupted_source_save_before_reading_the_source() {
    let project = saved_project("source-recover");
    let locks = TempDir::new("source-recover-locks");
    let a = open(&project, &locks, 1);
    let expected = json(&a);
    drop(a);
    // 저장 journal 거래가 대상 하나를 바꾼 뒤 commit marker 전에 멈춘 상태를 만듭니다(ADR-0078 복구 대상).
    let target = "design/document.stellar.yaml";
    let path = project.path().join(target);
    let old = std::fs::read(&path).unwrap();
    let new = b"half written: [".to_vec();
    let tx = project.path().join(".stellar/journal/tx-0abc");
    std::fs::create_dir_all(tx.join("orig")).unwrap();
    std::fs::create_dir_all(tx.join("new")).unwrap();
    std::fs::write(tx.join("orig/0"), &old).unwrap();
    std::fs::write(tx.join("new/0"), &new).unwrap();
    std::fs::write(
        tx.join("manifest"),
        format!(
            "stellar-journal 1\ntx 0abc\nwrite {} {} {} {} {target}\n",
            ContentHash::compute(&old),
            old.len(),
            ContentHash::compute(&new),
            new.len()
        ),
    )
    .unwrap();
    std::fs::write(&path, &new).unwrap();
    // 열기는 원본을 읽기 전에 거래를 앞 세대로 되돌립니다. 반쯤 쓴 원본을 읽지 않습니다.
    let b = open(&project, &locks, 2);
    assert_eq!(std::fs::read(&path).unwrap(), old);
    assert!(!tx.exists(), "복구한 거래는 남지 않음");
    assert_eq!(b.open_plan(), OpenPlan::Cache);
    assert_eq!(json(&b), expected);
}

fn git(repo: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .current_dir(repo)
        .output()
        .ok()?;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(String::from_utf8(out.stdout).unwrap())
}

#[test]
fn cli_dry_run_then_window_save_and_reopen_show_one_view_file_in_git_diff() {
    // V.10.6 검증 열: CLI dry-run → 저장 → 재열기 → Git diff.
    let project = saved_project("source-git-diff");
    let locks = TempDir::new("source-git-diff-locks");
    std::fs::remove_dir(project.path().join(".git")).unwrap();
    drop(open(&project, &locks, 1));
    if git(project.path(), &["init", "-q", "-b", "main"]).is_none() {
        eprintln!("시스템 Git이 없어 Git diff 단계를 건너뜁니다");
        return;
    }
    git(project.path(), &["add", "."]);
    git(project.path(), &["commit", "-qm", "base"]);

    let snapshot = open_snapshot(project.path()).unwrap();
    let request = ApplyRequest {
        base_revision: snapshot.revision(),
        document: sid(1),
        commands: vec![CommandInput::Rename {
            node: sid(11),
            name: "dry".into(),
        }],
    };
    apply(
        project.path(),
        &request,
        ApplyMode::DryRun,
        &LockPolicy::Required {
            dir: locks.path().to_path_buf(),
        },
    )
    .unwrap();
    assert_eq!(
        git(project.path(), &["status", "--porcelain"]).unwrap(),
        "",
        "dry-run은 쓰지 않음"
    );

    let mut a = open(&project, &locks, 2);
    assert_eq!(a.open_plan(), OpenPlan::Cache);
    a.apply(&rename(a.document().unwrap(), 30, 11, "Git에 보일 이름"), 0)
        .unwrap();
    assert_eq!(a.save_now().unwrap().source_files, 1);
    let reopened = open(&project, &locks, 3);
    assert_eq!(reopened.open_plan(), OpenPlan::Cache);
    assert_eq!(name_of(&reopened, 11), "Git에 보일 이름");

    let names = git(project.path(), &["diff", "--name-only"]).unwrap();
    let sources: Vec<&str> = names
        .lines()
        .filter(|name| name.ends_with(".stellar.yaml"))
        .collect();
    assert_eq!(sources.len(), 1, "{names}");
    assert!(sources[0].starts_with("design/pages/"), "{names}");
    let diff = git(project.path(), &["diff", "--", sources[0]]).unwrap();
    assert!(diff.contains("Git에 보일 이름"), "{diff}");
    let untracked = git(
        project.path(),
        &["status", "--porcelain", "--untracked-files=all"],
    )
    .unwrap();
    assert!(
        !untracked.contains(".stellar/"),
        "stamp·journal은 Git 밖: {untracked}"
    );
}

/// 지금 원본에 token set 하나를 더한 파일 집합에서 새 token 파일 경로와 내용(외부 편집기·CLI가 token 파일을 씀).
fn token_file(project: &TempDir) -> (String, String) {
    use stellar_core_types::typed_value::TypedValue;
    use stellar_core_types::value::Length;
    use stellar_project::document_io::source_v4::project::{decode, encode};
    use stellar_project::document_io::source_v4::token::{TokenSetSource, TokenSource};
    let files = read_source(project.path()).unwrap().unwrap().files;
    let mut text = decode(&files).unwrap();
    text.token_sets.push(TokenSetSource {
        id: sid(3_000),
        name: "Brand Space".into(),
        tokens: vec![TokenSource {
            id: sid(3_001),
            name: "space/4".into(),
            expression: sid(3_002),
            value: TypedValue::Length(Length::new(16.0).unwrap()),
        }],
    });
    let encoded = encode(&text).unwrap().files;
    let (path, content) = encoded
        .into_iter()
        .find(|(path, _)| path.starts_with("tokens/"))
        .unwrap();
    (path, content)
}

#[test]
fn a_library_only_source_change_is_taken_even_by_a_dirty_window() {
    // ADR-0088 결정 5: 컴포넌트·token 파일만 바뀌면 문서 stale이 아니며, dirty 창도 그 의미를 받고 저장을 계속합니다.
    let project = saved_project("source-library-only");
    let locks = TempDir::new("source-library-only-locks");
    let mut a = open(&project, &locks, 1);
    a.apply(&rename(a.document().unwrap(), 40, 12, "dirty 편집"), 0)
        .unwrap();
    let (path, content) = token_file(&project);
    let target = project.path().join("design").join(&path);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, &content).unwrap();
    assert_eq!(a.stale_state().unwrap(), None, "문서 stale이 아님");
    match a.sync_external(command(9)) {
        ExternalSync::Applied { changes: None, .. } => {}
        other => panic!("{other:?}"),
    }
    assert!(a.has_unsaved_changes(), "편집 보존");
    assert_eq!(name_of(&a, 12), "dirty 편집");
    let outcome = a.save_now().unwrap();
    assert_eq!(outcome.source_files, 1, "View 파일 하나만 씀");
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        content,
        "token 파일 그대로"
    );
    assert!(stamped(project.path()));
    let text = TextSourceSession::open(project.path(), window(5)).unwrap();
    assert_eq!(
        text.document()
            .model()
            .record(sid(12))
            .unwrap()
            .display_name,
        "dirty 편집"
    );
    let reopened = open(&project, &locks, 2);
    assert_eq!(reopened.open_plan(), OpenPlan::Cache);
}

#[test]
fn a_cache_opened_window_saving_over_a_library_only_change_keeps_its_own_save() {
    // cache로 연 창은 문서 밖 의미를 모르므로 writer가 디스크 원본에서 읽습니다. 그 사이 token 파일만 바뀌었으면
    // writer는 디스크 base 위에 쓰고, 창은 그 저장을 자기 저장으로 받아야 합니다(다음 저장이 자기 저장을 stale로
    // 보지 않음).
    let project = saved_project("source-cache-library");
    let locks = TempDir::new("source-cache-library-locks");
    drop(open(&project, &locks, 1));
    let mut a = open(&project, &locks, 2);
    assert_eq!(a.open_plan(), OpenPlan::Cache);
    a.apply(&rename(a.document().unwrap(), 41, 12, "cache 창 편집"), 0)
        .unwrap();
    let (path, content) = token_file(&project);
    let target = project.path().join("design").join(&path);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, &content).unwrap();
    // 감시 event가 오기 전에 저장합니다.
    let outcome = a.save_now().unwrap();
    assert_eq!(outcome.source_files, 1, "View 파일 하나만 씀");
    assert!(!a.has_unsaved_changes(), "저장을 반영함");
    assert_eq!(
        a.source_base(),
        Some(&read_source(project.path()).unwrap().unwrap().base),
        "창의 원본 base가 디스크와 같음"
    );
    assert_eq!(a.stale_state().unwrap(), None, "자기 저장은 stale이 아님");
    a.apply(&rename(a.document().unwrap(), 42, 12, "다음 편집"), 0)
        .unwrap();
    assert!(a.save_now().unwrap().wrote_anything(), "다음 저장도 성공");
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        content,
        "token 파일 그대로"
    );
}
