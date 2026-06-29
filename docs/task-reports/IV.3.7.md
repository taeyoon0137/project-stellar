# 작업 보고서: IV.3.7 Windows winget 설치 어댑터

- WBS: IV.3.7
- 작업명: 승인 시 `winget install Git.Git`(argv), WinGet 미존재 시 수동 설치 안내
- 상태: 완료 (`[x]`)
- 선행: IV.3.5(설치 상태), II.6.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로직·mock 검증). Windows 실제 실행기는 `cfg(windows)`로 분리(미실행).

## 변경 파일 목록

- `crates/git-porcelain/src/install_windows.rs` (신규: `request_install`, `WingetInstaller`, `RealWingetInstaller`(windows), 승인·winget 게이트 테스트)
- `crates/git-porcelain/src/lib.rs` (`pub mod install_windows;`)
- `docs/task-reports/IV.3.7.md`(신규), 계획서(IV.3.7 `[ ]`→`[x]`)

## 구현 요약

- `request_install(approval, installer)`:
  - `NotApproved` → winget 확인·설치 없이 `NotApproved`(프로세스 생성 0).
  - 승인 + WinGet 없음 → 설치 시도 없이 **`WingetMissing`(수동 설치 안내)**.
  - 승인 + WinGet 존재 → `start_git_install()` 호출 → `Started`.
- `RealWingetInstaller`(Windows 전용)는 `winget install --id Git.Git -e --source winget`을
  **argv(shell 없음)**로 실행하고, `winget --version`으로 가용성을 확인합니다. 권한 상승 없음.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-git-porcelain` (전체) | 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | 0 |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 0 |

| fixture | 결과 |
|---|---|
| **미승인** → 아무 동작 없음(설치 호출 0) | 통과 |
| **승인 + winget 존재** → 설치 1회 호출(`Started`) | 통과 |
| **승인 + winget 없음** → `WingetMissing`(설치 호출 0) | 통과 |
| 승인 시 설치 spawn 오류 전파 | 통과 |

## 차이(작업 지시와 다른 점)

- macOS 어댑터(IV.3.6)와 같은 주입 패턴을 씁니다. 각 플랫폼 어댑터는 독립 모듈이라 승인 enum이
  소폭 중복되지만 모듈 독립성을 우선했습니다.
- 실제 `winget` 실행기는 Windows 전용 cfg이며 DEV-MAC에서는 빌드·검증하지 않습니다(로직·mock은
  cross-platform 검증).

## 남은 위험 / 한계

- `RealWingetInstaller`는 단위 테스트로 검증하지 않습니다(실제 설치 회피). Windows 통합 검증은
  CI-WIN/LAB-WIN 단계에서 수행합니다.
- 설치 후 재탐지·재probe는 IV.3.8에서 추가합니다.
