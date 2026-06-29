# 작업 보고서: IV.6.3 clone 후 NotAStellarProject 상태

- WBS: IV.6.3
- 작업명: clone 결과에 stellar.yaml 없으면 거부·보존(상태 반환), 자동 변환 금지
- 상태: 완료 (`[x]`)
- 선행: IV.6.2(clone), IV.5.1(probe)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 git, 로컬 검증; `cfg(unix)`).

## 변경 파일 목록

- `crates/project/src/clone.rs` (`clone_and_open`, `ClonedProject`, IV.6.3 테스트)
- `docs/task-reports/IV.6.3.md`(신규), 계획서(IV.6.3 `[ ]`→`[x]`)

## 구현 요약

- `clone_and_open(request, runner)`: clone(IV.6.2) 후 `stellar.yaml` 존재를 확인합니다.
  - 없으면 `ClonedProject::NotAStellarProject { path }`를 반환합니다. clone 결과는 **보존**되며
    (자동 삭제·변환 없음), 보존·삭제 선택은 호출자 몫입니다.
  - 있으면 `ClonedProject::StellarProject { path }` 후보로 반환합니다(매니페스트 검증은 IV.6.4).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-project` (전체) | 0 |
| `cargo fmt -p stellar-project -- --check` | 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| **일반 repo(stellar.yaml 없음) clone → `NotAStellarProject`** | 통과 |
| clone 결과 보존(경로·파일 유지, stellar.yaml 미생성) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 자동 변환·삭제 없이 상태로 반환합니다.

## 남은 위험 / 한계

- `stellar.yaml`이 있는 경우의 `ProjectProbe` 통과 요구·손상 매니페스트 거부는 IV.6.4에서
  추가합니다.
