//! Semantic Merge fuzz target (XVI.6.6).
//!
//! 고정 seed의 결정론적 `#[test]` harness입니다(새 의존성·nightly 없음, XVI.6.5와 같은 방식). 반복 수는
//! `STELLAR_SEMANTIC_MERGE_FUZZ_ITERATIONS`로 늘릴 수 있습니다.
//!
//! 허용 결과는 셋뿐입니다(invalid silent merge 0).
//!
//! 1. clean 결과: 세 입력 어디에도 없던 snapshot 위반이 0이어야 합니다.
//! 2. conflict 결과: conflict가 `(StableId, 대상)` 순서로 유일하고, 결과 bytes가 round-trip합니다.
//! 3. 구조화 오류(`MergeError`).
//!
//! panic은 실패입니다. 입력은 `world` fixture에 무작위 편집(이름·표시·삭제·추가·이동·token 타입과 값·interface
//! Property 목록·override·참조)을 양쪽에 따로 적용해 만들고, 편집 결과가 그쪽에서 이미 invalid여도 됩니다. 두
//! 번째 대상은 유효 `MergeInput` bytes를 무작위로 변형한 decoder 입력입니다. 실패 입력은
//! `CARGO_TARGET_TMPDIR/semantic-merge-fuzz/`에 hex로 저장합니다.

mod common;

use std::panic::{self, AssertUnwindSafe};

use common::*;
use stellar_semantic_diff::keys;
use stellar_semantic_diff::snapshot::{
    NodeKind, ObjectKind, Origin, SemanticObject, SemanticSnapshot,
};
use stellar_semantic_diff::validate::introduced_violations;
use stellar_semantic_diff::value::SemanticValue;
use stellar_semantic_merge::merge::merge;
use stellar_semantic_merge::model::{MergeInput, MergeResult};

const DEFAULT_ITERATIONS: u64 = 400;

fn iterations() -> u64 {
    std::env::var("STELLAR_SEMANTIC_MERGE_FUZZ_ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_ITERATIONS)
}

/// SplitMix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// 문서 scene node 목록.
fn scene_nodes(snapshot: &SemanticSnapshot) -> Vec<u64> {
    (0..120)
        .filter(|n| {
            snapshot.get(sid(*n)).is_some_and(|o| {
                matches!(o.kind(), ObjectKind::Node(_)) && matches!(o.origin(), Origin::Document(_))
            })
        })
        .collect()
}

/// 편집 한 번. 결과가 그쪽에서 invalid여도 됩니다.
fn mutate(snapshot: &SemanticSnapshot, rng: &mut Rng, fresh: &mut u64) -> SemanticSnapshot {
    let nodes = scene_nodes(snapshot);
    let pick = |rng: &mut Rng| nodes[rng.below(nodes.len() as u64) as usize];
    let has = |id: u64| snapshot.contains(sid(id));
    match rng.below(12) {
        0 if !nodes.is_empty() => {
            let id = pick(rng);
            edit(snapshot, id, |o| {
                set(o, keys::DISPLAY_NAME, text(&format!("n{}", rng.below(4))))
            })
        }
        1 if !nodes.is_empty() => {
            let id = pick(rng);
            let visible = rng.below(2) == 0;
            edit(snapshot, id, |o| {
                set(o, keys::VISIBLE, SemanticValue::Bool(visible))
            })
        }
        2 if nodes.len() > 1 => {
            let id = pick(rng);
            let mut out = snapshot.clone();
            out.remove(sid(id));
            out
        }
        3 if !nodes.is_empty() => {
            let parent = pick(rng);
            *fresh += 1;
            let mut out = snapshot.clone();
            let _ = out.insert(node(
                *fresh,
                Some(parent),
                0x10 + rng.below(0xE0) as u8,
                "new",
            ));
            out
        }
        4 if nodes.len() > 1 => {
            let id = pick(rng);
            let parent = pick(rng);
            let order = key(0x10 + rng.below(0xE0) as u8);
            edit(snapshot, id, |o| {
                if o.parent().is_some() && parent != id {
                    o.set_position(Some(sid(parent)), Some(order));
                }
            })
        }
        5 if has(TOKEN) => edit(snapshot, TOKEN, |o| {
            set(o, keys::TOKEN_TYPE, SemanticValue::Uint(1 + rng.below(3)))
        }),
        6 if has(TOKEN) => edit(snapshot, TOKEN, |o| {
            set(o, keys::TOKEN_VALUE, SemanticValue::Uint(rng.below(5)))
        }),
        7 if has(INNER) => {
            let all = [INNER_PROP, INNER_PROP_2, 34];
            let keep: Vec<u64> = all.into_iter().filter(|_| rng.below(2) == 0).collect();
            set_properties(snapshot, INNER, &keep)
        }
        8 if has(INSTANCE) || has(NESTED) => {
            let owner = if has(INSTANCE) && (rng.below(2) == 0 || !has(NESTED)) {
                INSTANCE
            } else {
                NESTED
            };
            let member = [INNER_PROP, INNER_PROP_2, 34][rng.below(3) as usize];
            let value = if rng.below(3) == 0 {
                SemanticValue::Id(sid(OUTER_PROP))
            } else {
                text("v")
            };
            put(snapshot, &override_path(owner, member), value)
        }
        9 if !nodes.is_empty() => {
            // 다른 node를 clipping 대상으로 가리킵니다(대상이 다른 쪽에서 지워질 수 있음).
            let id = pick(rng);
            let target = pick(rng);
            let path = snapshot
                .get(sid(id))
                .unwrap()
                .path(&[keys::frame::CLIPPING]);
            put(snapshot, &path, SemanticValue::Id(sid(target)))
        }
        10 if has(INNER) => {
            let mut out = snapshot.clone();
            out.remove(sid(INNER));
            if rng.below(2) == 0 {
                out.remove(sid(INNER_ROOT));
            }
            out
        }
        11 if !nodes.is_empty() => {
            *fresh += 1;
            let mut instance = SemanticObject::new(
                sid(*fresh),
                ObjectKind::Node(NodeKind::ComponentInstance),
                Origin::Document(sid(DOCUMENT)),
            )
            .with_position(
                Some(sid(pick(rng))),
                Some(key(0x10 + rng.below(0xE0) as u8)),
            );
            set(&mut instance, keys::DISPLAY_NAME, text("instance"));
            refer(&mut instance, INNER);
            let mut out = snapshot.clone();
            let _ = out.insert(instance);
            out
        }
        _ => snapshot.clone(),
    }
}

fn side(base: &SemanticSnapshot, rng: &mut Rng, fresh: &mut u64) -> SemanticSnapshot {
    let mut out = base.clone();
    for _ in 0..1 + rng.below(4) {
        out = mutate(&out, rng, fresh);
    }
    out
}

fn record_failure(label: &str, seed: u64, input: &MergeInput, message: &str) -> String {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("semantic-merge-fuzz");
    let _ = std::fs::create_dir_all(&dir);
    let hex: String = input
        .to_canonical_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let path = dir.join(format!("{label}-{seed}.hex"));
    let _ = std::fs::write(&path, hex);
    format!("{label} seed {seed}: {message} ({})", path.display())
}

/// 결과가 허용 결과 셋 중 하나인지(XVI.6.6). 위반이면 설명을 돌려줍니다.
fn judge(input: &MergeInput, result: &MergeResult) -> Option<String> {
    if result.is_clean() {
        let introduced =
            introduced_violations(&result.merged, &[&input.base, &input.ours, &input.theirs]);
        if !introduced.is_empty() {
            return Some(format!("invalid silent merge: {introduced:?}"));
        }
    }
    for pair in result.conflicts.windows(2) {
        if (pair[0].object, &pair[0].target) >= (pair[1].object, &pair[1].target) {
            return Some(format!("conflict 순서·유일성 위반: {pair:?}"));
        }
    }
    match MergeResult::from_canonical_bytes(&result.to_canonical_bytes()) {
        Ok(decoded) if &decoded == result => None,
        other => Some(format!("결과 round-trip 실패: {other:?}")),
    }
}

fn guarded<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    let hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let outcome = panic::catch_unwind(AssertUnwindSafe(f));
    panic::set_hook(hook);
    outcome.map_err(|panic| {
        panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "panic".into())
    })
}

fn random_input(seed: u64) -> MergeInput {
    let mut rng = Rng(seed);
    let base = world();
    let mut fresh = 1000 + seed * 16;
    let ours = side(&base, &mut rng, &mut fresh);
    let theirs = side(&base, &mut rng, &mut fresh);
    MergeInput { base, ours, theirs }
}

#[test]
fn random_merges_are_valid_or_explicit_conflicts() {
    let mut failures = Vec::new();
    let (mut clean, mut conflicted, mut errors) = (0u64, 0u64, 0u64);
    for seed in 0..iterations() {
        let input = random_input(seed);
        match guarded(|| merge(&input)) {
            Err(panic) => failures.push(record_failure("merge", seed, &input, &panic)),
            Ok(Err(_)) => errors += 1,
            Ok(Ok(result)) => {
                if let Some(problem) = judge(&input, &result) {
                    failures.push(record_failure("merge", seed, &input, &problem));
                } else if result.is_clean() {
                    clean += 1;
                } else {
                    conflicted += 1;
                }
            }
        }
    }
    eprintln!("merge fuzz: clean {clean}, conflict {conflicted}, 구조화 오류 {errors}");
    assert!(failures.is_empty(), "{failures:#?}");
    assert!(
        clean > 0 && conflicted > 0,
        "clean {clean}, conflict {conflicted}"
    );
}

#[test]
fn mutated_input_bytes_never_panic_or_merge_silently_invalid() {
    let mut failures = Vec::new();
    let (mut accepted, mut rejected) = (0u64, 0u64);
    for seed in 0..iterations() / 2 {
        let input = random_input(seed);
        let mut bytes = input.to_canonical_bytes();
        let mut rng = Rng(seed ^ 0xFADE);
        for _ in 0..1 + rng.below(4) {
            let at = rng.below(bytes.len() as u64) as usize;
            match rng.below(3) {
                0 => bytes[at] ^= 1 << rng.below(8),
                1 => bytes.truncate(at.max(1)),
                _ => bytes.insert(at, rng.below(256) as u8),
            }
        }
        let outcome = guarded(|| {
            MergeInput::from_canonical_bytes(&bytes)
                .ok()
                .map(|decoded| (merge(&decoded).ok(), decoded))
        });
        match outcome {
            Err(panic) => failures.push(record_failure("decode", seed, &input, &panic)),
            Ok(None) => rejected += 1,
            Ok(Some((result, decoded))) => {
                accepted += 1;
                if let Some(problem) = result.as_ref().and_then(|r| judge(&decoded, r)) {
                    failures.push(record_failure("decode", seed, &decoded, &problem));
                }
            }
        }
    }
    eprintln!("decoder fuzz: 수용 {accepted}, 거부 {rejected}");
    assert!(failures.is_empty(), "{failures:#?}");
    assert!(rejected > 0, "변형 입력 일부는 거부되어야 합니다");
}

#[test]
fn campaign_is_deterministic() {
    let run = || {
        (0..20)
            .map(|seed| {
                let input = random_input(seed);
                merge(&input).map(|r| r.to_canonical_bytes()).ok()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
}
