# 작업 보고서: XIV.4.2 staged·unstaged 연속 수정 속성의 세 값 보존

- WBS: XIV.4.2
- 작업명: 같은 PropertyPath가 staged와 unstaged에서 연속 수정된 경우 base/head, staged, working 세 값을 모두 보존
- 상태: 완료 (`[x]`)
- 선행: XIV.4.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-diff/src/status.rs` — `LayerValue`, `PropertyLayers`, `SemanticStatus::{properties, property, layered_properties}`.
- `docs/task-reports/XIV.4.2.md`(신규), 계획서(XIV.4.2 `[ ]`→`[x]`).

## 구현 요약

- `SemanticStatus::classify`가 staged·unstaged 어느 쪽에서든 속성 변경이 있는 모든 경로에 대해
  HEAD·Index·WorkingTree 값을 `PropertyLayers`로 함께 보존합니다. 값은 변경의 old/new가 아니라 세
  snapshot에서 직접 읽으므로 객체 추가·삭제와 겹친 경우도 정확합니다.
- `LayerValue`는 객체 없음(`NoObject`)·속성 없음(`Unset`)·값을 구분하고 canonical 비교(float bit
  단위)로 `is_staged`(HEAD≠Index), `is_unstaged`(Index≠WT), `is_layered`(둘 다)를 판정합니다.
- `layered_properties()`는 연속 수정된 속성만, `property(path)`는 경로 하나의 세 값을 돌려줍니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-diff` | unit 50 passed, fuzz 통합 4 passed, 0 failed |
| `cargo clippy -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-semantic-diff -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **H=1, I=2, W=3 fixture**: rect 11 폭 1→2(staged)→3(unstaged)에서 `property(size)`가 head=1·index=2·working=3, staged 변경 (1→2)·unstaged 변경 (2→3) 각각 하나, `layered_properties()`에 그 경로 하나(`same_property_staged_then_modified_keeps_three_values`) | 통과 |
| 같은 테스트: staged만(1,2,2)·unstaged만(1,1,3)은 한 영역만 표시, HEAD로 되돌림(1,2,1)도 세 값 보존, 바뀌지 않은 경로는 `None` | 통과 |
| 부분 staged fixture에서 영역별 다른 속성의 세 값, staged 추가 객체를 WT에서 다시 바꾼 경우 head=`NoObject`(`layers_cover_object_addition_and_removal`) | 통과 |

## 차이(작업 지시와 다른 점)

- 연속 수정 경로만이 아니라 바뀐 모든 속성 경로의 세 값을 보존합니다. UI가 영역 하나만 바뀐 속성도 같은
  형식으로 표시할 수 있게 하기 위함이며, 연속 수정 여부는 `is_layered`로 구분합니다.

## 남은 위험 / 한계

- 세 값은 메모리 상 status에만 있고 FFI 표현(XIV.13.1)은 아직 없습니다.

## 후속 수정 (리뷰 반영)

- **Git 연결 없음(리뷰 minor, 사실 기록)**: H=1·I=2·W=3 fixture는 XIV.4.1과 같은 메모리 구성(chunk bytes 경유)이며
  실제 Git index·작업 트리에서 읽은 snapshot이 아닙니다. 상태 `[x]` 유지 근거와 후속 필요 사항은 XIV.4.1 보고서의
  후속 절과 같습니다.
