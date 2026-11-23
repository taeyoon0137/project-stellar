# 작업 보고서: XXIII.2.1 독립 CandidateSnapshot C

- WBS: XXIII.2.1
- 작업명: SemanticMergeEngine 결과를 독립 `CandidateSnapshot C`로 생성
- 상태: 완료 (`[x]`)
- 선행: XIV.10.9, XXIII.1.3
- 결정: [ADR-0081](../adr/ADR-0081-semantic-merge-resolution-and-git-driver.md) 결정 4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-merge/src/candidate.rs`(신규), `crates/semantic-merge/src/lib.rs`(모듈 등록).
- `crates/ops/src/compare.rs` — `ProjectCandidate`, `candidate_from_projects`; `semantic_merge_projects`가 같은 경로 사용.

## 구현 요약

- `CandidateSnapshot`은 입력 세 snapshot(base·A·B)의 사본, 병합 결과 snapshot, conflict 목록을 가진 메모리 값입니다. 파일·Git
  index를 읽거나 쓰지 않습니다.
- ops `candidate_from_projects`는 세 폴더를 기존 읽기 전용 snapshot(XXIII.1.3)으로 열어 C를 만듭니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-merge --lib candidate` | 통과 |
| `cargo test -p stellar-ops --lib compare` | 4 passed |

| 검사 | 결과 |
|---|---|
| C 생성 후 A·B snapshot 값 동일, C가 A·B와 다른 독립 값 | 통과 |
| 세 프로젝트 폴더의 전체 파일 digest가 C 생성 전후 동일(`candidate_from_projects_leaves_source_files_unchanged`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- C는 입력 세 snapshot을 복사해 메모리를 세 배 더 씁니다. 큰 프로젝트의 비용은 Merge Composer UI(XXIII.4) 때 측정합니다.
