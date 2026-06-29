# 작업 보고서: IV.3.6 macOS xcode-select 설치 어댑터

- WBS: IV.3.6
- 작업명: 승인 시에만 `xcode-select --install` 시작(권한 상승·무승인 설치 금지)
- 상태: 완료 (`[x]`)
- 선행: IV.3.5(설치 상태), II.5.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증; 실제 설치는 트리거하지 않음).

## 변경 파일 목록

- `crates/git-porcelain/src/install_macos.rs` (신규: `request_install`, `XcodeSelectInstaller`, `RealXcodeSelectInstaller`(macOS), 승인 게이트 테스트)
- `crates/git-porcelain/src/lib.rs` (`pub mod install_macos;`)
- `docs/task-reports/IV.3.6.md`(신규), 계획서(IV.3.6 `[ ]`→`[x]`)

## 구현 요약

- `request_install(approval, installer)`: **승인된 경우에만** `installer.start_install()`을
  호출합니다. `NotApproved`이면 installer를 **호출하지 않고**(프로세스 생성 0) `NotApproved`를
  반환합니다.
- 프로세스 생성을 `XcodeSelectInstaller` trait로 주입해, 테스트에서 실제 설치를 트리거하지 않고
  승인 게이트를 검증합니다.
- `RealXcodeSelectInstaller`(macOS 전용)는 `xcode-select --install`을 **argv(shell 없음)**로
  실행하며 **sudo·권한 상승을 사용하지 않습니다**(설치 UI는 관리자 권한 불필요).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-git-porcelain` (전체) | 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | 0 |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| **승인 없음 → 설치 호출 0회**(spawn 0) | 통과 |
| **승인 시 → 설치 요청 1회 호출** | 통과 |
| 승인 시 spawn 오류 전파 | 통과 |

## 차이(작업 지시와 다른 점)

- 승인 게이트 로직은 플랫폼 독립(주입형 trait)으로 두고, 실제 `xcode-select` 실행기는 macOS
  전용 cfg로 분리했습니다. 테스트는 mock으로 실제 설치를 트리거하지 않습니다.
- 권한 상승 금지는 실행기에서 sudo·osascript-admin을 쓰지 않고 plain `Command`로 강제했습니다.

## 남은 위험 / 한계

- 실제 `RealXcodeSelectInstaller`는 단위 테스트로 검증하지 않습니다(실제 설치 UI 트리거 회피).
  동작 검증은 수동/통합 단계에서 수행합니다.
- Windows(winget) 어댑터는 IV.3.7, 설치 후 재탐지는 IV.3.8에서 추가합니다.
