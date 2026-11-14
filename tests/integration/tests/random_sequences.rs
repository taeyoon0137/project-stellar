//! 종단 시나리오: 200개 seed의 무작위 command sequence.
//!
//! seed마다 random valid document(VI.10.5)에서 시작해 창 history로 무작위 동작을 이어 갑니다: 요청 1개에서
//! 3개짜리 transaction(뒤 요청은 앞 요청과 충돌해 거부될 수 있음), 선택 변경, undo, redo. 단계마다
//!
//! - transaction canonical bytes는 decode 왕복하고,
//! - 성공한 실행 뒤 문서는 전체 validator(VI.10.1)를 통과하며 revision이 정확히 1 오르고,
//! - 거부된 transaction은 레코드·revision·history를 바꾸지 않고,
//! - 창 선택은 문서에 있는 node만 담습니다.
//!
//! 끝에서 전체 undo(초기 상태)와 전체 redo(최종 상태)를 확인하고, 모든 seed의 최종 문서를 document_io 메모리
//! 왕복으로 확인합니다(4번째 seed마다 중간 상태도). 마지막으로 20번째 seed마다의 최종 문서를 한 generation에
//! 저장하고 디스크에서 lazy로 하나씩 다시 열어 의미 동일성을 확인합니다. 저장 transaction은 chunk 파일마다
//! fsync하므로 디스크 단계는 일부 seed로 제한해 테스트 시간을 수 초로 유지합니다.

mod common;

use common::{
    assert_same_document, assert_same_records, assert_valid, save_documents, Editor, TempDir,
};
use stellar_commands::selection::{Selection, SetSelection};
use stellar_commands::test_support::{random_valid_request, FreshIds};
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::ids::StableId;
use stellar_document::model::DocumentModel;
use stellar_document::test_support::random_document::{random_valid_document, SplitMix64};
use stellar_document::validate::validate_document;
use stellar_project::document_io::load::{load_document, MemoryChunks};
use stellar_project::document_io::serialize::serialize_document;
use stellar_project::identity::parse_project_id;
use stellar_project::saved_project::SavedProjectLoader;
use stellar_project::scaffold::write_default_scaffold;

const SEEDS: u64 = 200;
const STEPS: usize = 12;
/// 디스크 저장·재열기 대상 seed 간격.
const DISK_EVERY: u64 = 20;
const SPACE: u64 = 41;
const PROJECT_ID: &str = "0195c530-3acd-7bc1-8414-c213f61cb968";

/// seed 하나의 실행 통계.
#[derive(Default)]
struct Stats {
    committed: usize,
    rejected: usize,
    selections: usize,
    undos: usize,
    redos: usize,
}

fn live_nodes(model: &DocumentModel) -> Vec<StableId> {
    model
        .columns()
        .iter_live()
        .filter_map(|handle| model.stable_id(handle))
        .collect()
}

fn assert_selection_live(editor: &Editor, label: &str) {
    for id in editor.selection().ids() {
        assert!(
            editor.model().contains(*id),
            "{label}: 선택에 없는 node {id}"
        );
    }
}

/// document_io 메모리 왕복: 직렬화 → chunk 집합 → loader.
fn memory_round_trip(model: &DocumentModel, label: &str) {
    let serialized = serialize_document(model).expect("직렬화");
    let mut chunks = MemoryChunks::new();
    chunks.insert_document(&serialized);
    let loaded = load_document(&serialized.manifest.payload, &mut chunks).expect("로드");
    assert_same_records(model, &loaded.model, label);
    let records = model.to_records().expect("레코드");
    assert_eq!(
        loaded.diagnostics,
        validate_document(&records).expect("유효"),
        "{label}: 진단"
    );
}

/// 한 seed를 실행하고 (최종 문서, 통계)를 돌려줍니다.
fn run_seed(seed: u64) -> (DocumentModel, Stats) {
    let mut editor = Editor::new(random_valid_document(seed), SPACE * 1_000 + seed);
    let mut rng = SplitMix64::new(seed ^ 0x5E9_0000);
    let mut ids = FreshIds::new(seed ^ 0x5E9_0001);
    let mut stats = Stats::default();
    let initial_records = editor.model().to_records().expect("레코드");
    let initial_snapshot = editor.snapshot();
    let initial_selection = editor.selection();

    for step in 0..STEPS {
        let label = format!("seed {seed} step {step}");
        match rng.below(12) {
            0..=5 => {
                let mut commands = Vec::new();
                for _ in 0..1 + rng.below(3) {
                    if let Some(request) =
                        random_valid_request(&editor.state, &mut rng, &mut ids, 16)
                    {
                        commands.push(request.command);
                    }
                }
                if commands.is_empty() {
                    continue;
                }
                let transaction = editor.transaction(commands);
                assert_eq!(
                    CommandTransaction::decode(&transaction.encode()).expect("decode"),
                    transaction,
                    "{label}: canonical bytes 왕복"
                );
                let before = editor.model().to_records().expect("레코드");
                let revision = editor.state.revision();
                let entries = editor.window.history().len();
                match editor
                    .window
                    .record_document(&mut editor.state, &transaction, None)
                {
                    Ok(_) => {
                        stats.committed += 1;
                        assert_eq!(editor.state.revision().0, revision.0 + 1, "{label}");
                        assert_eq!(editor.window.history().len(), entries + 1, "{label}");
                        assert_eq!(editor.window.history().redo_len(), 0, "{label}: redo 폐기");
                        assert_valid(editor.model(), &label);
                    }
                    Err(_) => {
                        stats.rejected += 1;
                        assert_eq!(
                            editor.model().to_records().expect("레코드"),
                            before,
                            "{label}: 거부는 무변경"
                        );
                        assert_eq!(editor.state.revision(), revision, "{label}");
                        assert_eq!(editor.window.history().len(), entries, "{label}");
                    }
                }
            }
            6 | 7 => {
                let nodes = live_nodes(editor.model());
                if nodes.is_empty() {
                    continue;
                }
                let picked: Vec<StableId> =
                    (0..1 + rng.below(3)).map(|_| *rng.pick(&nodes)).collect();
                let event = SetSelection::new(Selection::new(picked, None));
                if editor
                    .window
                    .record_selection(&editor.state, &event)
                    .expect("선택 기록")
                    .is_some()
                {
                    stats.selections += 1;
                }
            }
            8 | 9 => {
                if editor.undo() {
                    stats.undos += 1;
                    assert_valid(editor.model(), &label);
                }
            }
            _ => {
                if editor.redo() {
                    stats.redos += 1;
                    assert_valid(editor.model(), &label);
                }
            }
        }
        assert_selection_live(&editor, &label);
        if step == STEPS / 2 && seed.is_multiple_of(4) {
            memory_round_trip(editor.model(), &format!("{label}: 중간 왕복"));
        }
    }

    // 전체 undo → 초기 상태, 전체 redo → 최종 상태.
    let final_records = editor.model().to_records().expect("레코드");
    let final_selection = editor.selection();
    let depth = editor.window.history().len();
    let final_redo = editor.window.history().redo_len();
    for _ in 0..depth {
        assert!(editor.undo(), "seed {seed}: undo 항목");
    }
    assert!(!editor.undo(), "seed {seed}: 더 되돌릴 것 없음");
    assert_eq!(
        editor.model().to_records().expect("레코드"),
        initial_records,
        "seed {seed}: 전체 undo 레코드"
    );
    assert_eq!(
        editor.snapshot(),
        initial_snapshot,
        "seed {seed}: 전체 undo"
    );
    assert_eq!(
        editor.selection(),
        initial_selection,
        "seed {seed}: 초기 선택"
    );
    // 끝에서 되돌려 둔 redo 갈래가 있었다면 그 위에 쌓였으므로, 되돌린 수만큼만 다시 적용합니다.
    let pending = final_redo;
    assert_eq!(
        editor.window.history().redo_len(),
        depth + pending,
        "seed {seed}"
    );
    for _ in 0..depth {
        assert!(editor.redo(), "seed {seed}: redo 항목");
    }
    assert_eq!(editor.window.history().redo_len(), pending, "seed {seed}");
    assert_eq!(
        editor.model().to_records().expect("레코드"),
        final_records,
        "seed {seed}: 전체 redo 레코드"
    );
    assert_eq!(
        editor.selection(),
        final_selection,
        "seed {seed}: 최종 선택"
    );
    memory_round_trip(editor.model(), &format!("seed {seed}: 최종 왕복"));
    (editor.state.into_model(), stats)
}

#[test]
fn two_hundred_seed_command_sequences_survive_history_and_disk_round_trip() {
    let mut on_disk = Vec::new();
    let mut totals = Stats::default();
    for seed in 0..SEEDS {
        let (model, stats) = run_seed(seed);
        if seed.is_multiple_of(DISK_EVERY) {
            on_disk.push(model);
        }
        totals.committed += stats.committed;
        totals.rejected += stats.rejected;
        totals.selections += stats.selections;
        totals.undos += stats.undos;
        totals.redos += stats.redos;
    }
    // 분포 회귀 방지: 각 동작이 충분히 섞였는지.
    assert!(totals.committed > 600, "확정 {}", totals.committed);
    assert!(totals.rejected > 15, "거부 {}", totals.rejected);
    assert!(totals.selections > 150, "선택 {}", totals.selections);
    assert!(totals.undos > 150, "undo {}", totals.undos);
    assert!(totals.redos > 30, "redo {}", totals.redos);

    // 최종 문서 일부를 한 generation에 저장하고 디스크에서 하나씩 lazy 재열기.
    let sandbox = TempDir::new("sequences");
    let root = sandbox.path.join("project");
    std::fs::create_dir_all(&root).expect("프로젝트 폴더");
    write_default_scaffold(&root, "무작위 sequence", PROJECT_ID).expect("scaffold");
    let project_id = parse_project_id(PROJECT_ID).expect("project id");
    let models: Vec<&DocumentModel> = on_disk.iter().collect();
    save_documents(&root, project_id, &models, &[]);
    let project = SavedProjectLoader::new().open(&root).expect("재열기");
    assert_eq!(
        project.document_ids().expect("문서 id").len(),
        on_disk.len()
    );
    for model in &on_disk {
        let label = format!("문서 {}", model.document_id());
        let loaded = project
            .load_document(model.document_id())
            .unwrap_or_else(|e| panic!("{label}: 로드 실패: {e}"));
        assert_same_document(model, &loaded.model, &label);
    }
}
