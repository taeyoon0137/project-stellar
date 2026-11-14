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
  하고 전체 재구성 검사를 뺐습니다. 대상 검사가 놓치던 경우 — 새로 path element가 되는 id를 다른 참조(clip·modifier,
  교체 레코드의 새 clip)가 이미 가리키는 결과 — 를 `ReferenceTypeMismatch`로 거부합니다(교체 node의 옛 clip과 원래 있던
  element 재사용은 제외). 전체 검사 함수는 test oracle(`#[cfg(test)]`)로만 남겼습니다. 이때 이것을 "유일한 경우"로 보고
  "범위 검사 통과 ⇒ 전체 validator 통과"라고 적었으나 틀렸습니다. clip을 그대로 둔 교체 레코드가 빠져 있었고, 아래
  "사후 수정 — 유지한 clip과 path element id 재사용"에서 고쳤습니다.
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
  계획서 XVI.4.7로 등록했습니다(아래 사후 수정 절).

## 사후 수정 — 유지한 clip과 path element id 재사용 (2026-11-14, 사후 리뷰 posthoc_expr_comp_cmd)

- **지적**: 위 범위 검사에 검증 구멍이 있었습니다(회귀). 같은 RestoreNodeRecords 안에서 (a) clip을 그대로 둔 채 이름만 바꾼
  Frame F(clip 대상은 미해결 id X)와 (b) X를 새 path element로 쓰는 Path 레코드를 함께 보내면, 범위 검사는 통과하지만 결과
  문서는 VI.10.1 `RefTypeMismatch`(clip 대상이 PathElement)를 어겼습니다. 둘째 반복은 교체 node의 clip을 새 payload 검사에
  맡기고 넘어갔고, 셋째 반복은 clip이 그대로면 대상이 새로 claim된 element여도 건너뛰었습니다. RestoreNodeRecords는 codec에
  등록된 command(`0x0032`)라 디코드된 입력으로도 이 경로에 닿습니다.
- **수정**: 교체 레코드 clip 검사의 예외를 좁혔습니다. clip을 그대로 두었더라도 대상이 이 command가 새로 claim한 element(교체
  전에는 교체 node의 element가 아니던 id)면 `ReferenceTypeMismatch`로 거부합니다. 계속 허용하는 경우는 clip을 그대로 두었고
  대상이 교체 전에도 교체 node의 element였던 경우(이미 있던 위반이라 이 command가 만든 것이 아님)뿐입니다.
- **경우 정리**: 부모·순서·modifier stack이 그대로인 이 command가 VI.10.1에서 새로 만들 수 있는 위반은 root 종류, 자식이 있는
  node의 종류, modifier owner 종류, path element id(NIL·중복), 참조 대상 종류입니다. 참조 대상 종류는 다음처럼 나눠 검사합니다.
  - 교체되지 않은 Frame의 clip이 종류가 바뀌는 node를 가리킴: 첫 반복의 들어오는 clip 검사(modifier 참조는 종류와 무관).
  - 교체되지 않은 clip, 모든 modifier 참조(교체 node 것 포함)가 새로 claim한 element를 가리킴: 둘째 반복.
  - 교체 레코드의 clip(바꿨든 유지했든)이 기존 element·다른 종류 id를 가리킴: 첫 반복(`kind_after`). 새로 claim한 element를
    가리킴: 셋째 반복. 다시 claim한 element(교체 전에도 element)는 clip을 바꾼 경우만 거부.
  - 교체되지 않은 clip이 다시 claim한 element를 가리킴: 교체 전에도 element였으므로 이미 있던 위반입니다.

  이 정리는 사람이 한 분석이며 증명은 아닙니다. 그래서 "전체 validator와 같은 판정"은 아래 property test로 확인한 범위에서만
  주장합니다.
- **test**: `restore_rejects_new_path_elements_that_references_point_to`에 이 조합을 레코드 순서 두 가지로 추가했습니다.
  `restore_keeps_allowing_unchanged_clip_to_reclaimed_element`(이미 잘못된 문서에서 clip 유지 + element 다시 claim은 범위 검사와
  oracle 모두 허용)를 새로 만들었습니다. property test `scoped_restore_validation_never_accepts_what_the_full_validator_rejects`에는
  문서마다 "payload를 유지한 Frame의 미해결 clip 대상을 다른 node의 새 Path element로 재사용"하는 후보를 의도적으로 섞고, 그런
  후보가 50건을 넘는지 확인합니다. 수정 전 코드에서는 회귀 test(이 조합이 `Ok`)와 property test(seed 1)가 모두 실패했고, 수정 후
  모두 통과합니다.
- 검증(`DEV-MAC`): `cargo test -p stellar-commands` unit 85 passed(1 ignored는 기존 장기 fuzz), doctest 3 passed. clippy·fmt는
  마지막 `cargo xtask check-core`에 포함했습니다.
- **남은 한계(등록)**: 50,000 node에서 28.9 ms인 O(N) 문서 index 비용은 계획서 XVI.4.7(모델 수준 증분 id·자식·참조 index)로
  등록했습니다. 성능 예산 파일(`config/performance-budgets.yaml`)은 ADR 보호 경로라 이 수정에서 항목을 추가하지 않았고, 측정
  기준은 XVI.4.7 검증 열에 두었습니다.

## 5만 leaf에서 단일 Frame 레코드의 이름 복원

`DocumentModel`이 Frame clip과 활성·비활성 modifier의 참조를 소유 node의 편집과 함께 갱신합니다. 역 command는 이 인덱스로 참조를 읽고, 자식이 허용되지 않는 종류만 자식 수를 조회합니다. Path element의 ID 중복과 clip 대상 종류를 판정해야 하는 경우에는 기존 전체 ID 검사를 사용합니다.

Apple M3 Max, macOS 27.2, release 빌드에서 root 한 개·그룹 100개·leaf Frame 50,000개를 만들고 같은 leaf의 이름을 101회 번갈아 복원했습니다. 검증과 command 실행은 개별 측정이며 실행에는 요청 생성, 검증, 역 command 생성 및 변경 적용이 포함됩니다.

| 시간 | 변경 전 p50 | 변경 후 p50 | 변경 전 p95 | 변경 후 p95 |
|---|---:|---:|---:|---:|
| 검증 | 15.684 ms | 0.333 µs | 16.559 ms | 0.458 µs |
| command 실행 | 15.523 ms | 0.875 µs | 16.116 ms | 1.750 µs |

재측정 명령은 `cargo test -p stellar-commands --release measure_restore_single_frame_50k -- --ignored --nocapture`입니다. 같은 대상의 warm-cache 측정입니다. 다른 node 종류, 앱 화면 갱신, 저장, 기준 장비의 결과는 별도 검증이 필요합니다. 전체 ID 인덱스와 다른 command의 전체 트리 조회가 남아 있어 XVI.4.7은 진행 전 상태를 유지합니다.
