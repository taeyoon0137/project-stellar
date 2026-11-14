# 작업 보고서: XIV.5.1 StageSelectionRequest와 dependency closure 결과

- WBS: XIV.5.1
- 작업명: 선택한 SemanticChange ID 목록을 받는 `StageSelectionRequest`와 결과 dependency closure 정의
- 상태: 완료 (`[x]`)
- 선행: XIV.4.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-diff/src/stage.rs`(신규) — `ChangeUnit`, `ChangeId`, `SemanticChange::id`, `resolve_changes`, `UnknownChange`, `SelectionOperation`, `StageSelectionRequest`, `StageSelection`, `put_ids`/`read_ids`.
- `crates/semantic-diff/src/lib.rs` — `pub mod stage;`.
- `docs/task-reports/XIV.5.1.md`(신규), 계획서(XIV.5.1 `[ ]`→`[x]`).

## 구현 요약

- **변경 ID**: `ChangeId = (객체 StableId, 대상 단위)`, 단위는 객체 삭제·객체 추가·위치·속성 경로입니다.
  적용(XIV.3.9)이 같은 대상을 두 번 다루는 목록을 거부하는 단위와 같아 올바른 diff 안에서 유일합니다.
  교체(삭제+추가)는 두 ID입니다. ID에는 값이 없으므로 `resolve_changes`가 현재 변경 목록에서 다시 찾고,
  없으면 `UnknownChange`(오래된 요청)로 거부합니다.
- **요청**: `StageSelectionRequest { operation(Stage·Unstage), selected, approved }`. `approved`는 이전
  계획의 필수 함께-변경 중 호출자가 승인한 ID입니다(XIV.5.3·XIV.5.4에서 사용).
- **결과**: `StageSelection { operation, selected, closure }`는 closure ⊇ selected를 생성 시 강제하고
  `required_additions() = closure − selected`, `is_self_contained()`를 제공합니다.
- **canonical bytes**: 요청 `SSRQ`, 결과 `SSEL`(version 1, LE). ID 집합은 엄격 오름차순이어야 하며 decoder는
  절단·trailing·magic·version·작업 code·비정렬·경로 root 불일치·과대 개수를 구조화 오류로 거부합니다(미리
  할당하지 않음, panic 없음).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-diff` | unit 57 passed, fuzz 통합 4 passed, 0 failed |
| `cargo clippy -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-semantic-diff -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **round-trip**: stage 요청(선택 3 + 승인 3, 속성 index·key segment 경로 포함)·unstage 요청·selection(필수 4개) bytes 왕복 동일, `required_additions` 정확, closure 누락 거부(`request_and_selection_round_trip`) | 통과 |
| 부분 staged fixture의 세 diff에서 ID 유일·전부 되찾기, unstaged ID 목록 정확, 없는 ID는 `UnknownChange`(`change_ids_are_unique_within_a_diff_and_resolve_back`) | 통과 |
| 교체는 삭제·추가 두 ID(`replacement_has_distinct_remove_and_add_ids`) | 통과 |
| 모든 길이 절단·trailing·magic·작업 code·비정렬·root 불일치·`u32::MAX` 개수·closure 누락 bytes 거부, panic 0(`malformed_bytes_are_rejected_without_panic`) | 통과 |

## 차이(작업 지시와 다른 점)

- unstage(XIV.7)도 같은 요청·결과 형식을 쓰도록 `operation` 필드를 두었습니다.
- closure 계산 자체는 dependency graph(XIV.5.2)가 담당하고, 이 작업은 결과 형식과 불변식만 정의합니다.

## 남은 위험 / 한계

- `ChangeId`는 diff 기준 상대 ID라 status가 바뀌면(저장·외부 Git 조작) 같은 ID가 다른 값의 변경을 가리킬
  수 있습니다. 적용 단계는 현재 status로 다시 계산한 변경을 쓰므로 값 불일치는 생기지 않지만, UI는 요청 직전
  status를 다시 보여야 합니다.
