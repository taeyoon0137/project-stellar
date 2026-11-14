//! document loader/validator fuzz harness (XVI.6.3, 테스트 전용).
//!
//! XVI.6.1·XVI.6.2와 같은 방식(고정 seed, `#[test]`, 새 의존성·nightly 없음)의 결정론적 fuzz입니다. 두 갈래입니다:
//!
//! 1. **byte 입력**: 유효 문서(VI.10.5 random valid document)의 subtree·node·정의 bytes와 컴포넌트 resource bytes
//!    (IX.9.1 library·contract·drilling, IX.9.3 View variant model)를 SplitMix64로 변형해 각 decoder에 넣습니다.
//!    수용한 subtree는 레코드로 묶어 validator(VI.10.1)·loader(VI.10.3)까지 통과시킵니다.
//! 2. **레코드 변형**: 유효 문서 레코드를 의미 단위로 깨뜨립니다 — 부모를 자기 자신·다른 node(cycle)·없는 id·NIL로
//!    바꾸기, 2-cycle, node 삭제(dangling 부모), id 중복·NIL, 형제 OrderKey 중복, View root를 없는 id·비 Frame으로,
//!    64단계 상한 경계·초과 깊이 chain(최대 5,000단계, 2만 단계는 고정 테스트) 추가, modifier stack 소유자 바꾸기·중복, 자식 있는 node를 Text로
//!    바꾸기, 레코드 순서 섞기.
//!
//! 판정(입력마다): **panic 0**(`catch_unwind`), hang 없음(worker thread + watchdog), 그리고 불변식:
//!
//! - validator가 통과시킨 레코드는 loader가 반드시 모델을 만듭니다(`LoadModelError::Model`은 버그 신호).
//! - 만든 모델의 `to_records`는 다시 validator를 통과하고 canonical 정렬한 입력과 같으며, View subtree 분할이
//!   성공합니다.
//! - 수용한 bytes는 다시 인코딩·디코딩해도 같은 값입니다(컴포넌트 resource는 canonical decoder라 bytes도 같음).
//!
//! [`FuzzStats`]는 대상별 수용·거부 횟수를 셉니다(모든 경로가 실행됐는지 확인용).

use std::collections::BTreeSet;

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;

use super::component_fixtures::component_library_fixture;
use super::random_document::{random_valid_document, SplitMix64};
use super::variant_fixtures::checkout_fixture;
use crate::component::definition::ComponentLibrary;
use crate::component::drilling::DrillingSet;
use crate::control::ControlContractTable;
use crate::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use crate::frame::FramePayload;
use crate::model::{DocumentModel, LoadModelError};
use crate::record_codec::{decode_node_record, encode_node_record, SubtreeRecords};
use crate::records::{DocumentRecords, NodePayload, NodeRecord};
use crate::text::TextPayload;
use crate::validate::{validate_document, MAX_NODE_DEPTH};
use crate::view_variant::commands::ViewVariantModel;

/// byte 입력 상한(변형 후 절단).
pub const MAX_INPUT_BYTES: usize = 64 * 1024;

/// byte 입력 대상.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ByteTarget {
    /// `DocumentDefinition::decode`.
    Definition,
    /// `decode_node_record`.
    NodeRecord,
    /// `SubtreeRecords::decode` → validator → loader.
    Subtree,
    /// `ComponentLibrary::decode`.
    ComponentLibrary,
    /// `DrillingSet::decode`.
    Drilling,
    /// `ControlContractTable::decode`.
    Contracts,
    /// `ViewVariantModel::decode`.
    VariantModel,
}

/// 모든 byte 대상.
pub const BYTE_TARGETS: [ByteTarget; 7] = [
    ByteTarget::Definition,
    ByteTarget::NodeRecord,
    ByteTarget::Subtree,
    ByteTarget::ComponentLibrary,
    ByteTarget::Drilling,
    ByteTarget::Contracts,
    ByteTarget::VariantModel,
];

/// 대상별 수용·거부 횟수.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FuzzStats {
    /// (대상, 수용 여부) → 횟수.
    pub bytes: std::collections::BTreeMap<(ByteTarget, bool), u64>,
    /// 레코드 변형 입력 중 validator 통과 수.
    pub records_valid: u64,
    /// 레코드 변형 입력 중 validator 거부 수.
    pub records_invalid: u64,
    /// 거부 위반 종류 이름(진단 확인용).
    pub violations: BTreeSet<String>,
}

impl FuzzStats {
    fn count(&mut self, target: ByteTarget, accepted: bool) {
        *self.bytes.entry((target, accepted)).or_insert(0) += 1;
    }
}

/// fuzz 입력 하나.
#[derive(Debug, Clone)]
pub enum FuzzInput {
    /// byte 입력.
    Bytes {
        /// 대상.
        target: ByteTarget,
        /// bytes.
        bytes: Vec<u8>,
    },
    /// 레코드 변형 입력.
    Records(DocumentRecords),
}

/// 레코드를 validator·loader에 넣고 불변식을 확인합니다. 위반은 설명 문자열입니다.
pub fn check_records(records: &DocumentRecords, stats: &mut FuzzStats) -> Result<(), String> {
    let validation = validate_document(records);
    let loaded = DocumentModel::from_records(records.clone());
    match (validation, loaded) {
        (Err(error), Err(LoadModelError::Invalid(_))) => {
            stats.records_invalid += 1;
            for violation in &error.violations {
                let name = format!("{violation:?}");
                let name = name.split([' ', '{', '(']).next().unwrap_or("").to_string();
                stats.violations.insert(name);
            }
            if error.violations.is_empty() {
                return Err("validator가 위반 0개 오류를 반환".into());
            }
            Ok(())
        }
        (Err(_), Ok(_)) => Err("validator가 거부한 레코드를 loader가 수용".into()),
        (_, Err(LoadModelError::Model(error))) => Err(format!(
            "validator가 통과시킨 레코드를 loader가 거부(버그 신호): {error:?}"
        )),
        (Ok(_), Err(LoadModelError::Invalid(_))) => {
            Err("validator 판정과 loader 판정이 다름".into())
        }
        (Ok(_), Ok((model, _))) => {
            stats.records_valid += 1;
            let mut out = model
                .to_records()
                .map_err(|e| format!("to_records 실패: {e:?}"))?;
            validate_document(&out).map_err(|e| format!("재검증 실패: {e:?}"))?;
            let mut expected = records.clone();
            expected.sort_canonical();
            out.sort_canonical();
            if out != expected {
                return Err("load → to_records 결과가 입력과 다름".into());
            }
            let subtrees = out
                .view_subtrees()
                .map_err(|e| format!("View subtree 분할 실패: {e:?}"))?;
            let rebuilt = DocumentRecords::from_subtrees(
                out.definition.clone(),
                subtrees.into_iter().map(|(_, s)| s).collect(),
            );
            if rebuilt != out {
                return Err("subtree 분할·재결합 결과가 다름".into());
            }
            Ok(())
        }
    }
}

fn single_view_records(subtree: SubtreeRecords) -> DocumentRecords {
    let mut definition = DocumentDefinition::new(StableId::new_v7(1, 0x0f16, 1), String::new());
    let mut page = PageDefinition::new(StableId::new_v7(2, 0x0f16, 2), String::new());
    let _ = page.add_view(ViewDefinition::new(
        StableId::new_v7(3, 0x0f16, 3),
        String::new(),
        subtree.root,
    ));
    let _ = definition.add_page(page);
    DocumentRecords::from_subtrees(definition, vec![subtree])
}

/// byte 입력 하나를 대상 decoder(와 수용 시 후속 단계)에 넣습니다.
pub fn check_bytes(target: ByteTarget, bytes: &[u8], stats: &mut FuzzStats) -> Result<(), String> {
    match target {
        ByteTarget::Definition => match DocumentDefinition::decode(bytes) {
            Ok(definition) => {
                stats.count(target, true);
                let again = DocumentDefinition::decode(&definition.encode())
                    .map_err(|e| format!("정의 재디코딩 실패: {e:?}"))?;
                if again != definition {
                    return Err("정의 round-trip 불일치".into());
                }
            }
            Err(_) => stats.count(target, false),
        },
        ByteTarget::NodeRecord => match decode_node_record(bytes) {
            Ok(record) => {
                stats.count(target, true);
                let again = decode_node_record(&encode_node_record(&record))
                    .map_err(|e| format!("node 레코드 재디코딩 실패: {e:?}"))?;
                if again != record {
                    return Err("node 레코드 round-trip 불일치".into());
                }
            }
            Err(_) => stats.count(target, false),
        },
        ByteTarget::Subtree => match SubtreeRecords::decode(bytes) {
            Ok(subtree) => {
                stats.count(target, true);
                let again = SubtreeRecords::decode(&subtree.encode())
                    .map_err(|e| format!("subtree 재디코딩 실패: {e:?}"))?;
                if again != subtree {
                    return Err("subtree round-trip 불일치".into());
                }
                check_records(&single_view_records(subtree), stats)?;
            }
            Err(_) => stats.count(target, false),
        },
        ByteTarget::ComponentLibrary => match ComponentLibrary::decode(bytes) {
            Ok(library) => {
                stats.count(target, true);
                if library.encode() != bytes {
                    return Err("component library가 canonical bytes가 아닌데 수용".into());
                }
            }
            Err(_) => stats.count(target, false),
        },
        ByteTarget::Drilling => match DrillingSet::decode(bytes) {
            Ok(set) => {
                stats.count(target, true);
                if set.encode() != bytes {
                    return Err("drilling 모음이 canonical bytes가 아닌데 수용".into());
                }
            }
            Err(_) => stats.count(target, false),
        },
        ByteTarget::Contracts => match ControlContractTable::decode(bytes) {
            Ok(table) => {
                stats.count(target, true);
                let again = ControlContractTable::decode(&table.encode())
                    .map_err(|e| format!("contract 재디코딩 실패: {e:?}"))?;
                if again != table {
                    return Err("contract round-trip 불일치".into());
                }
            }
            Err(_) => stats.count(target, false),
        },
        ByteTarget::VariantModel => match ViewVariantModel::decode(bytes) {
            Ok(model) => {
                stats.count(target, true);
                let again = ViewVariantModel::decode(&model.encode())
                    .map_err(|e| format!("variant model 재디코딩 실패: {e:?}"))?;
                if again != model {
                    return Err("variant model round-trip 불일치".into());
                }
            }
            Err(_) => stats.count(target, false),
        },
    }
    Ok(())
}

/// 입력 하나를 실행합니다.
pub fn check_input(input: &FuzzInput, stats: &mut FuzzStats) -> Result<(), String> {
    match input {
        FuzzInput::Bytes { target, bytes } => check_bytes(*target, bytes, stats),
        FuzzInput::Records(records) => check_records(records, stats),
    }
}

/// 변형 원본 묶음.
#[derive(Debug, Clone)]
pub struct Seeds {
    /// 유효 문서 레코드.
    pub documents: Vec<DocumentRecords>,
    /// 대상별 유효 bytes.
    pub bytes: Vec<(ByteTarget, Vec<u8>)>,
}

/// 유효 원본: random valid document `documents`개(+ 그 subtree·node·정의 bytes)와 컴포넌트 resource bytes.
pub fn seeds(documents: u64) -> Result<Seeds, String> {
    let mut out = Seeds {
        documents: Vec::new(),
        bytes: Vec::new(),
    };
    for seed in 0..documents {
        let records = random_valid_document(seed)
            .to_records()
            .map_err(|e| format!("seed {seed}: {e:?}"))?;
        out.bytes
            .push((ByteTarget::Definition, records.definition.encode()));
        for node in records.nodes.iter().take(4) {
            out.bytes
                .push((ByteTarget::NodeRecord, encode_node_record(node)));
        }
        let subtrees = records
            .view_subtrees()
            .map_err(|e| format!("seed {seed}: {e:?}"))?;
        for (_, subtree) in subtrees.into_iter().take(2) {
            out.bytes.push((ByteTarget::Subtree, subtree.encode()));
        }
        out.documents.push(records);
    }
    let components = component_library_fixture()?;
    out.bytes
        .push((ByteTarget::ComponentLibrary, components.library.encode()));
    out.bytes
        .push((ByteTarget::Drilling, components.drilling.encode()));
    for table in components.contracts.values() {
        out.bytes.push((ByteTarget::Contracts, table.encode()));
    }
    out.bytes
        .push((ByteTarget::VariantModel, checkout_fixture().model.encode()));
    Ok(out)
}

const TOKENS: [&[u8]; 8] = [
    &[0xFF, 0xFF, 0xFF, 0xFF],
    &[0x00, 0x00, 0x00, 0x00],
    &[0x01, 0x00, 0x00, 0x00],
    &[0x00, 0x00, 0xC0, 0x7F],
    &[0x00, 0x00, 0x80, 0x7F],
    &[0x00, 0x00, 0x00, 0x80],
    &[0xC3, 0x28],
    &[0x40, 0x00, 0x00, 0x00],
];

/// bytes를 1~8회 변형합니다(결정론적).
pub fn mutate_bytes(rng: &mut SplitMix64, source: &[u8], other: &[u8]) -> Vec<u8> {
    let mut bytes = source.to_vec();
    let rounds = 1 + rng.below(8);
    for _ in 0..rounds {
        let len = bytes.len() as u64;
        match rng.below(10) {
            0 if len > 0 => {
                let at = rng.below(len) as usize;
                bytes[at] ^= 1 << rng.below(8);
            }
            1 => {
                let at = rng.below(len + 1) as usize;
                bytes.insert(at, rng.next_u64() as u8);
            }
            2 if len > 0 => {
                let start = rng.below(len) as usize;
                let end = (start + 1 + rng.below(16) as usize).min(bytes.len());
                bytes.drain(start..end);
            }
            3 if len >= 4 => {
                let at = rng.below(len - 3) as usize;
                let token = *rng.pick(&TOKENS);
                for (i, byte) in token.iter().enumerate() {
                    if at + i < bytes.len() {
                        bytes[at + i] = *byte;
                    }
                }
            }
            4 => {
                let at = rng.below(len + 1) as usize;
                let token = *rng.pick(&TOKENS);
                for (i, byte) in token.iter().enumerate() {
                    bytes.insert(at + i, *byte);
                }
            }
            5 if len > 0 => {
                let start = rng.below(len) as usize;
                let end = (start + 1 + rng.below(32) as usize).min(bytes.len());
                let copy = bytes[start..end].to_vec();
                let at = rng.below(bytes.len() as u64 + 1) as usize;
                bytes.splice(at..at, copy);
            }
            6 if !other.is_empty() => {
                let cut = rng.below(len + 1) as usize;
                let from = rng.below(other.len() as u64) as usize;
                bytes.truncate(cut);
                bytes.extend_from_slice(&other[from..]);
            }
            7 => {
                let cut = rng.below(len + 1) as usize;
                bytes.truncate(cut);
            }
            8 if len > 0 => {
                let at = rng.below(len) as usize;
                bytes[at] = *rng.pick(&[0x00, 0x01, 0x02, 0x7F, 0x80, 0xFF]);
            }
            _ if len >= 8 => {
                let a = rng.below(len - 4) as usize;
                let b = rng.below(len - 4) as usize;
                for i in 0..4 {
                    bytes.swap(a + i, b + i);
                }
            }
            _ => {}
        }
        bytes.truncate(MAX_INPUT_BYTES);
    }
    bytes
}

fn random_id(rng: &mut SplitMix64) -> StableId {
    StableId::new_v7(
        rng.below(1 << 40),
        0x0bad,
        rng.next_u64() & 0xFFFF_FFFF_FFFF,
    )
}

fn frame_record(id: StableId, parent: StableId, key: u8) -> Option<NodeRecord> {
    Some(NodeRecord {
        id,
        parent: Some(parent),
        order_key: OrderKey::from_bytes(vec![key.max(1)]).ok()?,
        visible: true,
        locked: false,
        display_name: String::new(),
        payload: NodePayload::Frame(FramePayload::default()),
    })
}

/// 의미 단위 변형으로 invalid ref·cycle·깊이·중복 문서를 만듭니다(1~3회).
pub fn mutate_records(rng: &mut SplitMix64, source: &DocumentRecords) -> DocumentRecords {
    let mut records = source.clone();
    let rounds = 1 + rng.below(3);
    for _ in 0..rounds {
        let n = records.nodes.len() as u64;
        if n == 0 {
            break;
        }
        let a = rng.below(n) as usize;
        let b = rng.below(n) as usize;
        match rng.below(14) {
            0 => records.nodes[a].parent = Some(records.nodes[a].id),
            1 => {
                let id = records.nodes[b].id;
                records.nodes[a].parent = Some(id);
            }
            2 => {
                let (ia, ib) = (records.nodes[a].id, records.nodes[b].id);
                records.nodes[a].parent = Some(ib);
                records.nodes[b].parent = Some(ia);
            }
            3 => records.nodes[a].parent = Some(random_id(rng)),
            4 => records.nodes[a].parent = None,
            5 => {
                records.nodes.remove(a);
            }
            6 => {
                let copy = records.nodes[a].clone();
                records.nodes.push(copy);
            }
            7 => records.nodes[a].id = StableId::NIL,
            8 => {
                let key = records.nodes[b].order_key.clone();
                records.nodes[a].order_key = key;
            }
            9 => {
                let target = if rng.one_in(2) {
                    random_id(rng)
                } else {
                    records.nodes[a].id
                };
                if let Some(view) = records
                    .definition
                    .pages
                    .first_mut()
                    .and_then(|p| p.views.first_mut())
                {
                    view.root_frame = target;
                }
            }
            10 => {
                // 64단계 상한 경계와 초과 깊이 chain.
                let depth = *rng.pick(&[
                    MAX_NODE_DEPTH as usize - 1,
                    MAX_NODE_DEPTH as usize,
                    MAX_NODE_DEPTH as usize + 1,
                    200,
                    5_000,
                ]);
                let mut parent = records.nodes[a].id;
                let base = rng.next_u64() & 0xFFFF_FFFF;
                for i in 0..depth {
                    let id = StableId::new_v7(
                        base + i as u64,
                        0x0dee,
                        (base ^ i as u64) & 0xFFFF_FFFF_FFFF,
                    );
                    if let Some(record) = frame_record(id, parent, 0x80) {
                        records.nodes.push(record);
                    }
                    parent = id;
                }
            }
            11 if !records.modifier_stacks.is_empty() => {
                let s = rng.below(records.modifier_stacks.len() as u64) as usize;
                let owner = if rng.one_in(3) {
                    random_id(rng)
                } else {
                    records.nodes[b].id
                };
                records.modifier_stacks[s].node = owner;
                if rng.one_in(4) {
                    let copy = records.modifier_stacks[s].clone();
                    records.modifier_stacks.push(copy);
                }
            }
            12 => {
                records.nodes[a].payload = NodePayload::Text(TextPayload::empty(0));
            }
            _ => {
                // 레코드 순서 섞기(의미 불변).
                let len = records.nodes.len();
                for i in (1..len).rev() {
                    let j = rng.below(i as u64 + 1) as usize;
                    records.nodes.swap(i, j);
                }
            }
        }
    }
    records
}

/// `seed` campaign 입력(결정론적): 변형하지 않은 원본 전체(수용 경로 보장) 뒤에 변형 입력 `count`개.
pub fn campaign(seeds: &Seeds, seed: u64, count: usize) -> Vec<FuzzInput> {
    let mut rng = SplitMix64::new(seed);
    let mut out: Vec<FuzzInput> = seeds
        .bytes
        .iter()
        .map(|(target, bytes)| FuzzInput::Bytes {
            target: *target,
            bytes: bytes.clone(),
        })
        .chain(seeds.documents.iter().cloned().map(FuzzInput::Records))
        .collect();
    out.reserve(count);
    for _ in 0..count {
        if rng.one_in(3) && !seeds.documents.is_empty() {
            let source = rng.pick(&seeds.documents);
            out.push(FuzzInput::Records(mutate_records(&mut rng, source)));
        } else if !seeds.bytes.is_empty() {
            let (target, source) = rng.pick(&seeds.bytes).clone();
            let (_, other) = rng.pick(&seeds.bytes);
            let bytes = mutate_bytes(&mut rng, &source, other);
            // 같은 bytes를 다른 decoder에도 넣어 형식 혼동 입력을 만듭니다.
            let target = if rng.one_in(5) {
                *rng.pick(&BYTE_TARGETS)
            } else {
                target
            };
            out.push(FuzzInput::Bytes { target, bytes });
        }
    }
    out
}

/// 입력 설명(발견 보고용, 앞 64 bytes hex).
pub fn describe(input: &FuzzInput) -> String {
    match input {
        FuzzInput::Bytes { target, bytes } => {
            let hex: String = bytes.iter().take(64).map(|b| format!("{b:02x}")).collect();
            format!("{target:?} {} bytes: {hex}", bytes.len())
        }
        FuzzInput::Records(records) => format!(
            "records nodes={} stacks={}",
            records.nodes.len(),
            records.modifier_stacks.len()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::DocumentViolation;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    const PER_INPUT_LIMIT: Duration = Duration::from_secs(3);
    const NO_PROGRESS_LIMIT: Duration = Duration::from_secs(60);

    fn iterations() -> usize {
        std::env::var("STELLAR_DOCUMENT_FUZZ_ITERATIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1_500)
    }

    /// campaign을 worker thread에서 실행하고 panic·불변식 위반·느린 입력을 모읍니다(watchdog 포함).
    fn run_campaign(seed: u64, count: usize) -> (FuzzStats, Vec<String>) {
        let (progress, watch) = mpsc::channel::<usize>();
        let worker = std::thread::Builder::new()
            .name("document-loader-fuzz".into())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let seeds = seeds(12).expect("seed 원본");
                let inputs = campaign(&seeds, seed, count);
                let mut stats = FuzzStats::default();
                let mut findings = Vec::new();
                for (index, input) in inputs.iter().enumerate() {
                    let started = Instant::now();
                    let outcome = catch_unwind(AssertUnwindSafe(|| check_input(input, &mut stats)));
                    match outcome {
                        Ok(Ok(())) => {}
                        Ok(Err(violation)) => findings.push(format!(
                            "#{index} 불변식: {violation} — {}",
                            describe(input)
                        )),
                        Err(_) => findings.push(format!("#{index} panic — {}", describe(input))),
                    }
                    if started.elapsed() > PER_INPUT_LIMIT {
                        findings.push(format!(
                            "#{index} 느린 입력 {:?} — {}",
                            started.elapsed(),
                            describe(input)
                        ));
                    }
                    let _ = progress.send(index);
                }
                (stats, findings)
            })
            .expect("fuzz worker");
        let mut last = None;
        loop {
            match watch.recv_timeout(NO_PROGRESS_LIMIT) {
                Ok(index) => last = Some(index),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    panic!("hang: {NO_PROGRESS_LIMIT:?} 동안 진행 없음(마지막 완료 입력 {last:?})")
                }
            }
        }
        worker.join().expect("fuzz worker join")
    }

    #[test]
    fn loader_and_validator_fuzz_campaign_has_zero_panics() {
        // 검증 기준(XVI.6.3): invalid refs/cycles 처리, panic 0.
        let seed = std::env::var("STELLAR_DOCUMENT_FUZZ_SEED")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0x5EED_D0C5);
        let (stats, findings) = run_campaign(seed, iterations());
        assert!(
            findings.is_empty(),
            "fuzz 발견 {}건:\n{}",
            findings.len(),
            findings.join("\n")
        );
        // 모든 byte 대상이 수용·거부 경로를 한 번 이상 실행했습니다.
        for target in BYTE_TARGETS {
            assert!(
                stats.bytes.get(&(target, false)).copied().unwrap_or(0) > 0,
                "{target:?} 거부 경로 미실행"
            );
            assert!(
                stats.bytes.get(&(target, true)).copied().unwrap_or(0) > 0,
                "{target:?} 수용 경로 미실행: {stats:?}"
            );
        }
        assert!(
            stats.records_valid > 0 && stats.records_invalid > 0,
            "{stats:?}"
        );
        // invalid ref·cycle·깊이·중복 위반을 실제로 만들어 거부했습니다.
        for name in [
            "ParentCycle",
            "MissingParent",
            "DepthExceeded",
            "DuplicateId",
        ] {
            assert!(
                stats.violations.contains(name),
                "{name} 미발생: {:?}",
                stats.violations
            );
        }
    }

    #[test]
    fn campaign_inputs_are_deterministic() {
        let seeds = seeds(4).unwrap();
        let a: Vec<String> = campaign(&seeds, 7, 200).iter().map(describe).collect();
        let b: Vec<String> = campaign(&seeds, 7, 200).iter().map(describe).collect();
        assert_eq!(a, b);
        let c: Vec<String> = campaign(&seeds, 8, 200).iter().map(describe).collect();
        assert_ne!(a, c);
    }

    fn base_records() -> DocumentRecords {
        seeds(1).unwrap().documents.remove(0)
    }

    fn violations(records: &DocumentRecords) -> Vec<DocumentViolation> {
        let mut stats = FuzzStats::default();
        check_records(records, &mut stats).unwrap();
        validate_document(records)
            .err()
            .map(|e| e.violations)
            .unwrap_or_default()
    }

    #[test]
    fn handcrafted_invalid_refs_and_cycles_are_rejected_without_panic() {
        let base = base_records();
        assert!(violations(&base).is_empty(), "원본은 유효");
        let child = base
            .nodes
            .iter()
            .position(|n| n.parent.is_some())
            .expect("자식 node");
        let other = base
            .nodes
            .iter()
            .rposition(|n| n.parent.is_some())
            .expect("자식 node");
        let has = |records: &DocumentRecords, pred: &dyn Fn(&DocumentViolation) -> bool| {
            violations(records).iter().any(pred)
        };

        // 자기 부모(self cycle).
        let mut records = base.clone();
        records.nodes[child].parent = Some(records.nodes[child].id);
        assert!(has(&records, &|v| matches!(
            v,
            DocumentViolation::ParentCycle { .. }
        )));
        // 2-cycle.
        if child != other {
            let mut records = base.clone();
            let (a, b) = (records.nodes[child].id, records.nodes[other].id);
            records.nodes[child].parent = Some(b);
            records.nodes[other].parent = Some(a);
            assert!(!violations(&records).is_empty());
        }
        // 없는 부모(dangling ref).
        let mut records = base.clone();
        records.nodes[child].parent = Some(StableId::new_v7(9, 0x0bad, 9));
        assert!(has(&records, &|v| matches!(
            v,
            DocumentViolation::MissingParent { .. }
        )));
        // View root가 없는 id.
        let mut records = base.clone();
        records.definition.pages[0].views[0].root_frame = StableId::new_v7(8, 0x0bad, 8);
        assert!(has(&records, &|v| matches!(
            v,
            DocumentViolation::MissingRootFrame { .. }
        )));
        // 깊이 상한: 64단계는 통과, 초과는 DepthExceeded. 2만 단계도 panic·stack overflow 없이 처리.
        let root = base.definition.pages[0].views[0].root_frame;
        for (depth, ok) in [
            (MAX_NODE_DEPTH as usize, true),
            (MAX_NODE_DEPTH as usize + 1, false),
            (20_000, false),
        ] {
            let mut records = base.clone();
            let mut parent = root;
            for i in 0..depth {
                let id = StableId::new_v7(50_000 + i as u64, 0x0dee, 50_000 + i as u64);
                records.nodes.push(frame_record(id, parent, 0x01).unwrap());
                parent = id;
            }
            let found = violations(&records);
            assert_eq!(
                found
                    .iter()
                    .any(|v| matches!(v, DocumentViolation::DepthExceeded { .. })),
                !ok,
                "depth {depth}"
            );
        }
        // modifier stack 소유자가 없는 node.
        if !base.modifier_stacks.is_empty() {
            let mut records = base.clone();
            records.modifier_stacks[0].node = StableId::new_v7(7, 0x0bad, 7);
            assert!(has(&records, &|v| matches!(
                v,
                DocumentViolation::ModifierOwnerMissing { .. }
            )));
        }
        // id 중복.
        let mut records = base.clone();
        let copy = records.nodes[child].clone();
        records.nodes.push(copy);
        assert!(has(&records, &|v| matches!(
            v,
            DocumentViolation::DuplicateId { .. }
        )));
    }
}
