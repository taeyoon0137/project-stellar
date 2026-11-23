# 작업 보고서: XXIV.2.2 unresolved conflict의 원본 보존과 nonzero exit

- WBS: XXIV.2.2
- 작업명: unresolved semantic conflict가 있으면 원본을 보존하고 nonzero exit로 앱에서 해결해야 함을 표시
- 상태: 완료 (`[x]`) — 2026-11-23 근거 커밋 기록: 구현·검증과 선행 판정은 이미 `[x]` 조건을 충족했고 작업 커밋 제목이 `Task(<ID>)` 형식이 아니어서 `[~]`에 머물렀습니다. 아래 "근거 커밋" 절에 실제 작업 커밋을 적었습니다. 이전 기록: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 계획 1.1의 "올바른 커밋"이 없음: 구현 커밋이 `Feat:`·`Fix:`·`Test:` 제목이라 `Task(<ID>)` 제목 커밋이 없고, 원장(`cargo xtask generate-ledger`)이 근거 커밋과 검증 시각을 채우지 못합니다. 구현·검증은 완료입니다; 선행 XXIV.2.1(`[~]`)가 `[x]`가 아님(계획 1.2 규칙 4). 보고 당시 기록: 완료 (`[x]`)
- 선행: XXIV.2.1
- 결정: [ADR-0081](../adr/ADR-0081-semantic-merge-resolution-and-git-driver.md) 결정 8

## 필수 실행 환경

- `DEV-MAC` (Rust, 시스템 Git 2.x, 로컬 검증).

## 변경 파일 목록

- `tools/stellar-cli/src/merge_driver.rs` — conflict 기록(`.stellar/merge/`), 성공 시 이전 기록 삭제, 해결 안내 메시지.
- `tools/stellar-cli/tests/merge_driver.rs` — conflict 시스템 Git 테스트.

## 구현 요약

- conflict가 있으면 `%A`를 쓰지 않고 1(`unresolved-conflicts`)로 끝납니다. Git은 그 경로를 unmerged(stage 1·2·3)로 남기고
  작업 트리에는 `%A`의 ours bytes가 marker 없이 남습니다.
- conflict 목록(종류·객체·대상·세 상태)을 `%P`를 담은 프로젝트의 `.stellar/merge/<경로>.json`에 임시 파일 뒤 rename으로 씁니다.
  폴더 안 `.gitignore`(`*`)로 Git 밖에 둡니다. 기록 실패는 경고 진단이며 driver 결과를 바꾸지 않습니다. 같은 경로가 나중에 병합에
  성공하면 이전 기록을 지웁니다.
- stdout 결과 JSON과 stderr 안내에 앱이나 `stellar merge-driver resolve`로 해결하라는 문구를 담습니다. driver는 merge 상태를
  끝내지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-cli --test merge_driver` | 3 passed |
| `cargo clippy -p stellar-cli --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 같은 폭을 다르게 바꾼 두 branch의 `git merge` exit 1, 작업 트리 = ours bytes, unmerged 경로 = 원본 파일, stage 3개 | 통과 |
| `.stellar/merge/` 기록 1개(property-value, 대상 root Frame), `git status --porcelain` = `UU <파일>`뿐, `MERGE_HEAD` 유지 | 통과 |

## 차이(작업 지시와 다른 점)

- "앱에서 해결" 안내에 headless 해결 명령(`merge-driver resolve`, XIV.12.4 행에서 추가)을 함께 적습니다.

## 남은 위험 / 한계

- 프로젝트 루트(`stellar.yaml`)를 찾지 못하면 기록을 쓰지 않고 경고만 남깁니다.

## 근거 커밋 (2026-11-23 기록)

이 행의 작업은 아래 커밋에 있습니다. 이 기록 커밋(`Task(XXIV.2.2)`)은 원장이 근거 커밋을 찾도록 남긴 것이고 코드 변경은 없습니다.

- `82da9464` Feat: merge driver conflict 시 원본 보존과 Git 밖 conflict 기록
