# 작업 보고서: XIV.7.1 선택 staged 변경의 Index 기준 역적용

- WBS: XIV.7.1
- 작업명: 선택 staged changes를 Index snapshot에서 inverse 적용해 CandidateIndexSnapshot 계산(기준은 HEAD가 아니라 현재 Index)
- 상태: 완료 (`[x]`)
- 선행: XIV.4.1(`[x]`, 이 lane), XIV.3.10(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-diff/src/stage.rs` — `ChangeId::inverse`, `StageSelection::inverted`(crate 내부), `build_unstage_candidate`, unstage 요청·오류 ID 변환.
- `docs/task-reports/XIV.7.1.md`(신규), 계획서(XIV.7.1 `[ ]`→`[x]`).

## 구현 요약

- `build_unstage_candidate(head, index, request)`는 staged 목록(HEAD → Index)의 ID로 받은 선택을 **현재 Index**에
  역적용합니다. 계산은 Index → HEAD 방향 diff(staged 변경의 역변경)에서 stage와 같은 공통 경로
  (`build_candidate`: closure·승인 검사·적용·semantic validator·문서 validator)로 합니다. 선택하지 않은 staged
  변경은 그대로 남습니다.
- `ChangeId::inverse`(추가 ↔ 삭제, 위치·속성 동일)로 요청·결과 selection·ID 오류(UnknownChange·
  ApprovalRequired·UnexpectedApproval)를 staged ID 기준으로 주고받습니다. `change.inverse().id() ==
  change.id().inverse()`입니다.
- HEAD·Index snapshot은 읽기만 합니다. stage 요청은 `WrongOperation`으로 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-diff` | unit 76 passed, fuzz 통합 4 passed, 0 failed |
| `cargo clippy -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-semantic-diff -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **staged A+B에서 A만 제거**: A(rect 11 크기)·B(ellipse 14 추가) staged → A만 unstage하면 rect 11은 HEAD 값, 14는 남음, 결과 = `apply(Index, [A의 역변경])`, 남은 staged = [B]; B만 unstage하면 14 제거·A 유지; 둘 다 unstage하면 HEAD와 동일(`unstaging_one_of_two_staged_changes_inverts_it_on_the_index`) | 통과 |
| staged 목록에 없는 ID → `UnknownChange`(staged ID로 보고), stage 요청 거부, `ChangeId::inverse` 규칙·역변경 ID 일치(`unstage_rejects_stale_ids_and_stage_requests`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 필수 함께-unstage 검사(XIV.7.2)는 같은 공통 경로가 이미 적용하며, 계획 API와 component fixture는 XIV.7.2에서
  추가합니다.

## 남은 위험 / 한계

- candidate를 Git index에 기록하는 경로(XIV.7.3 → XIV.6)는 이 lane에서 아직 없습니다(XIV.5.5 이후 선행 미완).
