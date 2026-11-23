# 작업 보고서: XIV.11.3 세 stage를 Semantic Merge에 전달하고 raw ↔ semantic 대응 유지

- WBS: XIV.11.3
- 작업명: 세 snapshot을 Semantic Merge engine에 전달하고 raw file conflict와 semantic conflict mapping을 유지
- 상태: 완료 (`[x]`)
- 선행: XIV.10.9, XIV.11.2
- 결정: [ADR-0081](../adr/ADR-0081-semantic-merge-resolution-and-git-driver.md) 결정 5·7

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/ops/src/candidate/session.rs`(신규) — `ConflictSession`, `SessionUnit`, `UnitKind`, generation stage 열기.
- `crates/ops/src/candidate/session/tests.rs`(신규) — 메모리 Git 포트와 대응 fixture.
- `crates/ops/src/candidate.rs` — 모듈 등록.

## 구현 요약

- `ConflictSession::open`은 `MergeGit` 포트로 unmerged 목록과 진행 중 작업을 읽고 경로를 나눕니다: 컴포넌트 원본은 세 stage
  blob을 codec으로 읽은 파일 단위, `design/current`는 세 stage pointer가 가리키는 세대를 작업 트리의 manifest·chunk에서
  비공개 사본으로 열어(`for_materialized_snapshot`, 무손실 검사 `saved_semantic_snapshot`) 만든 문서 단위입니다. 그 밖의
  `stellar.yaml`·`design/` 경로는 이유와 함께 의미 병합 제외, 나머지는 raw conflict(XIV.11.4)입니다.
- 단위마다 `CandidateSnapshot`을 만들고 raw 경로 → 객체(세 snapshot 합집합), 객체 → raw 경로(`paths_for`), 전역 번호 conflict →
  단위 경로를 제공합니다. 세션은 작업 트리·index를 읽기만 합니다.
- XIV.11.2의 git-porcelain materialize는 ops에서 부를 수 없어(crate graph) 포트로 같은 일을 합니다(ADR-0081 결정 7).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops --lib candidate::session` | 2 passed |
| `cargo clippy -p stellar-ops --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 컴포넌트 conflict + raw README + `stellar.yaml`: 단위 1·raw 1·제외 1, 원본 경로 → {정의, root, 자식}, 폭 conflict → 원본 경로, `paths_for`(root·자식) = 원본 경로, 자동 병합 자식 출처 B, Git 호출은 읽기뿐 | 통과 |
| 한 stage가 conflict marker인 원본은 단위가 아니라 제외 목록 | 통과 |

## 차이(작업 지시와 다른 점)

- generation 단위의 대응(`design/current` → 문서 객체)은 시스템 Git 통합 테스트(XIV.12.4 행의 CLI 테스트)에서 확인합니다.

## 남은 위험 / 한계

- generation 단위는 세 세대의 manifest·chunk가 작업 트리에 있어야 합니다. 한쪽이 GC로 지운 세대는 제외 목록에 이유와 함께
  남습니다.
