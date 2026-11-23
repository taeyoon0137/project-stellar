# XII.6.3 macOS 프로젝트 복제와 열기

## WBS

- WBS: XII.6.3
- 상태: 완료 (`[x]`) — 2026-11-23 근거 커밋 기록(ledger-refresh): 2026-11-15 재검증에서 구현·검증은 끝났고 `Task(XII.6.3)` 커밋만 없어 `[~]`였습니다. 선행 IV.6.4는 `[x]`입니다. 아래 "근거 커밋" 절에 실제 작업 커밋을 적었습니다. 이전 기록: `[~]` — 구현은 커밋됐지만(`c1779e5a`, `7530f8a6`) `Task(XII.6.3)` 커밋이 없습니다. 아래 "2026-11-15 재검증" 절을 참고합니다.
- 선행: IV.6.4

## 변경 파일

- `crates/project/src/clone.rs`, `crates/git-porcelain/src/repo.rs`에서 취소 가능한 복제와 기존 대상 보존을 처리합니다.
- `crates/runtime/src/app/create.rs`, `crates/runtime/src/app.rs`, `crates/ffi/src/lib.rs`, `tools/xtask/src/ffi_header.rs`에서 복제 요청을 기존 프로젝트 열기와 연결합니다.
- macOS의 `ShellAppCoordinator.swift`, `ShellRuntimeEventPump.swift`와 기존 Welcome UI가 주소, 대상 폴더, 취소를 전달합니다. `ShellProjectOpenCheck.swift`, `ShellIntegrationCheck.swift`, `main.swift`, `scripts/macos/smoke.sh`에 검증 경로가 있습니다.

## 명령과 종료 코드

| 명령 | 종료 코드 |
| --- | --- |
| `Stellar --project-clone-check "$HOME"` | 0 |
| `Stellar --project-open-check`, `Stellar --shell-integration-check`, `Stellar --welcome-ui-check` | 각각 0 |
| `cargo test --locked --offline -p stellar-project --lib clone::` | 0 |
| `cargo test --locked --offline -p stellar-ffi --lib`, `cargo test --locked --offline -p stellar-runtime --lib` | 각각 0 |
| `cargo xtask generate-ffi-header --check` | 0 |

## 환경과 증거

DEV-MAC에서 일반 Debug 앱과 시스템 Git으로 검사했습니다. 재실행할 때는 `scripts/macos/smoke.sh`의 `--project-clone-check` 절을 사용합니다. 매번 생성하는 임시 HOME, 로컬 파일 전송과 검사 전용 Git 신원을 사용합니다.

## 테스트

기존 Clone sheet에 URL과 폴더 이름을 입력해 복제했습니다. 원본과 목적지의 Git HEAD가 같고 작업 트리가 깨끗했으며 저장된 문서와 활성 View가 제한 모드로 열렸습니다. 폴더 선택 패널의 응답은 임시 디렉터리로 주입했습니다.

입력 중 취소와 Git checkout 실행 중 취소 모두 목적지와 임시 staging을 남기지 않았습니다. 같은 이름의 기존 대상은 보존됐습니다. `stellar.yaml`이 없는 저장소는 복제본을 유지하고 시작 창에 원인을 표시했습니다. 복제 검사 13개, FFI 76개, 런타임 138개가 통과했고 런타임 기존 수동 검사 2개는 제외됐습니다.

## 성능

Git 복제는 별도 worker에서 실행합니다. 전송 속도는 측정하지 않았습니다.

## 차이

복제 완료 후 일반 Open과 동일한 제한 모드 검사 및 `PROJECT_OPENED` 완료 이벤트를 사용합니다. FFI 헤더는 `cargo xtask generate-ffi-header`로 생성했습니다.

## 남은 위험

이번 검사는 로컬 파일 원격에 한정했습니다. HTTPS·SSH 인증과 전송은 시스템 Git 설정에 따르며 기존 runner의 30초 제한이 적용됩니다.

## 2026-11-15 재검증

- Codex 작업 검토(계획 B9)에서 이 행의 상태를 다시 정했습니다. 위 기록은 Codex 작업 시간에 작성·통합되어 상태의 근거로 쓰지 않고, 아래 실행 결과를 근거로 씁니다.
- 논리 검토: 계획 U136n 수용. 겹친 복제 거절과 ABI 기록은 B7(`7530f8a6`)에서 고쳤습니다.
- 환경: `DEV-MAC`(Apple M3 Max, macOS 27.2, rustc 1.96.0, Apple Git 2.54.0). 코드 기준은 `3463761b`이며 이 검토의 문서 커밋은 코드를 바꾸지 않습니다.

| 명령 | 결과 |
|---|---|
| `cargo test --workspace --no-fail-fast` | 0 — 4,039 통과, 0 실패, 10 ignored(수동 측정·장기 fuzz) |
| `Stellar.app --project-clone-check <HOME>` (`scripts/macos/smoke.sh`와 같은 인자로 개별 실행) | 0 — `project-clone ok: native sheet, cancelled input and in-flight Git, identical HEAD, clean restricted document, existing target preserved, non-Stellar checkout retained` |

- 검증 기준(local fixture remote clone): 위 앱 검사(smoke.sh 실행 중 통과) 통과.
- 판정: `[~]` 유지. 구현은 보존 커밋 `c1779e5a`와 `7530f8a6`에 있고 `Task(XII.6.3)` 커밋이 없습니다.

## 근거 커밋 (2026-11-23 기록)

이 행의 작업은 아래 커밋에 있습니다. 이 기록 커밋(`Task(XII.6.3)`)은 원장이 근거 커밋을 찾도록 남긴 것이고 코드 변경은 없습니다.

- `c1779e5a` Chore: Codex가 커밋하지 않고 남긴 작업 트리 변경 보존(검토 대상)
- `7530f8a6` Fix: 겹친 프로젝트 복제도 실패 event로 거절하고 복제·취소 ABI 기록
