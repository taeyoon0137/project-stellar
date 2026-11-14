# 작업 보고서: XV.7.1 update 요청의 CurrentLibrarySnapshot·CandidateLibrarySnapshot

- WBS: XV.7.1
- 작업명: dependency update request가 selector 재해석 후 CurrentLibrarySnapshot과 CandidateLibrarySnapshot 생성
- 상태: 완료 (`[x]`)
- 선행: XV.2.4(`[x]`), XV.5.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 실제 Git fixture 테스트는 시스템 `git`을 씁니다.

## 변경 파일 목록

- `crates/dependencies/src/library_update.rs`(신규) — `LibraryUpdateRequest`, `CurrentLibrarySnapshot`,
  `CandidateLibrarySnapshot`, `LibraryUpdateCandidate`, `UpdateSnapshotSource` trait,
  `CacheUpdateSource`, `CandidateLoadError`, `UpdateError`, `prepare_library_update`, 테스트 공급자.
- `crates/dependencies/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XV.7.1.md`(신규), 계획서(XV.7.1 `[ ]`→`[x]`).

## 구현 요약

- **요청**: 사용 프로젝트 의존성 logical name과 선택 새 선택자입니다. 새 선택자가 없으면 `stellar.yaml` 선언
  선택자를 다시 해석합니다(branch 이동 확인).
- **현재**: lock의 root entry(경로 `[name]`)를 찾고 선언 library id와 같은지 확인합니다. snapshot은 cache에서만
  읽고(lock content hash 일치 필수, network 없음), 실패하면 `CurrentUnavailable`로 중단합니다.
- **후보**: `SelectorResolver`(운영은 XV.2.4 `GitSelectorResolver`, `ls-remote`·fetch·commit 검증)로 exact
  commit을 얻고, `CacheUpdateSource`가 materialize(immutable cache에 추가) → ProjectProbe·storage validator
  (`validate_library_entry`, XV.3.4) → snapshot 순으로 만듭니다. 다른 library id 저장소는 `Rejected`입니다.
- **검토 전용**: lock·문서·로드 상태를 바꾸지 않습니다. 결과는 현재/후보 commit, 같은 commit 여부, 선택자 변경
  여부를 제공합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-dependencies` | 종료 코드 0 |
| `cargo test -p stellar-dependencies` | 87 passed, 0 failed; doctest 1 passed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |
| 테스트 바이너리 30회 반복 실행(scratchpad `loop.sh`) | 30회 모두 통과(아래 Fix 이후) |

| 검사 | 결과 |
|---|---|
| 실제 Git: lock v1(first) + 새 선택자 v2 → current = first, candidate = second, 선택자 변경, 버전별 snapshot 내용·content hash 다름, lock 불변(`new_selector_yields_exact_current_and_candidate_commits`) | 통과 |
| 선언 branch `main` 재해석 → candidate = 이동한 second, 선택자 변경 없음 / tag v1 재해석 → 같은 commit·같은 snapshot(`declared_branch_is_reinterpreted_and_unchanged_tag_is_same_commit`) | 통과 |
| 미선언 이름·lock 없음·library id 불일치·없는 tag·후보 library id 불일치(ProjectProbe 거부)·현재 cache hash 불일치 구조화 오류, 오류에 URL 원문 없음(`update_failures_are_structured`) | 통과 |

## 차이(작업 지시와 다른 점)

- 검증 중 기존 `cache_lock::tests::two_process_readers_share_entries_while_fetch_is_held`가 약 13회 중 1회
  실패했습니다. 자식 프로세스가 `fs::write`로 ready 파일을 만드는 사이 부모가 빈 파일을 읽는 경쟁이었고, 이 작업
  변경과 무관한 기존 테스트 결함입니다. 별도 커밋 `Fix: cache lock 두 프로세스 테스트의 ready 파일 공개 경쟁
  제거`(임시 파일 → `rename` 공개)로 고쳤고, 이후 30회 반복에서 실패가 없었습니다.
- 후보의 전이 의존성 그래프 재해석(후보 commit의 `stellar.yaml` 전이 선언)은 이 단계에 넣지 않았습니다. lock 갱신
  (XV.7.4)이 resolver를 다시 실행해야 하는 부분입니다.

## 남은 위험 / 한계

- 운영 `UpdateSnapshotSource`에는 실제 `LibraryExportSource`(XV.5.1 차이 참고)와 project의 ProjectProbe·storage
  validator 연결(XV.3.4 `[~]`)이 필요합니다.
