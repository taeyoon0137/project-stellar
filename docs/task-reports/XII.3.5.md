# 작업 보고서: XII.3.5 입력 T0 수신 trace와 요청 지연

- WBS: XII.3.5
- 상태: `[~]` — 수신·대기열·요청 trace 구현과 Rust 검증 완료, 정상 앱의 DocumentThread 전달 미연결
- 선행: XI.3.4 `[x]`, XII.1.2 `[x]`
## 필수 실행 환경

`DEV-MAC`. 검증 로그는 `/tmp/stellar-interaction-t0-test.log`,
`/tmp/stellar-interaction-t0-clippy.log`, `/tmp/stellar-interaction-macos-smoke.log`에 남겼습니다.

## 변경 파일 목록

- `crates/ffi/src/input.rs`: DocumentThread 입력 분류.
- `crates/ffi/src/input_queue.rs`: 입력과 receipt 보관, 병합 시 최초 시각 유지.
- `crates/ffi/src/lib.rs`: 잠금·변환 전 T0 수신 기록, 요청 trace 종단 테스트.
- `crates/runtime/src/input_trace.rs`, `crates/runtime/src/lib.rs`: T0/T1 후보와 지연 계산, input lane 요청 API.
- `docs/adr/ADR-0029-input-latency-trace.md`: FFI 내부 변경의 결정과 미연결 경계.
- 구현 계획과 `planning/task-status.yaml`: 현행 설명·완료 기준·선행을 유지하고 상태만 `[~]`로 갱신.

## 통합 근거와 구현 요약

2026-11-14 `git fetch origin main`으로 확인한 최신 origin 및 이 lane의 기준점은 `eac4531b`입니다.
`git cherry HEAD 289c3793 3892c5e3` 결과 대상 7개 커밋은 모두 미통합(`+`)이었습니다.
그중 `a1cc25f8`의 구현, `b59a8623`의 모듈 문서 수정, `289c3793`의 미연결 경계 설명을 선별 반영했습니다.
최신 origin의 시계 기준점 검증·미래 timestamp 방어와 DocumentThread 종료 보완은 유지했습니다.
ADR-0029는 현재 origin 파일 목록과 로컬 전체 커밋 이력에서 다른 주제와 충돌하지 않았습니다.

`InputReceipt`는 운영체제 시각을 코어 단조 domain으로 옮긴 값과 FFI 수신 시각을 함께 가집니다.
move·wheel을 합치면 최신 이벤트 값과 가장 이른 receipt를 보존하고 병합 수를 셉니다.
`submit_hit_test`는 input lane 접수·실행 시작·종료 시각을 기록합니다. 지연은 수신→시작,
수신함 대기는 접수→시작, 처리 시간은 시작→종료의 포화 뺄셈입니다.
trace에는 입력 위치·키·문자열을 담지 않습니다. C ABI와 직렬화 형식은 유지합니다.

## 차이(작업 지시와 다른 점)

정상 진입점 증거와 미연결 경계는 다음과 같습니다.

- 정상 프로젝트 창은 `ShellCanvasHostViewController`에서 `CanvasView.runtimeHandle`을 설정합니다.
  `CanvasView`의 AppKit mouse/key/wheel handler → `submitInput` →
  `stellar_runtime_submit_mac_event` → `InputQueue::push_received`까지 T0 receipt가 남습니다.
- `submitted_events_carry_t0_into_document_thread_hit_test_trace`는 FFI 제출 뒤 테스트 코드가
  `pop_received`로 꺼내 `DocumentThread::spawn`으로 만든 thread에 보냅니다. hit-test 본문은 `()`를
  반환하는 closure입니다. 실제 scene hit test나 정상 프로젝트의 AppRuntime dispatch 증거가 아닙니다.
- 정상 앱에는 `pop_received` → `submit_hit_test` → canvas hit test → 상호작용 상태 기계 연결이
  아직 없습니다. 기존 `stellar_runtime_poll_input`은 입력 검사에 쓰이며 receipt를 버립니다.
- 따라서 원 패치의 `[x]` 표기를 그대로 옮기지 않았습니다. XII.3.7 재개 시 이 연결을 구현하고 정상
  프로젝트 창에서 trace가 생기는지 검사해야 합니다. WBS 설명·선행은 변경하지 않았습니다.

## 실행한 명령 / 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime -p stellar-ffi` | 종료 0. ffi unit 64, runtime unit 51, integration 8+2, runtime doctest 11 통과 |
| `cargo clippy -p stellar-ffi -p stellar-runtime --all-targets -- -D warnings` | 종료 0, 경고 없음 |
| `cargo fmt -p stellar-ffi -p stellar-runtime -- --check` | 종료 0 |
| `cargo xtask generate-ledger` | 종료 0, 1,145행 재생성 |
| `cargo xtask sync-plan --check` | 종료 0 |
| `cargo xtask validate-task-status` | 종료 0 |
| `cargo xtask generate-ffi-header --check` | 종료 0, drift 없음 |
| `cargo xtask validate-crate-graph` | 종료 0 |
| `cargo xtask validate-adr <전체 변경 경로>` | 종료 0, 신규 ADR을 포함한 실제 변경 경로로 검사 |
| `cargo xtask validate-task-report docs/task-reports/XII.3.5.md` | 필수 절·명령 표 누락으로 처음 종료 65, 형식 보완 후 종료 0 |
| `scripts/macos/smoke.sh` | 종료 0, `BUILD SUCCEEDED`, `smoke OK`; 입력 60건·10,000 drag 병합, shell integration 110·project open 173 검사 통과 |
| `git diff --check` | 종료 0 |

- 수동 시계 fixture는 T0 `[90, 100]`, T1 `[400, 420]`, queue latency 300ns를 검사합니다.
  FFI fixture는 down→100 drag→up의 병합 수 `[0, 99, 0]`과 수신·접수·실행 시각 순서를 검사합니다.
- 최신 timestamp 방어 회귀 검사 3건을 포함한 기존 ffi 테스트도 함께 통과했습니다.
- macOS 빌드에 AppIntents.framework 의존성이 없어 metadata 추출을 생략한다는 도구 경고 1건이
  남았습니다. Swift 컴파일 오류·경고는 없습니다. 실제 프로젝트 열기 검사는 통과했지만 입력의
  DocumentThread 연결 또는 편집 완료를 증명하지 않습니다.
- 원장 생성은 `cargo xtask generate-ledger`로 수행했습니다. 성능 게이트는 실행하지 않았습니다.

## 남은 위험 / 한계

- DocumentThread 수신함이 같은 key의 요청을 대체하면 이전 trace와 최초 receipt는 보존되지 않습니다.
  최신 요청 기준의 지연이므로 연속 입력의 대기를 과소평가할 수 있습니다.
- 정상 앱의 hit test, 입력별 trace ID, T0/T1 확정, trace 보존·내보내기, 기준 장비 성능은 미검증입니다.
  후보 시각 fixture를 실제 `T5 - T0` 성능 충족으로 해석하지 않습니다.
- 전체 workspace 검증과 push는 통합 담당 범위입니다.

## 2026-11-15 재검증

- Codex 작업 검토(계획 B9)에서 이 행의 상태를 다시 정했습니다. 위 기록은 Codex 작업 시간에 작성·통합되어 상태의 근거로 쓰지 않고, 아래 실행 결과를 근거로 씁니다.
- 논리 검토: 계획 U035 수용(상태를 보수적으로 표시).
- 환경: `DEV-MAC`(Apple M3 Max, macOS 27.2, rustc 1.96.0, Apple Git 2.54.0). 코드 기준은 `3463761b`이며 이 검토의 문서 커밋은 코드를 바꾸지 않습니다.

| 명령 | 결과 |
|---|---|
| `cargo test --workspace --no-fail-fast` | 0 — 4,039 통과, 0 실패, 10 ignored(수동 측정·장기 fuzz) |

- `tests::mac_input::submitted_events_carry_t0_into_document_thread_hit_test_trace` 통과.
- 판정: `[~]` 유지. 정상 앱은 아직 hit-test 요청을 DocumentThread로 보내지 않아 실제 앱 경로의 trace 증거가 없습니다.
