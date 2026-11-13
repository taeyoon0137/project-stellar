# 작업 보고서: XIII.3.5 초기 릴리스 agent lock 비활성 검증

- WBS: XIII.3.5
- 작업명: 잠금된 agent subtree와 외부 변경 충돌 정책은 로드맵 Phase R3에서 연결하며 초기 릴리스에는
  agent lock 코드가 비활성임을 검증
- 상태: 완료 (`[x]`)
- 선행: XIII.3.2(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/external_change.rs` — 모듈 문서(agent lock은 R3, 초기 릴리스에 상태·feature 없음)와
  검증 테스트 `initial_release_build_keeps_no_agent_state`.
- `docs/task-reports/XIII.3.5.md`(신규), 계획서(XIII.3.5 `[ ]`→`[x]`).

## 구현 요약

- 초기 릴리스에는 agent lock 상태·코드·Cargo feature를 두지 않습니다. 외부 변경 판단
  (`WindowExternalState`)은 agent 잠금을 참고하지 않고, 대기 상태(`ExternalChangePending`)는
  LoadedBase·CurrentMemory·ExternalDisk 세 snapshot만 보존합니다.
- R3에서 잠금된 subtree 충돌 정책을 연결할 때 이 테스트를 함께 바꾸도록 문서에 적었습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 241 passed, 0 failed; fuzz 통합 4 passed) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |

| 검사(`initial_release_build_keeps_no_agent_state`) | 결과 |
|---|---|
| **feature off build에 agent state 없음** — (1) 대기 상태 완전 분해가 세 snapshot 필드만으로 컴파일되고 창 상태 변형이 세 가지뿐(필드·변형이 늘면 컴파일 실패) | 통과 |
| (2) `crates/project/src` 전체 `.rs` 소스에 agent lock 상태 식별자(잠금·잠금 subtree·agent 편집 계열) 없음 | 통과 |
| (3) `crates/project/Cargo.toml`에 agent 관련 feature·`[features]` 없음(feature off가 유일한 빌드) | 통과 |

## 차이(작업 지시와 다른 점)

- "feature off build"는 agent lock feature 자체가 없는 현재 빌드로 검증했습니다. feature를 미리 만들어
  비워 두지 않았습니다(빈 placeholder 금지, 계획 1.2-6).

## 남은 위험 / 한계

- 소스 검사는 식별자 문자열 기준입니다. 다른 이름으로 agent 상태를 추가하면 잡지 못하므로 R3 이전의
  agent 관련 변경은 리뷰로도 확인해야 합니다.
