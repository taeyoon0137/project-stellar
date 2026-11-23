//! **테스트 전용** command 실행 fuzz target (VII.6.3).
//!
//! 외부 fuzz 엔진 없이 쓸 수 있는 결정론적 harness입니다. 입력 bytes 하나([`fuzz_one`])로
//!
//! 1. 임의 bytes를 command·요청·transaction·선택 event로 디코딩하고(거부는 정상),
//! 2. random valid document(VI.10.5) 위에서 생성기(VII.6.1) command의 canonical bytes를 입력으로
//!    변조(bit flip·덮어쓰기·절단)해 디코딩되면 실행하고,
//! 3. 유효 command와 그 역을 섞어 실행합니다.
//!
//! 어떤 경로에서도 **panic·out-of-bounds가 없어야** 하며, 실행이 성공하면 문서는 전체 validator
//! (VI.10.1)를 통과해야 합니다(command가 문서 불변식을 깨면 panic으로 드러남). 거부(오류)는 정상입니다.
//! 장기 nightly 실행(XVI.6.7)이나 외부 fuzz 엔진 연결은 같은 함수를 호출하면 됩니다.
#![cfg(any(test, feature = "test-support"))]

use stellar_document::test_support::random_document::{random_valid_document, SplitMix64};
use stellar_document::validate::validate_document;

use crate::codec::{decode_command, encode_command};
use crate::command::Command;
use crate::kind::CommandKind;
use crate::request::CommandRequest;
use crate::selection::SetSelection;
use crate::state::DocumentState;
use crate::test_support::{generate, FreshIds};
use crate::transaction::CommandTransaction;

/// fuzz 실행 통계.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FuzzStats {
    /// 처리한 입력 수.
    pub inputs: usize,
    /// 변조 bytes가 디코딩된 횟수.
    pub decoded: usize,
    /// 실행이 성공한 command 수.
    pub executed: usize,
    /// 실행이 거부된 command 수.
    pub rejected: usize,
}

fn seed_of(input: &[u8]) -> u64 {
    let mut raw = [0u8; 8];
    for (slot, byte) in raw.iter_mut().zip(input) {
        *slot = *byte;
    }
    u64::from_le_bytes(raw)
}

fn mix(input: &[u8]) -> u64 {
    // FNV-1a: 입력 전체를 PRNG seed에 섞습니다.
    input.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// 실행하고, 성공하면 문서 불변식을 확인합니다.
fn execute_checked(
    state: &mut DocumentState,
    command: Command,
    stats: &mut FuzzStats,
) -> Option<Command> {
    let request = CommandRequest::at(state, command);
    match state.execute(&request) {
        Ok(outcome) => {
            stats.executed += 1;
            let records = state
                .model()
                .to_records()
                .unwrap_or_else(|e| panic!("실행 후 레코드 생성 실패: {e}"));
            if let Err(error) = validate_document(&records) {
                panic!("command {request:?}가 문서 불변식을 깨뜨림: {error}");
            }
            Some(outcome.inverse)
        }
        Err(_) => {
            stats.rejected += 1;
            None
        }
    }
}

/// 입력 하나를 처리합니다. panic하지 않아야 합니다.
pub fn fuzz_one(input: &[u8], stats: &mut FuzzStats) {
    stats.inputs += 1;
    // 1. 임의 bytes 디코딩.
    let _ = decode_command(input);
    let _ = CommandRequest::decode(input);
    let _ = CommandTransaction::decode(input);
    let _ = SetSelection::decode(input);
    for start in (0..input.len()).step_by(7) {
        let _ = decode_command(&input[start..]);
    }

    // 2·3. 문서 위 실행.
    let seed = seed_of(input) % 1_000;
    let mut state = DocumentState::new(random_valid_document(seed));
    let mut rng = SplitMix64::new(mix(input));
    let mut ids = FreshIds::new(mix(input) ^ 0xF0_22);
    let mut pending_inverse: Option<Command> = None;
    for (step, byte) in input.iter().enumerate().take(24) {
        match byte % 4 {
            // 생성 command를 변조해 디코딩되면 실행.
            0 | 1 => {
                let kind = *rng.pick(CommandKind::ALL);
                let Some(command) = generate(kind, state.model(), &mut rng, &mut ids) else {
                    continue;
                };
                let mut bytes = encode_command(&command);
                if bytes.is_empty() {
                    continue;
                }
                let at = (rng.below(bytes.len() as u64)) as usize;
                match byte % 3 {
                    0 => bytes[at] ^= 1 << (step % 8),
                    1 => bytes[at] = input[(step * 5) % input.len()],
                    _ => bytes.truncate(at),
                }
                if let Ok(decoded) = decode_command(&bytes) {
                    stats.decoded += 1;
                    if let Some(inverse) = execute_checked(&mut state, decoded, stats) {
                        pending_inverse = Some(inverse);
                    }
                }
            }
            // 유효 후보 실행.
            2 => {
                let kind = *rng.pick(CommandKind::ALL);
                if let Some(command) = generate(kind, state.model(), &mut rng, &mut ids) {
                    if let Some(inverse) = execute_checked(&mut state, command, stats) {
                        pending_inverse = Some(inverse);
                    }
                }
            }
            // 직전 성공 command의 역 실행.
            _ => {
                if let Some(inverse) = pending_inverse.take() {
                    execute_checked(&mut state, inverse, stats);
                }
            }
        }
    }
}

/// CI smoke에서 실행할 입력 수(VII.6.3 정해진 smoke 범위).
pub const SMOKE_INPUTS: u64 = 600;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_fuzz_command_execution_is_panic_free() {
        // 검증 기준(VII.6.3): 정해진 fuzz smoke 범위에서 panic·out-of-bounds 무오류.
        let mut corpus = SplitMix64::new(0xF022_5EED);
        let mut stats = FuzzStats::default();
        for _ in 0..SMOKE_INPUTS {
            let len = 1 + corpus.below(160) as usize;
            let input: Vec<u8> = (0..len).map(|_| corpus.next_u64() as u8).collect();
            fuzz_one(&input, &mut stats);
        }
        // 빈 입력·극단값도 처리합니다.
        fuzz_one(&[], &mut stats);
        fuzz_one(&[0xFF; 64], &mut stats);
        fuzz_one(&[0x00; 64], &mut stats);
        eprintln!("fuzz smoke 통계: {stats:?}");
        assert_eq!(stats.inputs as u64, SMOKE_INPUTS + 3);
        assert!(stats.executed > 3_000, "실행 {stats:?}");
        assert!(stats.decoded > 1_000, "변조 디코딩 {stats:?}");
        assert!(stats.rejected > 500, "거부 {stats:?}");
    }

    #[test]
    fn adr_0079_kinds_survive_mutation_fuzz() {
        // ADR-0079 종류를 종류별로 생성·변조해 decode·실행합니다. panic 없이, 성공한 실행 뒤 문서가 validator를
        // 통과해야 합니다(execute_checked). 인스턴스가 없는 random document에서는 swap-instance와 인스턴스 구조
        // command(detach-instance, place-slot-content)가 생성되지 않습니다.
        let kinds: Vec<CommandKind> = CommandKind::ALL
            .iter()
            .copied()
            .filter(|kind| kind.codec_version() >= 2)
            .collect();
        // ADR-0079 종류 10개, ADR-0086 종류 3개(codec version 3), ADR-0099 Context 편집 1개(codec version 5),
        // ADR-0126 가이드·View 변수 편집 2개(codec version 7).
        // ADR-0129 인스턴스 구조 2개(codec version 8).
        assert_eq!(kinds.len(), 16);
        // UnsetProperty는 명시 layout longhand가 있어야 생성되므로 SetLayoutProperty 뒤에 둡니다.
        let mut kinds = kinds;
        kinds.sort_by_key(|kind| *kind == CommandKind::UnsetProperty);
        let mut executed = std::collections::BTreeMap::new();
        let mut stats = FuzzStats::default();
        for seed in 0..80u64 {
            let mut state = DocumentState::new(random_valid_document(seed));
            let mut rng = SplitMix64::new(seed ^ 0xAD_0079);
            let mut ids = FreshIds::new(seed ^ 0x79);
            for kind in &kinds {
                let Some(command) = generate(*kind, state.model(), &mut rng, &mut ids) else {
                    continue;
                };
                let bytes = encode_command(&command);
                let before = stats.executed;
                for flip in 0..6 {
                    let mut mutated = bytes.clone();
                    let at = rng.below(mutated.len() as u64) as usize;
                    mutated[at] ^= 1 << flip;
                    if let Ok(decoded) = decode_command(&mutated) {
                        execute_checked(&mut state, decoded, &mut stats);
                    }
                    let _ = decode_command(&mutated[..at]);
                }
                execute_checked(&mut state, command, &mut stats);
                if stats.executed > before {
                    *executed.entry(*kind).or_insert(0usize) += 1;
                }
            }
        }
        for kind in &kinds {
            if !matches!(
                kind,
                CommandKind::SwapInstance
                    | CommandKind::DetachInstance
                    | CommandKind::PlaceSlotContent
            ) {
                assert!(
                    executed.get(kind).copied().unwrap_or(0) > 0,
                    "{kind} 실행 0건"
                );
            }
        }
    }
}
