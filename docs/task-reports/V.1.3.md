# 작업 보고서: V.1.3 canonical decoder

- WBS: V.1.3
- 작업명: canonical decoder — length overflow, duplicate key, unknown mandatory tag 거부
- 상태: 완료 (`[x]`)
- 선행: V.1.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/canonical/decode.rs`(신규) — `CanonicalDecoder`, `DecodeError`.
- `crates/storage/src/canonical/encode.rs` — tagged field 형식(`Field`, `put_fields`,
  `EncodeError::DuplicateTag`) 추가.
- `crates/storage/src/canonical/mod.rs` — `decode` 모듈 등록, `MapEntry` 타입 별칭.
- `docs/task-reports/V.1.3.md`(신규), 계획서(V.1.3 `[ ]`→`[x]`).

## 구현 요약

- **`CanonicalDecoder`**: 입력 slice 위 cursor로 동작하는 decoder입니다. 모든 실패는 panic
  없이 `DecodeError`로 반환합니다(인덱스 초과·정수 overflow 경로 없음).
- **length overflow 거부**: 길이 프리픽스가 남은 입력보다 크면 `LengthOverflow`로 거부합니다.
  고정 폭 정수 절단은 `UnexpectedEof`.
- **duplicate key 거부**: `take_map`은 key가 canonical byte 순서로 **엄격히 증가**해야 하며,
  중복은 `DuplicateKey`, 역순은 `MisorderedKey`로 거부합니다(R4).
- **unknown mandatory tag 거부(R6)**: tagged field wire 형식을 확정했습니다 —
  `u32` LE tag + `u8` flags(bit0=mandatory) + 길이 프리픽스 payload, 레코드는 `u32` count +
  tag 오름차순. `take_fields(known_tags)`는 모르는 mandatory tag를 `UnknownMandatoryTag`로
  거부하고 모르는 optional tag는 skip합니다. encoder에도 같은 형식의 `put_fields`(중복 tag
  거부)를 추가했습니다.
- **추가 canonical 검증**: bool 0x00/0x01 외 거부, 비UTF-8 문자열 거부, 정의되지 않은
  flags bit 거부, `expect_end`로 trailing bytes 거부.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 18 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| malformed corpus 16종 전부 panic 없이 오류 반환(`malformed_corpus_returns_errors_without_panic`) | 통과 |
| primitive·map·field round-trip | 통과 |
| unknown mandatory tag 거부, unknown optional skip | 통과 |

## 차이(작업 지시와 다른 점)

- R6 검증을 위해 tagged field wire 형식(encoder `put_fields`)을 이 작업에서 함께 확정했습니다.
  V.1.2 시점에는 레코드 형식이 없었고 decoder의 unknown mandatory tag 거부에 필요합니다.

## 남은 위험 / 한계

- `take_f32`/`take_f64`는 아직 비유한 값을 거부하지 않습니다. V.1.4에서 encoder와 함께 core
  규칙(III.3.1)과 일치시킵니다.
- malformed corpus는 수작업 16종입니다. fuzzing 기반 corpus 확장은 후속 phase(저장 fault
  matrix, V.5.6)에서 다룹니다.
