# 작업 보고서: XII.10.28 Path point 편집 command

- WBS: XII.10.28
- 작업명: Path point 삽입·이동·삭제, handle 이동, handle 대칭 모드 전환, subpath 닫기·열기 command
- 상태: 완료 (`[x]`) — 2026-11-23 근거 커밋 기록: 구현·검증과 선행 판정은 이미 `[x]` 조건을 충족했고 작업 커밋 제목이 `Task(<ID>)` 형식이 아니어서 `[~]`에 머물렀습니다. 아래 "근거 커밋" 절에 실제 작업 커밋을 적었습니다. 이전 기록: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 계획 1.1의 "올바른 커밋"이 없음: 구현 커밋이 `Feat:`·`Fix:`·`Test:` 제목이라 `Task(<ID>)` 제목 커밋이 없고, 원장(`cargo xtask generate-ledger`)이 근거 커밋과 검증 시각을 채우지 못합니다. 구현·검증과 선행 판정은 `[x]` 조건을 충족합니다. 보고 당시 기록: 완료 (`[x]`) — 코어 command·문서 모델·property test까지. 셸 연결은 XII.10.29의 몫입니다.
- 선행: VI.6.3, VII.1.6, VII.6.1
- 결정: ADR-0079 결정 4

## 필수 실행 환경

- `DEV-MAC` (Rust 단위·property 테스트).

## 변경 파일 목록

- `crates/document/src/path.rs` — `HandleMode`(Independent·Aligned·Mirrored), `IdentifiedPath`의 point별 handle
  mode(기본값이 아닌 것만), 저장 형식의 선택적 handle mode 절, closed enum `PathEdit`와
  `IdentifiedPath::edited`, `PathError::{UnknownElement, InvalidEdit}`.
- `crates/commands/src/path_edit.rs`(신규) — `EditPathPoint` command(validate·invert·apply·codec), fixture 테스트.
- `crates/commands/src/{kind,command,codec,lib}.rs` — `edit-path-point`(0x0080, codec 2) 등록.
- `crates/commands/src/test_support.rs`, `crates/commands/src/property_tests.rs` — 생성기와 VII.6.1 왕복 등록.
- `docs/task-reports/XII.10.28.md`(신규).

## 구현 요약

- `EditPathPoint { node, edit: PathEdit }` 하나가 연산 7개를 closed enum으로 담습니다: `InsertPoint`(segment를
  t에서 분할, 곡선은 de Casteljau로 모양 유지), `MovePoint`(두 handle도 함께 이동), `DeletePoint`(앞뒤 segment를
  합침. 둘 다 곡선이면 바깥 handle로 곡선, 아니면 직선. 시작 point면 다음 point가 시작), `MoveHandle`(소속
  point의 mode로 반대 handle을 맞춤), `SetHandleMode`, `CloseSubpath`, `OpenSubpath`.
- point·handle은 StableId로 가리키며, 편집하지 않은 point·handle의 id와 mode는 유지합니다. 분할로 생기는 point와
  handle id는 command가 담습니다(재실행 결정론). 문서 안 다른 영속 id와 겹치면 거부합니다.
- handle mode는 `IdentifiedPath` 안에 기본값이 아닌 point만 저장합니다. 저장 bytes는 mode가 있을 때만 끝에
  `u32 개수 + (point id 16 + tag 1)*` 절을 붙이므로 mode 없는 path는 이전 형식과 같은 bytes입니다.
- 역은 원래 레코드를 되돌리는 `RestoreNodeRecords`입니다(지운 point·handle의 id·좌표를 정확히 복원).
- 검토 보완: path를 다시 만드는 기존 경로가 mode를 버리지 않게 했습니다. Fragment → Frame 자손 이동은
  `IdentifiedPath::with_data`, 복제·Paste는 `IdentifiedPath::remapped`로 mode를 새 point id로 옮깁니다
  (`crates/commands/src/{convert,duplicate}.rs`). semantic snapshot은 mode를 담지 않으므로 fixture가 inverse 뒤 path
  payload 전체도 비교하고, mode가 있는 point의 삭제·mode 변경 Undo test를 더했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-commands path_edit` | 6 passed(검토 보완 후, mode Undo 포함): 삽입(직선·곡선 분할 좌표, 기존 point id 유지), 이동·삭제 4종, Mirrored·Aligned·Independent handle, 닫기·열기, 잘못된 편집 9종 거부 |
| `cargo test -p stellar-commands property_tests` | `edit_path_point_round_trip` 포함 전체 통과(random document 160 seed, 60건 이상 forward/inverse snapshot 일치), 등록 검사 통과 |
| `cargo test -p stellar-document path` | `handle_mode_section_is_optional_and_validated` 포함 통과 |
| `cargo test -p stellar-document -p stellar-commands -p stellar-history` | document 369, commands 144, history 49 등 전부 통과 |
| `cargo clippy -p stellar-commands -p stellar-document -p stellar-history --all-targets -- -D warnings` | 경고 없음 |

| 검사 | 결과 |
|---|---|
| command별 apply/inverse fixture | 통과(각 fixture가 apply 결과 확인 → inverse 뒤 semantic snapshot 원본과 같음 → canonical bytes 왕복·절단 거부) |
| point 삽입 후 기존 point ID 유지 | 통과 |
| VII.6.1 property test 등록 | 통과(`every_phase_vii_command_is_registered`) |

## 차이(작업 지시와 다른 점)

- handle 대칭 모드는 기존 path 모델에 없어 `IdentifiedPath`에 저장 필드를 더했습니다(ADR-0079). mode가 없는 path의
  저장 bytes는 바뀌지 않지만, mode가 있는 path는 이전 앱이 읽지 못합니다(trailing bytes 거부).
- 분기형 vector network와 열린 subpath 끝에 point를 이어 붙이는 연산은 넣지 않았습니다(XII.10.29의 Pen 연장이
  필요하면 같은 closed enum에 tag를 더합니다).

## 남은 위험 / 한계

- 셸 벡터 편집 모드(XII.10.29)와 FFI 연결은 없습니다.
- `Aligned` 길이 유지는 f32 계산이라 반복 이동 시 미세 오차가 쌓일 수 있습니다.
- 닫는 구간(Close가 그리는 직선)은 분할 대상이 아닙니다.

## 근거 커밋 (2026-11-23 기록)

이 행의 작업은 아래 커밋에 있습니다. 이 기록 커밋(`Task(XII.10.28)`)은 원장이 근거 커밋을 찾도록 남긴 것이고 코드 변경은 없습니다.

- `2a4b9a95` Fix: Path 변환·복제가 handle mode를 잃지 않게 함
- `b254ff3e` Feat: Path point 편집 command와 handle 대칭 모드 추가
