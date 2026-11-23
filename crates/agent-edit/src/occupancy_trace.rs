//! 20만 node 문서의 lease 대상 계산·batch 적용 단일 점유 측정 (XXIX.3.4 검증 열, ADR-0124 결정 6).
//!
//! release build에서만 의미가 있어 기본 실행에서 뺍니다(`#[ignore]`). 실행:
//! `cargo test --release -p stellar-agent-edit --lib occupancy_trace -- --ignored --nocapture`.
//! 결과는 `docs/task-reports/XXIX.3.4.md`에 적습니다.

use std::time::Instant;

use stellar_ops::apply::{decode_commands, CommandInput};
use stellar_ops::commands::id::CommandId;
use stellar_ops::commands::lock::lease::{LeaseId, LeaseTable};
use stellar_ops::commands::state::DocumentState;
use stellar_ops::core_types::ids::StableId;
use stellar_ops::core_types::revision::DocumentRevision;

use crate::attached::edit::{agent_transaction, apply_approved};
use crate::lease::LeaseTargets;
use crate::provenance::AgentChangeProvenance;

const NODES: usize = 200_000;

fn micros(start: Instant) -> u128 {
    start.elapsed().as_micros()
}

/// `node` 이름 변경의 change set(문서 사본에서 만듦).
fn apply_probe(state: &DocumentState, node: StableId) -> stellar_ops::commands::change::ChangeSet {
    let records = state.model().to_records().unwrap();
    let (copy, _) =
        stellar_ops::commands::document::model::DocumentModel::from_records(records).unwrap();
    let mut copy = DocumentState::with_revision(copy, state.revision());
    let commands = decode_commands(&[CommandInput::Rename {
        node,
        name: "probe".to_string(),
    }])
    .unwrap();
    let transaction = stellar_ops::commands::transaction::CommandTransaction {
        id: CommandId::from_bytes([0xee; 16]),
        origin: stellar_ops::commands::id::CommandOrigin::User,
        requests: commands
            .into_iter()
            .map(|c| stellar_ops::commands::request::CommandRequest::new(state.revision(), c))
            .collect(),
    };
    copy.execute_transaction(&transaction).unwrap().changes
}

#[test]
#[ignore = "release 측정(문서 참고)"]
fn occupancy_trace_200k() {
    let model = stellar_ops::test_support::large_document(NODES);
    let records = model.to_records().unwrap();
    let root = records
        .nodes
        .iter()
        .find(|n| n.parent.is_none())
        .unwrap()
        .id;
    let leaves: Vec<StableId> = records
        .nodes
        .iter()
        .filter(|n| n.parent.is_some_and(|p| p != root))
        .map(|n| n.id)
        .collect();
    let groups: Vec<StableId> = records
        .nodes
        .iter()
        .filter(|n| n.parent == Some(root))
        .map(|n| n.id)
        .collect();
    let document = model.document_id();
    let mut state = DocumentState::with_revision(model, DocumentRevision(1));

    // lease 대상 계산: root 하나, group 400개(대상 상한 1000 안).
    let lease = LeaseId(StableId::new_v7(7, 7, 7));
    for (name, targets) in [
        ("root", vec![root]),
        ("groups-400", groups.iter().copied().take(400).collect()),
    ] {
        let request = LeaseTargets {
            document,
            targets,
            externals: Vec::new(),
        };
        let start = Instant::now();
        let resolved = request.resolve(state.model(), lease).unwrap();
        println!("lease-resolve {name}: {} us", micros(start));
        let mut table = LeaseTable::new();
        let start = Instant::now();
        table.insert(resolved).unwrap();
        println!("lease-table {name}: {} us", micros(start));
        // 판정 입력(ADR-0132 결정 1): 변경 node 하나의 적용 전 소속. 잠긴 subtree 크기와 관계없습니다.
        let probe = apply_probe(&state, leaves[0]);
        let start = Instant::now();
        let _ = table.snapshot(state.model(), &probe);
        println!(
            "lease-snapshot {name} (변경 node 1개): {} us",
            micros(start)
        );
    }

    // 사용자 편집 판정(XIX.4.3): 문서 전체 lease 안 node 이름 변경은 거부되고 문서는 그대로입니다. 적용·판정·되돌리기 전체.
    {
        let mut table = LeaseTable::new();
        table
            .insert(
                LeaseTargets {
                    document,
                    targets: vec![root],
                    externals: Vec::new(),
                }
                .resolve(state.model(), lease)
                .unwrap(),
            )
            .unwrap();
        let commands = decode_commands(&[CommandInput::Rename {
            node: leaves[1],
            name: "user".to_string(),
        }])
        .unwrap();
        let transaction = stellar_ops::commands::transaction::CommandTransaction {
            id: CommandId::from_bytes([0xef; 16]),
            origin: stellar_ops::commands::id::CommandOrigin::User,
            requests: commands
                .into_iter()
                .map(|c| stellar_ops::commands::request::CommandRequest::new(state.revision(), c))
                .collect(),
        };
        let start = Instant::now();
        let refused =
            stellar_ops::commands::lock::lease::execute_leased(&mut state, &transaction, &table);
        println!("user-edit in root lease (거부): {} us", micros(start));
        assert!(refused.is_err());
    }

    // batch 적용: rename N개, lease 판정·출처 기록 포함. lease는 문서 전체(root)와 group 하나(500 node) 두 경우.
    let mut provenance = AgentChangeProvenance::default();
    let session = crate::permission::AgentSessionId(StableId::new_v7(9, 9, 9));
    let one_group: Vec<StableId> = records
        .nodes
        .iter()
        .filter(|n| n.parent == Some(groups[0]))
        .map(|n| n.id)
        .collect();
    let cases: Vec<(&str, StableId, &[StableId], &[usize])> = vec![
        (
            "root",
            root,
            &leaves,
            &[1, 16, 64, 256, 512, 1024, 4096, 10_000],
        ),
        ("group", groups[0], &one_group, &[1, 64, 256, 499]),
    ];
    let mut round = 0u8;
    for (scope, target, nodes, counts) in cases {
        let mut table = LeaseTable::new();
        table
            .insert(
                LeaseTargets {
                    document,
                    targets: vec![target],
                    externals: Vec::new(),
                }
                .resolve(state.model(), lease)
                .unwrap(),
            )
            .unwrap();
        for &count in counts {
            round += 1;
            let inputs: Vec<CommandInput> = nodes
                .iter()
                .take(count)
                .map(|node| CommandInput::Rename {
                    node: *node,
                    name: format!("r{round}"),
                })
                .collect();
            let commands = decode_commands(&inputs).unwrap();
            let id = CommandId::from_bytes([round; 16]);
            let transaction = agent_transaction(state.revision(), lease, id, commands);
            let start = Instant::now();
            let applied = apply_approved(&mut state, &transaction, &table).unwrap();
            let apply_us = micros(start);
            let start = Instant::now();
            provenance
                .record_batch(session, "trace", lease, transaction, applied, state.model())
                .unwrap();
            println!(
                "batch {scope} {count}: apply {apply_us} us, record {} us",
                micros(start)
            );
        }
    }
}
