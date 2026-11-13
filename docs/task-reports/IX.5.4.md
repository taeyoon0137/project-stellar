# 작업 보고서: IX.5.4 preview session별 runtime state store

- WBS: IX.5.4
- 작업명: runtime state store를 preview session별로 유지하고 project 저장에는 definition·initial state만 포함
- 상태: 완료 (`[x]`)
- 선행: IX.5.1(이 lane에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/state_store.rs`(신규) — `PreviewSessionId`, `InstanceKey`, `InstanceState`,
  `StateStore`(open/close session, initialize, commit, 조회), `effective_property_values`, `StateStoreError`.
- `crates/document/src/component/mod.rs` — 모듈 등록.
- `docs/task-reports/IX.5.4.md`(신규), 계획서(IX.5.4 `[ ]`→`[x]`).

## 구현 요약

- **세션 메모리 전용**: `StateStore`는 (preview session → 인스턴스 chain → 상태 변수 → 값)을 메모리에만
  보관하며 직렬화 API가 없습니다. 저장 경로(`ComponentLibrary::encode`, scene payload)는 store를 읽지 않으므로
  project 저장에는 상태 머신 정의와 initial expression만 들어갑니다. session을 닫으면 그 상태가 사라집니다.
- **초기화**: 각 변수의 initial expression을 인스턴스의 유효 Property 값(타입이 맞는 direct override, 없으면
  정의 default — `effective_property_values`)으로 평가합니다. 하나라도 실패하면 기존 상태를 바꾸지 않습니다.
- **commit**: 여러 값을 한꺼번에 씁니다. 변수가 없거나 타입이 맞지 않는 값이 하나라도 있으면 아무것도
  바꾸지 않습니다(IX.5.5 원자 dispatch의 기반).
- 인스턴스 위치는 scene 인스턴스부터 nested instance node까지의 chain(`InstanceKey`)이라 nested 인스턴스
  상태도 같은 store에 둘 수 있습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 274 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 두 session 초기화·한 session만 commit → session 독립, library bytes·scene payload 동일, session close 후 상태 제거(`preview_changes_do_not_change_saved_bytes`) | 통과 |
| override 없음/있음에 따라 initial(`initiallyOn` 참조) 값 결정(`initial_state_uses_instance_property_values`) | 통과 |
| 타입 불일치 값이 섞인 commit은 전부 거부·상태 불변, 없는 변수·중복 session·없는 session 거부(`commit_is_all_or_nothing`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- nested instance의 유효 Property 값(정의 tree override + drilling binding)은 binding 계층(VIII.6, IX.4)이
  생기면 계산합니다. 현재 `initialize`는 호출자가 Property 값을 넘기는 형태로 열어 두었습니다.
- preview session 생성·종료를 셸 preview 수명과 묶는 것은 Phase XII의 몫입니다.
