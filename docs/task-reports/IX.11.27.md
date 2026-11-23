# 작업 보고서: IX.11.27 영속 command `SetStyleDefaults`·`UnsetProperty`

- WBS: IX.11.27
- 작업명: 영속 command `SetStyleDefaults`(활성 세트 변경)와 `UnsetProperty`(명시 값 지우기), persistent-commands 대역
  끝 번호와 command codec version 3(ADR-0082 결정 4·5, Compatibility)
- 상태: 부분 완료 (`[~]`) — command·codec·undo·property test·옛 codec journal·모르는 command 봉인은 확인했습니다. "세트
  변경 뒤 저장 diff가 한 줄"은 CLI `style-defaults set` 경로에서만 성립하고, `stellar apply`(`ops::apply`) 경로는
  generation 파일 두 개를 함께 바꿉니다(아래 "차이").
- 결정: ADR-0086 결정 10, ADR-0087 결정 2, ADR-0115 결정 4
- 선행: IX.11.25, IX.11.26, VII.1.6

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- 이 보고서만 추가했습니다. 코드 변경은 없습니다(판정에 쓴 CLI test는 IX.11.29 커밋에 있습니다).

## 확인한 코드 위치

- command: `crates/commands/src/style_defaults.rs`(`SetStyleDefaults` 0x0091, `UnsetProperty` 0x0092, 역 command
  `SetLayoutProperty` 0x0093). 번호·이름·codec version은 `crates/commands/src/kind.rs`.
- codec version: `crates/commands/src/codec.rs` `COMMAND_CODEC_VERSION`(3에서 두 종류가 들어왔고 지금 5).
- property test 등록: `crates/commands/src/property_tests.rs`(`set_style_defaults_round_trip`,
  `unset_property_round_trip`, `set_layout_property_round_trip`).
- journal: `crates/history/src/journal.rs`(header의 codec version, 옛 schema 1 journal decode, 새 codec 거부).
- 저장 한 줄: `crates/project/src/manifest_style_defaults.rs` `prepare_selection`·`commit`, CLI
  `tools/stellar-cli/src/style_defaults.rs` `set`.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-commands --lib style_defaults` | 통과(6). 번호·codec 왕복, set·unset의 undo 왕복, 세트 선택 undo 한 번, 실패 transaction 롤백 |
| `cargo test -p stellar-commands --lib property_tests::set_style` / `property_tests::unset` | 통과(각 1). apply → inverse → 원래 snapshot |
| `cargo test -p stellar-history` | 통과(55). schema 1 journal decode, 이 reader보다 새 codec 거부(`is_unknown_command`), 선언 codec 밖 종류 거부 |
| `cargo test -p stellar-cli --test style_defaults` | 통과(5). `set` 뒤 프로젝트에서 바뀐 파일은 `stellar.yaml` 하나, 더해진 줄은 `style_defaults: react-native` 하나, 되돌리면 원래 bytes |
| `stellar apply`에 canonical `SetStyleDefaults` 하나(같은 test의 첫 구현으로 확인) | `stellar.yaml` 한 줄과 함께 `design/current`, 새 `design/generations/*.starmanifest`가 바뀜 |

## 완료 기준 대비

| 기준 | 상태 |
|---|---|
| apply/inverse fixture와 property test 등록 | 확인 |
| Undo 한 번 복원 | `style_defaults_selection_undoes_in_one_step`, `set_and_unset_round_trip_through_undo` |
| 세트 변경 뒤 저장 diff가 한 줄 | CLI `set` 경로는 확인. `ops::apply` 경로는 generation 파일 두 개가 더 바뀜(미충족) |
| 옛 codec version journal decode | schema 1(codec 필드 없음) journal decode test. codec 3 이하 command bytes는 그대로 읽힘(codec.rs 주석, 왕복 test) |
| 모르는 command 봉인 동작 유지 | 새 codec·선언 밖 종류 거부 test, `unknown_command_kind_is_rejected` |

## 차이(작업 지시와 다른 점)

- 이 작업은 command 코드를 바꾸지 않았습니다. command 본체는 앞선 기본값 세트 작업이 넣었고, 이번에는 완료 기준을
  다시 실행해 판정했습니다.
- `ops::apply`는 문서 레코드가 바뀌지 않는 transaction도 generation을 새로 씁니다. 이 파일은 이 작업 소유 밖이라
  고치지 않았습니다. 고칠 내용: transaction 전후 `to_records()`가 같으면 generation 저장을 건너뛰고 준비한 세트 선택만
  commit합니다(ADR-0115 결정 4).

## 남은 위험 / 한계

- 위 `ops::apply` 수정 전까지 앱·`stellar apply`로 세트만 바꾸면 Git diff가 세 파일입니다.
- 선행 IX.11.25(엔진이 세트로 푼 값만 받음)가 `[ ]`입니다.
