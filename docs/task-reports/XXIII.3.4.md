# 작업 보고서: XXIII.3.4 적용 전 source revision·OID 재검사와 stale candidate 차단

- WBS: XXIII.3.4
- 작업명: source가 변한 경우 apply 전에 revision/OID를 재검사하고 stale candidate를 차단
- 상태: 완료 (`[x]`)
- 선행: XXIII.1.3, XXIII.3.3
- 결정: [ADR-0110](../adr/ADR-0110-window-reconciliation-and-merge-result-apply.md) 결정 7

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git fixture).

## 변경 파일 목록

- `crates/ops/src/merge_apply/composer.rs` — `recheck_sources`, `ComposerApplyError::StaleSource`, `apply_composition`이 쓰기 전
  재검사 호출.
- `crates/ops/src/merge_apply/tests/composer.rs` — source 변경 시험 두 개.
- `crates/ops/src/merge_apply.rs` — 모듈 문서.

## 구현 요약

- `compose`가 Source A·B의 해석 결과(`ResolvedSource`: commit OID·snapshot 원천·worktree)와 materialize 내용 지문(path·mode·blob ID,
  XXIII.1.3)을 `SourceFingerprint`로 기록합니다.
- `recheck_sources`: 같은 요청(`SourceRef`)을 다시 해석해 해석 결과가 같은지 비교합니다(branch·tag가 다른 commit을 가리키면
  `commit`). index·작업 트리 source는 같은 `materialize_source`로 지문을 다시 계산해 비교합니다(`content`). commit 계열 source의
  내용은 OID가 고정하므로 다시 읽지 않습니다. 원본을 바꾸지 않습니다.
- `apply_composition`은 C 검증 뒤, 새 branch·worktree 생성과 generation save **전에** 재검사합니다. 다르면
  `StaleSource { side, reason }`이고 아무것도 만들거나 쓰지 않습니다. target 쪽 변경은 XXIII.3.3의 잠금 안 revision 비교가
  `StaleTarget`으로 막습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops --lib merge_apply` | 통과(14: composer 7, target 3, document 4) |
| `cargo clippy -p stellar-ops --all-targets -- -D warnings` | 경고 0 |

| 검사(source update fixture 실패) | 결과 |
|---|---|
| C를 만든 뒤 branch b에 새 commit → `StaleSource { side: "b", reason: "commit" }`, target 파일 digest 그대로 | 통과 |
| A = 현재 작업 트리, C를 만든 뒤 tracked 파일 수정 → `StaleSource { side: "a", reason: "content" }`, 파일 digest 그대로 | 통과 |
| source가 그대로면 적용 성공(XXIII.3.3 시험 전부) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 재검사와 쓰기 사이의 짧은 구간에 source가 바뀌는 경우는 막지 않습니다. C는 재검사 시점의 source와 같고, target 쪽은 저장
  잠금과 optimistic base check가 보호합니다.
- index·작업 트리 source의 재검사는 그 source를 다시 materialize하므로 source 크기에 비례합니다.
