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

## 사후 수정 — RestoreNodeRecords 결과 검사 비용 (2026-11-14, 사후 리뷰 RESTORE-BACKSTOP-PERF)

- **지적**: 앞선 수정(`Fix: RestoreNodeRecords가 View root 종류 변경과 결과 문서 불변식 위반을 거부`)이 붙인 결과 문서
  전체 backstop(`model.to_records()`로 모든 레코드 clone + `validate_document` 최대 2회)이 레코드 하나만 바뀌어도 매번 돌아,
  두 변환의 undo·redo(역 command가 RestoreNodeRecords) 비용이 문서 크기에 비례해 크게 늘었습니다.
- **수정**: 부모·순서·modifier stack이 바뀌지 않는 이 command가 새로 만들 수 있는 VI.10.1 위반을 대상 검사가 모두 다루도록
  하고 전체 재구성 검사를 뺐습니다. 대상 검사가 놓치던 유일한 경우 — 새로 path element가 되는 id를 다른 참조(clip·modifier,
  교체 레코드의 새 clip)가 이미 가리키는 결과 — 를 `ReferenceTypeMismatch`로 거부합니다(교체 node의 옛 clip과 원래 있던
  element 재사용은 제외). 전체 검사 함수는 test oracle(`#[cfg(test)]`)로만 남겼습니다.
- **측정**(`DEV-MAC`, release, root ← group 100개 ← Frame, 이름만 바꾸는 복원 1건의 validate 평균, 임시 측정 test — 커밋하지
  않음, node 수는 leaf Frame 기준이고 root·group 101개 별도): node 1,000: 1.47 ms → 0.27 ms, 10,000: 85.4 ms → 3.70 ms,
  50,000: 247 ms → 28.9 ms.
- **test**: `restore_rejects_new_path_elements_that_references_point_to`(clip·modifier·같은 command의 새 clip 세 경우를 oracle과
  범위 검사가 모두 거부, 허용 경우 2개), `scoped_restore_validation_never_accepts_what_the_full_validator_rejects`(random 문서 150개
  × 후보 12개: 범위 검사 통과 ⇒ oracle 통과, 통과·거부 각 200건 초과). 새 검사를 끈 변이에서 두 test가 모두 실패함을 확인했습니다.
- 검증: `cargo test -p stellar-commands` 84 passed(1 ignored는 기존 장기 fuzz), `cargo clippy -p stellar-commands --all-targets --
  -D warnings` 경고 0, `cargo fmt -p stellar-commands -- --check` 통과.
- **남은 한계**: 대상 검사가 쓰는 문서 전체 index(`id_owners`·`ChildIndex`·`reference_index`, 각 O(N))는 그대로라 50,000 node에서
  28.9 ms로 undo·redo 10 ms 예산을 아직 넘습니다. 모델 수준 자식·참조·id index(증분 유지)가 필요하며 이 수정 범위(함수 단위) 밖입니다.
