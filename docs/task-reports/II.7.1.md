# 작업 보고서: II.7.1 macOS 빌드 게이트(CI)

- WBS: II.7.1
- 작업명: macOS arm64 CI job에 Rust core, FFI header drift, Xcode Debug build 연결
- 상태: 완료 (`[x]`) — CI-MAC 검증 완료
- 선행: II.5.4 (`[x]`)

## 필수 실행 환경

- `CI-MAC` (GitHub-hosted arm64 macOS 러너). 완료 기준 "clean runner에서 성공"을 GitHub Actions 결과로 확정.

## 변경 파일 목록

- `.github/workflows/bootstrap.yml` (`macos-shell` job 추가)
- `docs/task-reports/II.7.1.md`, 계획서(II.7.1 `[ ]`→`[~]`→`[x]`)

## 구현 요약

- `macos-shell` job(`runs-on: macos-latest`)에서 세 가지를 한 arm64 macOS 러너로 연결:
  1. Rust core: `cargo build -p stellar-ffi --locked`
  2. FFI header drift: `generate-ffi-header` 재생성 후 `git diff --exit-code`
  3. Xcode Debug build: `scripts/macos/smoke.sh`(Debug 빌드 + smoke/lifecycle/canvas)
- 기존 `bootstrap-macos`(Rust/xtask) job과 분리해 실패 신호가 섞이지 않게 함. 진단용 `Xcode and SDKs` 단계 포함.

## 실행한 명령 / 검증 (`CI-MAC`)

GitHub Actions run [28215584460](https://github.com/taeyoon0137/project-stellar/actions/runs/28215584460) (push, develop) — overall **success**.

| 검사 | 결과 |
|---|---|
| `macOS shell build (CI-MAC)` job conclusion | success |
| Rust core `cargo build -p stellar-ffi --locked` | 통과 |
| FFI header drift (`git diff --exit-code`) | 통과 |
| Xcode Debug build (`scripts/macos/smoke.sh`) | `** BUILD SUCCEEDED **` |
| smoke/lifecycle/canvas | `stellar core version: 0.0.0` / `lifecycle ok` / `canvas ok` / `smoke OK` |

러너 환경: Xcode 16.4, macOS SDK 15.5.

## 남은 경고 수 / 한계

- 경고 1건: `MACOSX_DEPLOYMENT_TARGET = 26.0`이 러너 SDK(15.5)의 지원 범위(10.13–15.5)를 초과한다는 deployment target 경고. 빌드는 SDK 15.5로 관대하게 성공했고 smoke도 통과했으나, **macOS 26 SDK 자체에 대한 검증은 아님**. 진정한 macOS 26 SDK 검증은 macOS 26 SDK 보유 러너가 필요. 현 게이트는 "빌드·링크·smoke 성공"을 보장.
- (그 외 benign: xcodebuild "multiple matching destinations", AppIntents metadata skip.)

## 작업 지시와 다른 점

- 단일 기존 job에 step을 추가하는 대신 전용 `macos-shell` job으로 분리(실패 격리). 세 요소(core/header/Xcode)는 한 job 안에서 연결되어 plan의 "연결" 요건을 충족합니다.
- 완료 기준은 "clean runner에서 성공"이며 CI에서 충족했습니다. deployment target과 러너 SDK의 간극은 위 한계에 기록.
