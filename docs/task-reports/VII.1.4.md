# 작업 보고서: VII.1.4 command 결과 change set

- WBS: VII.1.4
- 작업명: command 결과에 changed nodes, changed properties, invalidation classes, new revision 반환
- 상태: 완료 (`[x]`)
- 선행: VII.1.2, VI.9.7

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/change.rs`(신규) — `ChangeRecorder`, `ChangeSet`, 공통 node 필드 key `node_keys`
  (`0x0001`~`0x0005`), `node_field_invalidation`, `STRUCTURE_INVALIDATION`.
- `crates/commands/src/edit.rs` — `EditContext`가 변경 기록기를 보유(`changes`, `into_changes`).
- `crates/commands/src/state.rs` — `DocumentState::execute`(검사 → validate → invert → apply → revision+1),
  `CommandOutcome { changes, inverse }`.
- `crates/commands/src/command.rs` — `CommandError::RevisionOverflow`, probe가 변경을 기록.
- `crates/commands/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VII.1.4.md`(신규), 계획서(VII.1.4 `[ ]`→`[x]`).

## 구현 요약

- **`ChangeSet`**: changed nodes(node별 `InvalidationSet`, 삽입 포함·삭제 제외), changed properties
  (`PropertyPath`, root = node 또는 modifier id), 전체 invalidation(합집합), inserted/removed nodes(순 효과),
  new revision. 무효화 class는 VI.9.7의 `InvalidationClass`(geometry·layout·paint)를 재사용합니다.
- **순 효과 규칙**: 같은 실행 안에서 삽입 후 삭제된 node는 어느 쪽에도 남지 않고, 삭제 후 재삽입된 node는 구조
  무효화를 가진 "바뀐 node"가 됩니다. `merge`로 transaction 안 여러 command 결과를 시간 순서로 합칩니다.
- **공통 node 필드 key**: 표시 여부·잠금·표시 이름·부모·순서 key를 `0x00xx` 대역에 고정했습니다(Frame `0x01xx`,
  Image `0x06xx`, instance `0x0Axx`, modifier `0x10xx`와 비충돌). 표시 여부·부모·순서는 구조 무효화, 잠금·이름은
  scene/layout 입력이 아니라 무효화 없음입니다.
- **command 결과**: `DocumentState::execute`가 성공 시 revision을 하나 올리고 `ChangeSet`(new revision 포함)과
  undo용 역 command를 반환합니다. 실패 시 revision은 그대로입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-commands` | 완료 |
| `cargo test -p stellar-commands` | unit 19 passed, doctest 3 passed, 0 failed |
| `cargo clippy -p stellar-commands --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-commands -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 단일 property command 실행 → changed nodes `[n12]`, properties `[(n12, DISPLAY_NAME)]`, invalidation 없음, new revision 4, 역 command 정확(`single_property_command_reports_expected_change_set`) | 통과 |
| Frame SIZE 단일 변경 기록 → registry 무효화(layout+paint)와 경로 일치(`single_property_change_set`) | 통과 |
| 삽입·삭제 순 효과, merge 시간 순서(`insert_and_remove_are_net_effects`) | 통과 |
| 공통 필드 key 대역·유일성, Frame key 비충돌(`common_field_keys_are_unique_and_outside_other_bands`) | 통과 |
| 거부된 요청은 revision 유지, revision 오버플로 거부·문서 무변경(`rejected_request_keeps_revision`) | 통과 |

## 차이(작업 지시와 다른 점)

- "단일 property command"는 실제 문서 command(VII.2.4 SetProperty) 전이라 테스트 전용 probe(표시 이름 변경)로
  검증했고, registry 기반 무효화는 Frame SIZE 기록 테스트로 따로 검증했습니다. SetProperty의 end-to-end change set은
  VII.2.4에서 검증합니다.
- 공통 node 필드 key(`node_keys`)를 이 작업에서 정의했습니다. change set이 부모·순서·표시 여부 변경을 속성 경로로
  보고하려면 key가 필요하기 때문입니다.

## 남은 위험 / 한계

- apply 중간 실패 시 되돌리기는 아직 없습니다(VII.1.5 transaction guard).
- `ChangeSet`을 layout/scene dirty set으로 바꾸는 변환은 X.7.1·XI.2.1의 몫입니다.
