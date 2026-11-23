# 작업 보고서: XXIII.3.3 C를 target WorkingTree에 generation save

- WBS: XXIII.3.3
- 작업명: C를 target WorkingTree에 generation save하되 merge commit을 자동 생성하지 않음
- 상태: 완료 (`[x]`)
- 선행: V.5.4, XXIII.3.1
- 결정: [ADR-0110](../adr/ADR-0110-window-reconciliation-and-merge-result-apply.md) 결정 6

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git fixture).

## 변경 파일 목록

- `crates/ops/src/merge_apply/composer.rs`(신규) — `compose`(Source A·B·Base → C, source 기록), `Composition`,
  `apply_composition`, `ComposerApplyReport`, `ComposeError`, `ComposerApplyError`, generation save `write_documents`.
- `crates/ops/src/merge_apply.rs`(mod 등록), `merge_apply/tests.rs`, `tests/fixture.rs`(공용 fixture 확장),
  `tests/composer.rs`(신규).

## 구현 요약

- `compose`: source 해석(XXIII.1.1) → merge-base(XXIII.1.2) → 세 snapshot materialize(XXIII.1.3, 원본 미변경) → 각 snapshot의
  저장 generation을 `saved_semantic_snapshot`(무손실 증명)으로 올려 `CandidateSnapshot::build`(XXIII.2.1). 공통 base가 하나가
  아니면 `NoCommonBase`, A·B 컴포넌트 원본 집합이 base와 다르면 `ComponentSources`로 병합하지 않습니다.
- `apply_composition`: `CandidateSnapshot::validate`(XXIII.2.4) → C 문서 무손실 역투영 → 새 branch target이면 생성(XXIII.3.2)
  → generation save.
- generation save: 저장 잠금 → target revision을 검증 때 값과 비교(다르면 `StaleTarget`, 쓰기 0) → 현재 schema·저장된
  generation 확인 → 새로 참조하는 이미지 asset이 target에 있는지 확인 → 결과가 target 문서와 같으면 쓰지 않음 → 문서가 아닌 chunk는
  target pointer 재사용, 결과 문서 재직렬화 → `build_save_plan` + `run_save`(optimistic base check). `stellar apply` 쓰기와 같은
  저장 파이프라인입니다.
- Git: `commit`·`merge`·`add`·`reset`·`checkout`을 부르지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops --lib merge_apply` | 통과(composer 5, target 3, document 4) |
| `cargo clippy -p stellar-ops --all-targets -- -D warnings` | 경고 0 |

| 검사(Git state expected) | 결과 |
|---|---|
| current worktree: C의 두 변경이 target에 있고 generation +1, HEAD·`ls-files --stage`·MERGE_HEAD 없음이 적용 전과 같음, `git status`에 ` M design/current` | 통과 |
| new branch+worktree: Git이 만든 worktree에 C, 새 branch·worktree HEAD = 시작 commit(commit 0), merge 진행 표시 없음, 원래 worktree 그대로 | 통과 |
| existing worktree: 다른 linked worktree에 C | 통과 |
| target이 검증 뒤 바뀜 → `StaleTarget`, 파일 digest 그대로 | 통과 |
| unresolved conflict → `Blocked`, 쓰기 0 / `ChooseTheirs`로 해결한 뒤 적용 성공 | 통과 |

## 차이(작업 지시와 다른 점)

- 문서 의미(generation)만 씁니다. 컴포넌트 원본이 바뀐 source는 조용히 버리지 않고 거부합니다(ADR-0110 결정 6).
- 공통 base가 없는 비교(2-way·criss-cross)는 3-way 입력이 없어 거부합니다.

## 남은 위험 / 한계

- 텍스트 원본(ADR-0088)은 이 경로가 쓰지 않습니다. headless `stellar apply`와 같이 generation만 쓰고, 앱 창은 다음 열기·반영에서
  cache 이동(`Forward`)으로 원본을 맞춥니다.
- CLI·MCP 명령과 UI는 아직 없습니다(엔진 API만).
- 대형 프로젝트 적용 시간은 재지 않았습니다.
