//! Semantic Diff/apply/inverse fuzz target (XVI.6.5).
//!
//! 고정 seed의 결정론적 `#[test]` harness입니다(새 의존성·nightly 없음, XVI.6.1·XVI.6.2와 같은
//! 방식). 반복 수는 `STELLAR_SEMANTIC_FUZZ_ITERATIONS`로 늘릴 수 있습니다.
//!
//! 대상과 판정:
//!
//! 1. **유효 A/B**: random valid document A와, A를 문서 validator(VI.10.1)를 통과하는 편집으로
//!    바꾼 B(또는 무관한 valid document)에 대해 `apply(diff(A,B), A) == B`,
//!    `apply(inverse(diff(A,B)), B) == A`, diff·snapshot canonical bytes round-trip.
//! 2. **decoder**: 유효 diff·snapshot bytes를 무작위로 변형해 decode — panic 없이 Ok 또는 구조화
//!    오류만 허용하고, 수용한 입력은 다시 인코딩하면 byte-identical(canonical 성질).
//! 3. **stale 적용**: 수용한 diff와 다른 쌍의 diff를 엉뚱한 snapshot에 적용 — panic 없이 Ok 또는
//!    구조화 오류만 허용.
//!
//! 실패 입력은 `CARGO_TARGET_TMPDIR/semantic-diff-fuzz/`에 hex로 저장하고 테스트를 실패시킵니다.

use std::panic::{self, AssertUnwindSafe};

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::value::Size2;
use stellar_document::fragment::FragmentPayload;
use stellar_document::records::{DocumentRecords, NodePayload, NodeRecord};
use stellar_document::test_support::random_document::{random_valid_document, SplitMix64};
use stellar_document::validate::validate_document;
use stellar_semantic_diff::apply::apply;
use stellar_semantic_diff::build::SnapshotBuilder;
use stellar_semantic_diff::codec::{snapshot_from_bytes, snapshot_to_bytes};
use stellar_semantic_diff::diff::{diff, SemanticDiff};
use stellar_semantic_diff::snapshot::SemanticSnapshot;

const DEFAULT_ITERATIONS: u64 = 400;

fn iterations() -> u64 {
    std::env::var("STELLAR_SEMANTIC_FUZZ_ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_ITERATIONS)
}

fn build(records: &DocumentRecords) -> SemanticSnapshot {
    let mut builder = SnapshotBuilder::new();
    builder
        .add_document(records)
        .expect("valid document는 snapshot이 됩니다");
    builder.finish()
}

fn is_container(payload: &NodePayload) -> bool {
    matches!(payload, NodePayload::Frame(_) | NodePayload::Fragment(_))
}

/// 편집 한 번(결과가 validator를 통과하지 않을 수 있음).
fn mutate_once(
    records: &DocumentRecords,
    rng: &mut SplitMix64,
    fresh: StableId,
) -> DocumentRecords {
    let mut out = records.clone();
    if out.nodes.is_empty() {
        return out;
    }
    let index = rng.below(out.nodes.len() as u64) as usize;
    let containers: Vec<StableId> = out
        .nodes
        .iter()
        .filter(|n| is_container(&n.payload))
        .map(|n| n.id)
        .collect();
    match rng.below(7) {
        0 => out.nodes[index].visible = !out.nodes[index].visible,
        1 => out.nodes[index].display_name = format!("편집-{}", rng.below(1000)),
        2 => {
            if let NodePayload::Frame(frame) = &mut out.nodes[index].payload {
                let width = frame.size.width() + 1.0 + rng.below(8) as f32;
                if let Ok(size) = Size2::new(width, frame.size.height()) {
                    frame.size = size;
                }
            }
        }
        3 => {
            let current = out.nodes[index].order_key.clone();
            out.nodes[index].order_key = if rng.below(2) == 0 {
                OrderKey::between(None, Some(&current))
            } else {
                OrderKey::between(Some(&current), None)
            };
        }
        4 => {
            if out.nodes[index].parent.is_some() && !containers.is_empty() {
                let target = containers[rng.below(containers.len() as u64) as usize];
                out.nodes[index].parent = Some(target);
            }
        }
        5 => {
            let id = out.nodes[index].id;
            let has_children = out.nodes.iter().any(|n| n.parent == Some(id));
            if !has_children && out.nodes[index].parent.is_some() {
                out.nodes.remove(index);
                out.modifier_stacks.retain(|s| s.node != id);
            }
        }
        _ => {
            if !containers.is_empty() {
                let parent = containers[rng.below(containers.len() as u64) as usize];
                let last = out
                    .nodes
                    .iter()
                    .filter(|n| n.parent == Some(parent))
                    .map(|n| n.order_key.clone())
                    .max();
                out.nodes.push(NodeRecord {
                    id: fresh,
                    parent: Some(parent),
                    order_key: OrderKey::between(last.as_ref(), None),
                    visible: true,
                    locked: false,
                    display_name: "새 node".into(),
                    payload: NodePayload::Fragment(FragmentPayload),
                });
            }
        }
    }
    out
}

/// validator를 통과하는 편집만 쌓은 B.
fn valid_edit(records: &DocumentRecords, seed: u64) -> DocumentRecords {
    let mut rng = SplitMix64::new(seed ^ 0xD1FF_5EED);
    let mut out = records.clone();
    for step in 0..1 + rng.below(10) {
        let fresh = StableId::new_v7(2_100_000_000_000 + seed * 64 + step, 0x0f, seed);
        let candidate = mutate_once(&out, &mut rng, fresh);
        if validate_document(&candidate).is_ok() {
            out = candidate;
        }
    }
    out
}

/// 입력 bytes 변형(bit 뒤집기·절단·삽입·삭제·경계값 덮어쓰기).
fn mutate_bytes(bytes: &[u8], rng: &mut SplitMix64) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for _ in 0..1 + rng.below(4) {
        let len = out.len().max(1) as u64;
        let at = rng.below(len) as usize;
        match rng.below(6) {
            0 if !out.is_empty() => {
                let index = at.min(out.len() - 1);
                out[index] ^= 1 << rng.below(8);
            }
            1 => out.truncate(at),
            2 => out.insert(at.min(out.len()), rng.below(256) as u8),
            3 if at < out.len() => {
                out.remove(at);
            }
            4 if at + 4 <= out.len() => out[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes()),
            _ => {
                if let Some(byte) = out.get_mut(at) {
                    *byte = [0x00, 0x01, 0x7F, 0x80, 0xFF][rng.below(5) as usize];
                }
            }
        }
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 실패를 기록하고 메시지를 반환합니다.
fn record_failure(name: &str, seed: u64, input: &[u8], message: &str) -> String {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("semantic-diff-fuzz");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{name}-{seed}.hex"));
    let _ = std::fs::write(&path, format!("# {message}\n{}\n", hex(input)));
    format!("{name} seed {seed}: {message} (입력: {})", path.display())
}

/// panic을 잡아 실패 메시지로 바꿉니다.
fn guarded<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    panic::catch_unwind(AssertUnwindSafe(f)).map_err(|payload| {
        payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "panic".into())
    })
}

#[test]
fn valid_pairs_apply_and_inverse_reproduce() {
    let mut failures = Vec::new();
    let mut nonempty = 0u64;
    for seed in 0..iterations() {
        let a_records = random_valid_document(seed).to_records().unwrap();
        let edited = valid_edit(&a_records, seed);
        assert!(validate_document(&edited).is_ok(), "seed {seed}: B는 valid");
        let unrelated = random_valid_document(seed + 1_000_000)
            .to_records()
            .unwrap();
        for (label, b_records) in [("edit", &edited), ("unrelated", &unrelated)] {
            let outcome = guarded(|| {
                let a = build(&a_records);
                let b = build(b_records);
                let forward = diff(&a, &b);
                let backward = forward.inverse();
                let bytes = forward.to_canonical_bytes();
                let checks = [
                    ("apply(diff)==B", apply(&a, &forward).as_ref() == Ok(&b)),
                    ("apply(inverse)==A", apply(&b, &backward).as_ref() == Ok(&a)),
                    (
                        "diff bytes round-trip",
                        SemanticDiff::from_canonical_bytes(&bytes).as_ref() == Ok(&forward),
                    ),
                    (
                        "snapshot bytes round-trip",
                        snapshot_from_bytes(&snapshot_to_bytes(&b)).as_ref() == Ok(&b),
                    ),
                ];
                (
                    forward.len(),
                    checks.iter().find(|(_, ok)| !ok).map(|(n, _)| *n),
                )
            });
            match outcome {
                Ok((len, None)) => nonempty += u64::from(len > 0),
                Ok((_, Some(check))) => failures.push(format!("{label} seed {seed}: {check} 실패")),
                Err(panic) => failures.push(format!("{label} seed {seed}: panic {panic}")),
            }
        }
    }
    eprintln!(
        "valid pairs: {} 쌍, 비어 있지 않은 diff {nonempty}",
        iterations() * 2
    );
    assert!(failures.is_empty(), "{failures:#?}");
    assert!(
        nonempty >= iterations(),
        "편집이 실제 diff를 만들어야 합니다: {nonempty}"
    );
}

#[test]
fn malformed_bytes_never_panic_and_accepted_inputs_are_canonical() {
    let mut failures = Vec::new();
    let (mut accepted, mut rejected) = (0u64, 0u64);
    for seed in 0..iterations() {
        let a = build(&random_valid_document(seed).to_records().unwrap());
        let b = build(&valid_edit(
            &random_valid_document(seed).to_records().unwrap(),
            seed,
        ));
        let forward = diff(&a, &b);
        let mut rng = SplitMix64::new(seed ^ 0xBAD_B17E);
        let sources = [
            ("diff", forward.to_canonical_bytes()),
            ("snapshot", snapshot_to_bytes(&b)),
        ];
        for (name, valid) in sources {
            for round in 0..4 {
                let input = mutate_bytes(&valid, &mut rng);
                let outcome = guarded(|| match name {
                    "diff" => SemanticDiff::from_canonical_bytes(&input)
                        .map(|d| d.to_canonical_bytes() == input),
                    _ => snapshot_from_bytes(&input).map(|s| snapshot_to_bytes(&s) == input),
                });
                match outcome {
                    Ok(Ok(true)) => accepted += 1,
                    Ok(Ok(false)) => failures.push(record_failure(
                        name,
                        seed * 4 + round,
                        &input,
                        "수용한 입력의 재인코딩이 다릅니다",
                    )),
                    Ok(Err(_)) => rejected += 1,
                    Err(panic) => {
                        failures.push(record_failure(name, seed * 4 + round, &input, &panic))
                    }
                }
            }
        }
    }
    eprintln!("malformed bytes: 수용 {accepted}, 거부 {rejected}");
    assert!(failures.is_empty(), "{failures:#?}");
    assert!(
        accepted > 0 && rejected > 0,
        "수용 {accepted}, 거부 {rejected}"
    );
}

#[test]
fn stale_and_foreign_diffs_never_panic() {
    let mut failures = Vec::new();
    let mut errors = 0u64;
    for seed in 0..iterations() {
        let a = build(&random_valid_document(seed).to_records().unwrap());
        let b = build(&valid_edit(
            &random_valid_document(seed).to_records().unwrap(),
            seed,
        ));
        let c = build(&random_valid_document(seed + 7).to_records().unwrap());
        let forward = diff(&a, &b);
        let mut rng = SplitMix64::new(seed ^ 0x57A1E);
        // 변형 bytes 중 decoder가 수용한 diff도 적용해 봅니다.
        let mutated = mutate_bytes(&forward.to_canonical_bytes(), &mut rng);
        let decoded = SemanticDiff::from_canonical_bytes(&mutated).ok();
        let outcome = guarded(|| {
            let mut results = vec![
                apply(&c, &forward).is_err(),
                apply(&b, &forward).is_err() || forward.is_empty(),
                apply(&a, &forward.inverse()).is_err() || forward.is_empty(),
            ];
            if let Some(decoded) = &decoded {
                results.push(apply(&a, decoded).is_err());
                results.push(apply(&c, decoded).is_err());
            }
            results.into_iter().filter(|e| *e).count() as u64
        });
        match outcome {
            Ok(count) => errors += count,
            Err(panic) => failures.push(record_failure("apply", seed, &mutated, &panic)),
        }
    }
    eprintln!("stale/foreign apply: 구조화 오류 {errors}");
    assert!(failures.is_empty(), "{failures:#?}");
    assert!(errors > 0, "stale 적용이 구조화 오류를 내야 합니다");
}

#[test]
fn campaign_is_deterministic() {
    let run = || {
        (0..20)
            .map(|seed| {
                let a = build(&random_valid_document(seed).to_records().unwrap());
                let b = build(&valid_edit(
                    &random_valid_document(seed).to_records().unwrap(),
                    seed,
                ));
                let mut rng = SplitMix64::new(seed);
                let bytes = diff(&a, &b).to_canonical_bytes();
                (bytes.clone(), mutate_bytes(&bytes, &mut rng))
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
}
