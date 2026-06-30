# 작업 보고서: V.1.1 저장 canonical primitive 규칙·golden fixture

- WBS: V.1.1
- 작업명: 저장 primitive canonical 규칙 문서 + 구현 전 golden bytes 정의·스키마 검사
- 상태: 완료 (`[x]`)
- 선행: Phase III

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `docs/architecture/storage-canonical-primitives.md`(신규) — canonical 규칙 R1~R6 명세(구현 이전 문서).
- `docs/fixtures/storage/canonical-primitives.yaml`(신규) — primitive별 golden bytes(type/value/bytes), 22개 항목.
- `tools/xtask/src/storage_fixtures.rs`(신규) — fixture 스키마 validator `check_canonical_primitives`.
- `tools/xtask/src/lib.rs` — `pub mod storage_fixtures;` 등록.
- `tools/xtask/src/commands.rs` — `validate-storage-fixtures` 명령 등록(COMMANDS, 디스패치, `cmd_validate_storage_fixtures`).
- `docs/task-reports/V.1.1.md`(신규), 계획서(V.1.1 `[ ]`→`[x]`).

## 구현 요약

- **규칙 문서**: little-endian(R1), 고정 정수 폭(R2), 길이 프리픽스 UTF-8(R3), 정렬된 map key(R4),
  유한 float bits(R5), unknown field 정책(R6)을 구현 이전 명세로 작성했습니다. 인코딩 구현(V.1.2)·
  decoder(V.1.3)·float 정책(V.1.4)이 따를 1차 기준입니다.
- **golden bytes fixture**: `u8`~`i64`, `bool`, `f32`/`f64`, `string`(빈 문자열, ASCII, 멀티바이트
  `é`)의 기대 바이트를 소문자 16진수 little-endian으로 정의했습니다. 바이트 값은 scratchpad Rust로
  검산했습니다. 예: `u16_258 → 0201`, `i32_neg_256 → 00ffffff`, `string_hi → 020000006869`.
- **스키마 validator**: validator는 fixture의 **구조**만 검사합니다(최상위 `primitives` 시퀀스, 각
  항목의 `name`/`type`/`value`/`bytes`, type 허용 집합 12종, bytes는 소문자·짝수 길이 16진수,
  알 수 없는 키·중복 name 거부). golden bytes의 **값 정확성**은 검사하지 않습니다(encoder, V.1.2/
  V.1.4 소관). `cargo xtask validate-storage-fixtures`로 실행합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p xtask` | 109 passed, 0 failed |
| `cargo run -p xtask -- validate-storage-fixtures` | exit 0 (스키마 검사 통과) |
| `cargo fmt -p xtask -- --check` | 통과 |
| `cargo clippy -p xtask --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 실제 fixture 스키마 통과(`the_real_fixture_passes`) | 통과 |
| `missing-primitives`/`bad-type`/`bad-bytes`(홀수·대문자)/`missing-field`/`duplicate-name` 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 완료 기준(fixture 존재 + 스키마 검사 통과)을 충족합니다.

## 남은 위험 / 한계

- validator는 구조만 검증하고 golden bytes 값의 정확성은 검증하지 않습니다. 값 정확성은 V.1.2
  encoder가 동일 fixture와 byte-identical하게 일치하는지로 후속 검증합니다.
- 규칙 문서는 구현 이전 명세입니다. 인코딩 구현(V.1.2~V.1.4)과 불일치가 생기면 문서·fixture를
  source of truth로 두고 구현을 맞춥니다.
