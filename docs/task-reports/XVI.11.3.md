# 작업 보고서: XVI.11.3 두 branch conflict→Semantic Conflict 해결→git add E2E

## WBS

- WBS: XVI.11.3
- 작업명: 두 branch conflict→Semantic Conflict 해결→git add E2E
- 상태: 부분 완료 (`[~]`) — generation 문서·컴포넌트 원본 conflict는 기준을 통과하지만, 앱 창으로 편집한 프로젝트의 텍스트 원본 View 파일 conflict는 conflict 세션이 의미 병합하지 않습니다(실측).
- 선행: XIV.14.4
- 결정: [ADR-0111](../adr/ADR-0111-headless-release-e2e-scenarios.md)
- lane: `headless-e2e`

## 변경 파일

- `tests/integration/tests/e2e_branch_conflict_resolve.rs`: 시나리오

코드 변경은 테스트뿐입니다(제품 코드 변경 없음).

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-integration-tests --locked` | 0 — 전체 통합 테스트 통과(e2e 6개 파일 포함) |
| `cargo clippy -p stellar-integration-tests --tests --locked` | 0 — 경고 0 |
| `cargo xtask validate-adr --base 682c7b0c` | lane 결과 보고에 적음 |
| `STELLAR_CHECK_BASE=682c7b0c cargo xtask check-core` | lane 결과 보고에 적음 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: Apple Silicon macOS 로컬 실행, debug build, 시스템 Git. 앱 UI 없이 앱 runtime(`AppRuntime`)과 headless ops만 씁니다.
  테스트 HOME·전역 Git 설정은 테스트 프로세스 임시 폴더로 격리합니다(ADR-0111 결정 2).

## 테스트

| 테스트 | 결과 |
|---|---|
| `two_branch_conflict_is_resolved_semantically_and_added` | 통과 — 앱 생성 프로젝트 + 컴포넌트 base commit → feature(width 1280·이름·컴포넌트 200×40)와 main(width 1024·배경색·컴포넌트 240×40)을 `stellar apply`·`component-source resize`로 편집·commit → `git merge` conflict(`design/current`·컴포넌트 원본) → conflict 세션 단위 2개, raw·제외 0, semantic conflict 2개(width·컴포넌트 크기)만, 미해결 적용은 쓰지 않음 → theirs·ours 해결 → `apply_session`: unmerged 0, MERGE_HEAD = feature, HEAD = merge 전 main, commit 없음 → width 1280·이름(theirs)·배경색(ours)·컴포넌트 240, `validate` 오류 0, staged 재검증 통과, 작업 트리 = index |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음).

## 차이

- 시나리오는 CLI로 편집한(텍스트 원본이 없는) 프로젝트를 씁니다. 앱 창으로 두 branch를 편집하면 `design/pages/<page>/<view>.stellar.yaml`이 conflict로 남고 conflict 세션이 그 경로를 "의미 병합하지 않는 Stellar 저장 경로"로 제외해 unmerged가 남습니다(임시 실측, ADR-0111 결정 5).

## 남은 위험

- 텍스트 원본(View·문서·token) codec을 conflict 세션과 merge driver dispatch에 더해야 앱 기본 프로젝트에서 행 기준을 만족합니다(ADR-0081 단점).
- 선행 XIV.14.4는 원장에 `[ ]`입니다. 이 시나리오가 그 행의 증거로 쓰일 수 있는지는 통합 때 판정합니다.
