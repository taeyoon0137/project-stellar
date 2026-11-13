# 작업 보고서: VII.2.7 ConvertFragmentToFrame·ConvertShapeToPath

- WBS: VII.2.7
- 작업명: `ConvertFragmentToFrame`과 `ConvertShapeToPath` command로 Phase VI 변환 계획 적용
- 상태: 완료 (`[x]`)
- 선행: VI.4.4, VI.6.5, VII.1.6

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/convert.rs`(신규) — `ConvertFragmentToFrame`, `ConvertShapeToPath`, 역 command `RestoreNodeRecords`,
  payload 기반 leaf bounds, 자식 위치 이동, 테스트 5건.
- `crates/commands/src/command.rs` — 세 variant, `CommandError::{ConversionKind, ConversionNotRepresentable}`.
- `crates/commands/src/kind.rs`, `crates/commands/src/codec.rs`, `crates/commands/src/lib.rs` — `0x0030`~`0x0032` 등록·직렬화.
- `docs/task-reports/VII.2.7.md`(신규), 계획서(VII.2.7 `[ ]`→`[x]`).

## 구현 요약

- **ConvertFragmentToFrame**: VI.4.4 `plan_fragment_to_frame`을 그대로 적용합니다. Fragment StableId를 유지한 채 계획의 Frame
  payload(가시 자식 bounds 크기·위치, clip 없음)로 종류를 바꾸고, 새 Frame이 만드는 좌표계에 맞춰 자식(중첩 Fragment를 거친
  자손, 숨김 포함)의 local 위치에 `child_offset`을 더합니다(Frame transform, Ellipse·Polygon bounds, Line 끝점, Path command).
  leaf bounds는 layout 이전이라 payload에서 계산합니다(Frame = transform 적용 크기 상자, Shape·Path = path bounds, Image =
  배치 상자, Text = layout 전 bounds 없음). 위치 필드가 없는 자손(Rectangle·Text·Image)이 있는데 offset이 0이 아니면 시각
  결과를 보존할 수 없어 `ConversionNotRepresentable`로 거부합니다.
- **ConvertShapeToPath**: VI.6.5 `plan_shape_to_path`로 같은 렌더 geometry(VI.5.6 평가 결과)의 Path로 바꿉니다. StableId·자손
  위치·modifier stack은 유지됩니다. 새 point·handle id는 caller가 `required_ids` 개수만큼 제공하며(재실행 결정론) NIL·중복·
  문서에서 사용 중인 id를 거부합니다.
- **RestoreNodeRecords**(두 변환의 역): 지정 node들의 레코드를 종류까지 통째로 되돌립니다. 부모·OrderKey는 바꿀 수 없고, 새 종류가
  자식(container 여부)·modifier stack(Fragment 금지)·들어오는 Frame clip 참조(Path여야 함)와 맞는지, 새 payload의 clip 대상·path
  element id가 유효한지 검사합니다. 변환의 inverse는 변환 전 레코드(Fragment와 옮긴 자손, 또는 원래 Shape)를 담습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-commands` | 완료 |
| `cargo test -p stellar-commands` | unit 53 passed, doctest 3 passed, 0 failed |
| `cargo clippy -p stellar-commands --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-commands -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| Fragment(Frame·중첩 Fragment 안 Ellipse·숨김 Frame) 변환: Frame 50×30 @ (10,20)·clip 없음, StableId·자식 순서 유지, 자손 위치 offset 적용, 문서 validator 통과, undo 후 5개 node **레코드 bytes 동일**·snapshot 동일(`fragment_to_frame_applies_plan_and_undo_restores_bytes`) | 통과 |
| offset ≠ 0에서 Text 자식 → `ConversionNotRepresentable`, Frame 대상 → `ConversionKind`, offset = 0이면 Text 있어도 변환·undo(`fragment_conversion_rejects_wrong_kind_and_unmovable_children`) | 통과 |
| 둥근 Rectangle → Path: command가 VI.5.6 평가와 동일, StableId·stack 유지, validator 통과, undo 후 레코드 bytes 동일, codec(`shape_to_path_keeps_id_geometry_and_undo_restores_bytes`) | 통과 |
| id 개수 부족·중복·사용 중 id·Frame 대상 거부(`shape_to_path_rejects_bad_ids_and_kinds`) | 통과 |
| clip 대상 Path를 Shape로 되돌리기·자식 있는 Frame을 Text로·순서 변경·빈 요청 거부, 0x0032 codec(`restore_records_checks_kind_compatibility`) | 통과 |

## 차이(작업 지시와 다른 점)

- 두 변환의 역으로 레코드 복원 command `RestoreNodeRecords`(`0x0032`)를 함께 등록했습니다(종류 변경을 되돌리는 command가
  필요하기 때문).
- leaf bounds는 layout(Phase X) 이전이라 payload에서 근사합니다. 위치 필드가 없는 자손이 있으면 offset ≠ 0 변환을 거부합니다.

## 남은 위험 / 한계

- Text·Image·Rectangle의 배치 위치는 layout 단계에서 정해지므로, layout이 생기면 leaf bounds와 자식 이동 규칙을 layout 결과
  기준으로 바꿔야 합니다.
