# 작업 보고서: II.7.1 macOS 빌드 게이트(CI)

- WBS: II.7.1
- 작업명: macOS arm64 CI job에 Rust core, FFI header drift, Xcode Debug build 연결
- 상태: 구현 완료·CI 검증 대기 (`[~]`)
- 선행: II.5.4 (`[x]`)

## 필수 실행 환경

- `CI-MAC` (GitHub-hosted arm64 macOS 러너). "clean runner에서 성공"이 완료 기준이므로 로컬 빌드 성공만으로는 `[x]`로 올리지 않습니다.

## 변경 파일 목록

- `.github/workflows/bootstrap.yml` (`macos-shell` job 추가)
- `docs/task-reports/II.7.1.md`, 계획서(II.7.1 `[ ]`→`[~]`)

## 구현 요약

- `macos-shell` job(`runs-on: macos-latest`)에서 세 가지를 한 arm64 macOS 러너로 연결:
  1. Rust core: `cargo build -p stellar-ffi --locked`
  2. FFI header drift: `generate-ffi-header` 재생성 후 `git diff --exit-code`
  3. Xcode Debug build: `scripts/macos/smoke.sh`(Debug 빌드 + smoke/lifecycle/canvas)
- 기존 `bootstrap-macos`(Rust/xtask) job과 분리해, Xcode/SDK 문제로 실패해도 Rust 검사 신호가 섞이지 않게 함.
- 진단용 `Xcode and SDKs` 단계로 러너의 Xcode 버전·macOS SDK 목록을 출력.

## 실행한 명령 / 검증

| 검사 | 결과 |
|---|---|
| `scripts/macos/smoke.sh` (로컬 DEV-MAC, Xcode 27) | 종료 0 (smoke/lifecycle/canvas) |
| bootstrap.yml job 구조 파싱 | 3 job(bootstrap-macos, macos-shell, bootstrap-windows) 정상 |
| CI-MAC `macos-shell` job 통과 | **대기 중** (push 후 GitHub Actions 결과로 확정) |

## 남은 위험 / 한계

- deployment target이 macOS 26이라 러너의 Xcode/SDK가 macOS 26 SDK를 보유해야 Xcode 단계가 통과합니다. 현 GitHub-hosted 러너 이미지에 macOS 26 SDK가 없으면 `Xcode Debug build` 단계에서 실패할 수 있습니다.
- 실패 시 옵션: macOS 26 SDK 보유 러너(self-hosted 또는 신규 이미지) 사용, 또는 deployment target 재검토. CI 결과 확인 후 후속 조치.

## 작업 지시와 다른 점

- 단일 기존 job에 step을 추가하는 대신 전용 `macos-shell` job으로 분리(실패 격리). 세 요소(core/header/Xcode)는 한 job 안에서 연결되어 plan의 "연결" 요건을 충족합니다.
