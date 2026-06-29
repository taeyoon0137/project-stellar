# 작업 보고서: IV.6.4 clone 후 ProjectProbe 통과 요구

- WBS: IV.6.4
- 작업명: valid Stellar repo는 ProjectProbe 통과 후에만 성공(손상 매니페스트 실패)
- 상태: 완료 (`[x]`)
- 선행: IV.6.3(clone 분류), IV.5.1(probe)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 git, 로컬 검증; `cfg(unix)`).

## 변경 파일 목록

- `crates/project/src/clone.rs` (`clone_and_open` probe 통합, `ClonedProject::StellarProject{probe}`, `CloneError::Probe`, IV.6.4 테스트)
- `docs/task-reports/IV.6.4.md`(신규), 계획서(IV.6.4 `[ ]`→`[x]`)

## 구현 요약

- `clone_and_open`에서 `stellar.yaml`이 있으면 [`probe_project`](IV.5.1)를 실행해 **통과해야만**
  `ClonedProject::StellarProject { path, probe }`로 반환합니다.
- 손상 매니페스트(YAML 문법·스키마 오류)는 probe가 실패하므로 `CloneError::Probe`로 반환합니다.
- `NotAStellarProject`(stellar.yaml 없음, IV.6.3)는 그대로 보존 상태로 반환합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-project` (전체) | 0 |
| `cargo fmt -p stellar-project -- --check` | 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| **valid Stellar repo clone → `StellarProject`**(probe 통과, 매니페스트 확인) | 통과 |
| **손상 매니페스트(anchor) clone → `CloneError::Probe`** | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. valid는 probe 통과 후에만 성공, corrupt는 실패합니다.

## 남은 위험 / 한계

- Phase IV(프로젝트 매니페스트·생성·열기·Clone) 전체가 완료되었습니다. 후속 Phase V(결정론적
  청크 저장)로 이어집니다.
