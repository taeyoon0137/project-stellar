# 작업 보고서: XII.2.1 ProjectWindowSession core type

> 아래 초기 구현·검증 기록은 선별 통합 원본 범위 `3892c5e3..2636e8ce`에서 가져왔습니다.
> 현재 기준 `eac4531b` 위의 재검증·수정·남은 범위는 [세션 통합 보고서](../status/session-takeover-integration.md)를 따릅니다.

## WBS

- WBS: XII.2.1
- 작업명: `ProjectWindowSession` core type에 ProjectIdentity, WindowSessionId, 독립 document state, view state, history,
  loaded generation을 정의합니다.
- 상태: 완료 (`[x]`)
- 선행: IV.2.3 (`[x]`), VII.5.1 (`[x]`), V.4.4 (`[x]`)

## 변경 파일

- `crates/runtime/src/window_session.rs` (신규) — `ProjectWindowSession`, 로드 오류 `WindowSessionLoadError`, 편집 오류
  `WindowSessionEditError`, 테스트 fixture(저장된 문서가 있는 임시 프로젝트)와 단위 테스트 4개.
- `crates/runtime/src/lib.rs` (수정) — `window_session` 모듈 공개.
- `crates/runtime/Cargo.toml` (수정) — `stellar-commands`·`stellar-history`·`stellar-storage` 의존(ADR-0013과 crate graph
  정책이 이미 허용한 edge), 테스트 전용 `stellar-document` `test-support` dev-dependency.
- `docs/task-reports/XII.2.1.md`(신규), 계획서(XII.2.1 `[ ]`→`[x]`)

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-runtime` | 0 (45 + 8 + 2 + 11 passed, 0 failed) |
| `cargo clippy -p stellar-runtime --all-targets -- -D warnings` | 0 (경고 0) |
| `cargo fmt -p stellar-runtime -- --check` | 0 |
| `cargo run -q -p xtask -- validate-crate-graph` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: DEV-MAC만이므로 없음

## 테스트

- `window_session::tests::two_sessions_do_not_share_state`(검증 기준): 같은 저장 프로젝트로 두 session을 열고 A에서
  이름 변경 → B 문서 semantic snapshot·revision·history 불변, B에서 선택 → A 선택·history 불변, B 편집은 A 변경을 보지
  않은 B 문서 위에서 실행, 두 session의 loaded generation은 같은 번호지만 별도 값, A를 버린 뒤에도 B 동작. 통과.
- `loads_identity_window_document_history_and_generation`: identity·window·작업 문서(첫 root 문서)·revision 0·빈
  history·빈 선택·generation 1을 로드하고, 작업 문서 chunk(manifest·ViewTree 2·subtree 2)를 기록함. 통과.
- `unsaved_project_opens_without_document_or_generation`: current pointer가 없는 프로젝트는 문서·generation 없이 열리고
  편집은 `NoDocument`. 통과.
- `corrupt_generation_is_a_typed_error`: manifest 디렉터리를 지우면 `WindowSessionLoadError::Generation`. 통과.
- 남은 경고 0.

## 성능

- 해당 없음(성능 경로 변경 없음. 로드는 창을 열 때 document thread에서 한 번).

## 차이

- 계획 1.2 규칙 5(실패 먼저 재현): 새 type이라 구현 전 테스트는 컴파일 실패로만 재현됩니다. 테스트를 먼저 쓰고 구현으로
  통과시켰습니다.
- 창 하나는 generation의 첫 root 문서 하나를 작업 문서로 엽니다. 다른 root 문서는 읽지 않고, 저장할 때 base chunk를 그대로
  이어 쓰도록 작업 문서가 차지한 chunk 집합(`document_chunks`)을 함께 보관합니다. 문서 전환은 이 행 범위 밖입니다.
- current generation을 `load_current_generation`(manifest hash 포함)과 `LazyGeneration::open`(문서 로드)으로 두 번 읽고 둘이
  다르면 `ConcurrentSave`로 실패합니다. storage(ADR 보호 경로)에 accessor를 더하지 않기 위해서입니다.
- view state는 선택만 있습니다. 탭·active view·viewport 연결은 XII.2.5 범위입니다.

## 남은 위험

- session은 아직 runtime session 표·FFI에 연결되지 않았습니다(XII.2.2).
- 같은 프로젝트를 여러 창에서 열 때 저장 충돌 정책은 XII.7 범위입니다.
