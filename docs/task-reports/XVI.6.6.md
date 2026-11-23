# 작업 보고서: XVI.6.6 Semantic Merge fuzz target

- WBS: XVI.6.6
- 작업명: Semantic Merge fuzz target을 추가해 결과 valid 또는 명시 conflict/error만 허용
- 상태: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 계획 1.1의 "올바른 커밋"이 없음: 구현 커밋이 `Feat:`·`Fix:`·`Test:` 제목이라 `Task(<ID>)` 제목 커밋이 없고, 원장(`cargo xtask generate-ledger`)이 근거 커밋과 검증 시각을 채우지 못합니다. 구현·검증은 완료입니다; 선행 XIV.10.8(`[~]`)가 `[x]`가 아님(계획 1.2 규칙 4). 보고 당시 기록: 완료 (`[x]`)
- 선행: XIV.10.8
- 결정: [ADR-0081](../adr/ADR-0081-semantic-merge-resolution-and-git-driver.md) 결정 10

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-merge/tests/semantic_merge_fuzz.rs`(신규).

## 구현 요약

- 고정 seed 결정론 harness(XVI.6.5와 같은 방식, 새 의존성 없음). 반복 수는 `STELLAR_SEMANTIC_MERGE_FUZZ_ITERATIONS`.
- 입력: `world` fixture에 무작위 편집 1\~4개(이름·표시·node 삭제·추가·이동, token 타입·값, interface Property 목록,
  scene·정의 tree override, clipping 참조, 컴포넌트 삭제, 새 인스턴스)를 양쪽에 따로 적용합니다. 편집 결과가 그쪽에서 이미
  invalid여도 됩니다.
- 판정: clean이면 세 입력 어디에도 없던 위반 0, conflict는 `(StableId, 대상)` 엄격 순서, 결과 bytes round-trip. 아니면
  `MergeError`. panic은 실패이고 입력을 `CARGO_TARGET_TMPDIR/semantic-merge-fuzz/`에 hex로 남깁니다.
- decoder 대상: 유효 `MergeInput` bytes를 비트 뒤집기·자르기·삽입으로 변형해 decode하고, 수용한 입력도 같은 판정으로 병합합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-merge --test semantic_merge_fuzz` | 3 passed(clean 249, conflict 151, 오류 0; decoder 수용 5·거부 195) |
| `STELLAR_SEMANTIC_MERGE_FUZZ_ITERATIONS=5000 cargo test -p stellar-semantic-merge --test semantic_merge_fuzz` | 3 passed(clean 3057, conflict 1943; decoder 수용 77·거부 2423) |
| `cargo clippy -p stellar-semantic-merge --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 결과 검증 반복(XIV.10.8)을 임시로 끈 엔진에서 같은 harness 실행 | seed 31·33·50 등에서 `invalid silent merge`(DanglingReference)로 실패 — harness가 결함을 잡음을 확인, 원복함 |
| 같은 seed 두 번 실행 결과 bytes 동일(`campaign_is_deterministic`) | 통과 |

## 차이(작업 지시와 다른 점)

- cargo-fuzz(nightly) 대신 기존 fuzz 행과 같은 결정론 harness입니다(XVI.6.1·XVI.6.5와 같은 방식).

## 남은 위험 / 한계

- 편집 생성기는 fixture의 객체 종류(scene node, 컴포넌트, 인스턴스, token)에 한정됩니다. View·Page 목록과 modifier 편집은
  엔진 unit test(`random_member_list_edits_stay_coherent` 등)가 다룹니다.
