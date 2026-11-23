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
        let _ = table.snapshot(state.model());
        println!("lease-table {name}: {} us", micros(start));
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
        ("root", root, &leaves, &[1, 16, 64, 256, 1024, 4096, 10_000]),
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
