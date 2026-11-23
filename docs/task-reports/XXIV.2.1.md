# 작업 보고서: XXIV.2.1 Git custom merge driver noninteractive mode

- WBS: XXIV.2.1
- 작업명: Git custom merge driver가 base/ours/theirs/result path를 받아 Semantic Merge를 실행하는 noninteractive mode
- 상태: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 계획 1.1의 "올바른 커밋"이 없음: 구현 커밋이 `Feat:`·`Fix:`·`Test:` 제목이라 `Task(<ID>)` 제목 커밋이 없고, 원장(`cargo xtask generate-ledger`)이 근거 커밋과 검증 시각을 채우지 못합니다. 구현·검증은 완료입니다; 선행 XXIV.1.3(`[~]`)가 `[x]`가 아님(계획 1.2 규칙 4). 보고 당시 기록: 완료 (`[x]`)
- 선행: XXIV.1.3
- 결정: [ADR-0081](../adr/ADR-0081-semantic-merge-resolution-and-git-driver.md) 결정 8

## 필수 실행 환경

- `DEV-MAC` (Rust, 시스템 Git 2.x, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-diff/src/records.rs` — `component_records`(컴포넌트 정의·정의 tree 역투영), `ProjectionFailure` 두 원인.
- `crates/ops/src/candidate/component_file.rs`(신규) — 원본 bytes ↔ snapshot, v3 encoder와 재해석 검증, 경로 판정.
- `crates/ops/src/candidate/file_merge.rs`·`file_merge/tests.rs`(신규) — 파일 하나의 3-way merge.
- `crates/ops/src/candidate.rs` — 모듈 등록.
- `tools/stellar-cli/src/merge_driver.rs`(신규), `tools/stellar-cli/src/lib.rs`·`cli.rs`(등록), `tests/fixtures/help.txt`(도움말 snapshot 한 줄).
- `tools/stellar-cli/tests/merge_driver.rs`(신규).

## 구현 요약

- `stellar merge-driver %O %A %B %P`: `%P`가 `design/components/*.stellar.yaml`이면 세 bytes를 컴포넌트 원본 codec으로 읽어
  snapshot으로 올리고 엔진으로 병합합니다. 빈 base는 정의 없음입니다.
- conflict가 없으면 candidate 검증(XIV.12.3)을 통과한 `ValidatedCandidate`만 v3 원본으로 쓰고, 그 bytes를 codec으로 다시 읽어
  같은 snapshot일 때만 `%A`에 쓰고 0을 반환합니다. driver는 Git을 실행하지 않습니다.
- 지원하지 않는 경로는 69(`unsupported-file`), 읽기·검증 실패는 65, conflict는 1이며 이때 `%A`를 쓰지 않습니다(XXIV.2.2에서 기록
  보강).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops --lib candidate::` | 6 passed |
| `cargo test -p stellar-cli --test merge_driver` | 2 passed |
| `cargo test -p stellar-cli` | 전체 통과(`output_fixtures`의 저장 잠금 시간 테스트 1개는 병렬 부하에서 한 번 실패, 단독 재실행 통과 — 이 변경과 무관) |
| `cargo clippy -p stellar-ops -p stellar-cli -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 무변경 파일을 다시 쓰면 원본과 같은 bytes(encoder 충실도) | 통과 |
| 폭·높이, 배경과 자식 추가 병합 결과 bytes = 두 편집을 차례로 적용한 원본 | 통과 |
| 시스템 Git: 두 branch가 같은 컴포넌트에 다른 자식 Frame을 추가 → driver 없으면 exit 1(대조군), driver 등록 저장소에서 `git merge` exit 0·작업 트리 clean·`query component` node 3개 | 통과 |
| 인자 수 오류 64, 지원하지 않는 파일 69이고 `%A` 미변경 | 통과 |

## 차이(작업 지시와 다른 점)

- 결과 형식은 이 기준 커밋의 유일한 컴포넌트 쓰기 버전 v3입니다. 텍스트 원본 v4(ADR-0077 lane)가 들어오면 codec 공개 encoder로
  바꿉니다. 그 전에도 재해석 검증이 형식 불일치를 실패로 막습니다.
- 등록은 이 행의 테스트에서 Git config를 직접 씁니다. 사용자 승인 등록 명령은 XXIV.2.3입니다.

## 남은 위험 / 한계

- View·문서 텍스트 원본은 아직 driver 대상이 아닙니다(V.10 전환 뒤 같은 dispatch에 codec 추가).
- 병합 결과가 정의 삭제면 driver가 표현할 수 없어 nonzero입니다(Git은 삭제 대 수정을 driver 없이 처리합니다).
