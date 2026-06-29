# 작업 보고서: IV.3.8 설치 후 재탐지·재probe

- WBS: IV.3.8
- 작업명: 설치 종료 후 Git 재탐지·버전 재probe(앱 재시작 불요)
- 상태: 완료 (`[x]`)
- 선행: IV.3.6, IV.3.7

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 git, 로컬 검증).

## 변경 파일 목록

- `crates/git-porcelain/src/reprobe.rs` (신규: `reprobe_into_state`, `reprobe_with_filesystem`, 테스트)
- `crates/git-porcelain/src/lib.rs` (`pub mod reprobe;`)
- `docs/task-reports/IV.3.8.md`(신규), 계획서(IV.3.8 `[ ]`→`[x]`)

## 구현 요약

- `reprobe_into_state(state, probe, is_supported)`: probe 결과(주입)를 상태 전이 이벤트로
  바꿔 적용합니다(`Some(version)`+supported→Available, +unsupported→UnsupportedVersion,
  `None`→Missing). IV.3.1 탐지·IV.3.2 probe·IV.3.5 상태머신을 엮습니다.
- `reprobe_with_filesystem(state, config, is_supported)`: 실제 `discover_git`+`probe_git_version`
  으로 **in-process 재탐지·재probe**. **앱 재시작을 요구하지 않습니다**.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-git-porcelain` (38 unit) | 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | 0 |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| **mock install 후 재probe → Available 전환**(Missing→…→ReprobeRequired→Available) | 통과 |
| 재probe 미발견 → Missing | 통과 |
| 재probe 미지원 버전 → UnsupportedVersion | 통과 |
| 실제 파일시스템 재probe가 설치된 git을 Available로 전이 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 재탐지·재probe를 in-process로 수행해 앱 재시작을 요구하지 않습니다.

## 남은 위험 / 한계

- "capability probe"는 현재 버전 probe로 한정합니다(추가 capability 검사가 필요하면 동일 흐름에
  주입). IV.3(System Git porcelain Git executable) 섹션 전체가 완료되었습니다.
