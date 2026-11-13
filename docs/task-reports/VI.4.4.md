# 작업 보고서: VI.4.4 Fragment→Frame 변환 계획

- WBS: VI.4.4
- 작업명: Fragment→Frame 순수 변환 계획 — 새 Frame은 Fragment StableId 유지, 필요한 기본 payload 추가
- 상태: 완료 (`[x]`)
- 선행: VI.4.1, VI.3.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/fragment.rs` — `plan_fragment_to_frame`,
  `FragmentToFramePlan`, `ConvertError`.
- `docs/task-reports/VI.4.4.md`(신규), 계획서(VI.4.4 `[ ]`→`[x]`).

## 구현 요약

- **순수 계획**: 문서를 변경하지 않고 계획만 반환합니다. 적용은
  `ConvertFragmentToFrame` command(VII.2.7)의 몫입니다.
- **StableId 유지**: 계획의 `stable_id`는 Fragment의 id 그대로라 외부 참조
  (ReferenceIndex inbound 등)가 변환 후에도 유효합니다.
- **기본 payload**: 가시 자식 bounds(VI.4.3)를 감싸는 size와 그 origin으로의
  translate transform, clipping `Disabled`(Fragment는 clip하지 않았으므로 시각
  결과 유지). 빈 Fragment는 크기 0·항등 transform입니다.
- **좌표계 보정**: 새 Frame이 좌표계를 만들므로 자식 local 위치에 더할
  `child_offset = −origin`을 함께 반환합니다. 자식 목록은 OrderKey 순서(VI.2.4)를
  유지합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 78 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| StableId·자식 순서(삽입 순서와 다른 OrderKey)·inbound 참조 유지, bounds 기반 payload·offset(`plan_keeps_id_child_order_and_references`) | 통과 |
| 빈 Fragment 계획, 비Fragment·stale 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- `child_offset` 적용은 자식 종류별 위치 표현(Frame transform, Shape 파라미터 등)에
  따라 command가 수행해야 합니다. 의미 동일성 검증은 VII.2.7의 undo 테스트로
  확인합니다.
