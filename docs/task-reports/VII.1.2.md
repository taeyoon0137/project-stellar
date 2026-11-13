# 작업 보고서: VII.1.2 직렬화 가능한 closed enum command와 validate·apply·invert

- WBS: VII.1.2
- 작업명: `Command` trait 대신 직렬화 가능한 closed enum, `validate`·`apply`·`invert` 인터페이스
- 상태: 완료 (`[x]`)
- 선행: VII.1.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/kind.rs`(신규) — `CommandKind` numeric registry(`ALL`, `RESERVED = 0x0000`, `from_u16`).
- `crates/commands/src/command.rs`(신규) — `Command` closed enum, `CommandError`, `validate`/`invert`/`apply`,
  테스트 전용 `TestProbe`와 공용 fixture.
- `crates/commands/src/edit.rs`(신규) — `EditContext`(apply가 쓰는 primitive 변경 창구).
- `crates/commands/src/codec.rs`(신규) — `encode_command`/`decode_command`, `CommandCodecError`, LE writer/reader.
- `crates/commands/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VII.1.2.md`(신규), 계획서(VII.1.2 `[ ]`→`[x]`).

## 구현 요약

- **closed enum**: `Command`는 trait 객체가 아닌 닫힌 enum이며 각 variant는 `CommandKind` 번호를 가집니다.
  registry 번호는 저장·journal 계약이라 오름차순·중복 없음·예약값(0) 미사용을 테스트로 고정합니다.
- **인터페이스**: `validate(&DocumentModel)`(무변경 검사), `invert(&DocumentModel)`(적용 전 문서 기준 역 command,
  무변경), `apply(&mut EditContext)`(primitive 변경만). `EditContext`는 `DocumentModel`의 공개 변경 API(삽입,
  leaf 삭제, payload 교체, 이름, modifier stack)와 1:1인 단일 변경 창구라, 이후 변경 기록(VII.1.4)·되돌리기
  journal(VII.1.5)을 한 곳에 붙입니다.
- **직렬화**: `kind u16 LE + payload 길이 u32 LE + payload`. 디코딩은 payload 해석 전에 registry에 없는 kind를
  `UnknownKind`로 거부하고, 절단·trailing bytes·잘못된 bool·과대 길이를 거부합니다(LE 고정 폭, V.1 규칙).
- **registry 초기 상태**: 프로덕션 registry는 비어 있고 각 문서 command 작업(VII.2.x)이 번호와 함께 등록합니다.
  command 계층 자체를 검증하기 위해 **테스트 빌드에만** `TestProbe`(`0xFFF0`, 이름 변경 후 선택적 실패 주입)를 둡니다.
  `CommandKind::ALL`에는 포함되지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-commands` | 완료 |
| `cargo test -p stellar-commands` | unit 11 passed, doctest 3 passed, 0 failed |
| `cargo clippy -p stellar-commands --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-commands -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| unknown command kind(0x0000·0x00FF·0x7777·0xFFFF, 빈 payload 포함) 거부(`unknown_command_kind_is_rejected`, `unknown_numbers_are_rejected`) | 통과 |
| registry 번호 오름차순·유일·예약값 미사용, 왕복(`registry_numbers_are_unique_sorted_and_not_reserved`) | 통과 |
| command canonical round-trip·재인코딩 동일(`command_round_trips`) | 통과 |
| 절단·trailing·과대 길이·bool 2 거부(`malformed_framing_is_rejected`) | 통과 |
| validate→invert→apply→inverse apply 원복, 없는 대상 거부(`validate_invert_apply_interface_round_trips` 외 1) | 통과 |

## 차이(작업 지시와 다른 점)

- 프로덕션 registry가 비어 있는 동안(VII.2.1 전) 프로덕션 빌드에서 `Command`는 값이 없는 타입이라, codec helper에
  `allow(dead_code)`와 `encode_command_into`에 `cfg_attr(not(test), allow(unreachable_code))`를 임시로 두었습니다.
  VII.2.1에서 첫 프로덕션 variant가 등록되면 정리합니다.

## 남은 위험 / 한계

- 실제 문서 command variant는 VII.2.x에서 추가됩니다. 그 전까지 command 계층 검증은 테스트 전용 probe에 의존합니다.
