# 작업 보고서: XII.2.5 창별 탭·active view·viewport·선택을 WindowViewState에 연결

> 아래 초기 구현·검증 기록은 선별 통합 원본 범위 `3892c5e3..2636e8ce`에서 가져왔습니다.
> 현재 기준 `eac4531b` 위의 재검증·수정·남은 범위는 [세션 통합 보고서](session-takeover-integration.md)를 따릅니다.

## WBS

- WBS: XII.2.5
- 작업명: 창별 열린 tab, active view, viewport, selection을 WindowViewState에 연결합니다.
- 상태: 완료 (`[x]`)
- 선행: VII.3.1 (`[x]`), XII.2.1 (`[x]`, 이 lane `6d0785e`)

## 변경 파일

- `crates/commands/src/view_tabs.rs` (신규) — `CanvasViewport`(유한 값, 배율 2%\~25600%), `ViewTab`, `ViewTabs`(열기·
  active·닫기·viewport·사라진 View 정리), `first_view`, `document_has_view`, 단위 테스트 5개.
- `crates/commands/src/selection.rs`, `crates/commands/src/lib.rs` (수정) — `WindowViewState`에 `tabs`(열린 탭·active
  view·탭별 viewport) 연결, 모듈 공개.
- `crates/history/src/window.rs` (수정) — `WindowHistory::view_mut`(탭·viewport·영역 preview처럼 기록하지 않는 view 상태용).
- `crates/runtime/src/window_session.rs` (수정) — 로드 시 첫 View 탭 열기, `tabs`·`open_view`·`close_view`·`set_viewport`,
  `WindowSessionEditError::View`, 창 두 개 격리 테스트.
- `crates/runtime/src/app.rs` (수정) — `open_view_tab`·`close_view_tab`·`set_viewport`, event `WindowViewChanged`(0x0108),
  실패 코드 `UnknownView`(19)·`NoActiveView`(20), 상태 snapshot `tab_count`·`active_view`·`viewport`.
- `crates/runtime/src/app_window_tests.rs` (수정) — runtime 경계 view state 격리 테스트.
- `crates/ffi/src/lib.rs`, `tools/xtask/src/ffi_header.rs`, `crates/ffi/include/stellar_ffi.h` (수정·재생성) — 새 event kind와
  실패 코드 노출(헤더 일치 테스트).
- `docs/adr/ADR-0028-window-session-shell-surface.md` (수정) — XII.2.5 절.
- `docs/task-reports/XII.2.5.md`(신규), 계획서(XII.2.5 `[ ]`→`[x]`)

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-commands -p stellar-history -p stellar-ffi` | 0 (commands 87 passed·1 ignored, history 35, ffi 60 등) |
| `cargo test -p stellar-runtime --lib` | 0 (53 passed) |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 |
| `cargo fmt --all -- --check` | 0 |
| `cargo run -q -p xtask -- generate-ffi-header` 후 헤더 일치 테스트 | 통과 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: DEV-MAC만이므로 없음

## 테스트

- 검증 기준 "창간 상태 격리":
  - `window_session::tests::view_state_opens_the_first_view_and_stays_per_window`: 같은 저장 프로젝트로 연 두 창이 각자 첫 View
    탭·기본 viewport로 시작하고, A의 탭 추가·viewport 변경·선택이 B에 보이지 않으며 B의 viewport 변경도 A에 보이지 않음.
    탭·viewport는 문서 revision·history·레코드를 바꾸지 않음(선택만 history 1개). 탭으로 돌아가면 그 탭 viewport 유지. 통과.
  - `app_window_tests::view_state_changes_stay_in_their_window`: `AppRuntime` 요청·완료 event·상태 snapshot으로 같은 격리와
    실패 코드(`UnknownView`, `UnknownWindowSession`, 마지막 탭을 닫은 뒤 `NoActiveView`)를 확인. 통과.
- `view_tabs` 단위 테스트: viewport 유한 값·배율 경계·-0.0 정규화, 탭 중복 없는 열기·active 전환, 탭별 viewport, 닫을 때
  이웃 active 규칙, 사라진 View 탭 정리. 통과.
- 남은 경고 0.

## 성능

- 해당 없음(탭·viewport 변경은 document thread의 O(탭 수) 작업, 상태 snapshot에 세 필드 추가).

## 차이

- 계획 1.2 규칙 5: 새 type·API라 구현 전 테스트는 컴파일 실패로만 재현됩니다.
- 셸에 탭 UI가 없으므로 이 행은 C 함수를 더하지 않고 event kind·실패 코드만 헤더에 노출했습니다. viewport의 셸 연결(줌 명령,
  배율 표시)은 XII.4.3에서 합니다.
- 탭 열기·닫기·viewport 변경은 history에 기록하지 않습니다(§23.2: 문서 편집과 선택만 기록).

## 남은 위험

- 복구 snapshot(V.9)의 탭·viewport 기록과 이 view state의 연결은 이 행 범위 밖입니다.
- View를 추가·삭제하는 command가 생기면 그 뒤 `ViewTabs::prune_missing`을 부르는 경로를 붙여야 합니다.
