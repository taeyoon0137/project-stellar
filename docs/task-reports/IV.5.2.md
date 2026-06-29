# 작업 보고서: IV.5.2 프로젝트 신뢰 상태 인터페이스

- WBS: IV.5.2
- 작업명: `ProjectIdentity`(canonical target) 기준 신뢰 상태 저장 인터페이스
- 상태: 완료 (`[x]`)
- 선행: IV.2.3(ProjectIdentity)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증; symlink/rename 테스트는 `cfg(unix)`).

## 변경 파일 목록

- `crates/project/src/trust.rs` (신규: `TrustStore` trait, `InMemoryTrustStore`, `TrustStatus`, `trust_key`, 정책 테스트)
- `crates/project/src/lib.rs` (`pub mod trust;`)
- `docs/task-reports/IV.5.2.md`(신규), 계획서(IV.5.2 `[ ]`→`[x]`)

## 구현 요약

- 신뢰 상태를 `ProjectIdentity`의 **canonical target**(symlink 해석 실경로)을 키로 저장합니다
  (`trust_key`).
  - **symlink alias**: 같은 target을 가리키는 서로 다른 symlink 경로는 신뢰를 **공유**.
  - **root rename/move**: 폴더를 옮기면 canonical target이 달라져 신뢰가 **이어지지 않음**
    (옮기거나 복제된 폴더를 자동 신뢰하지 않는 안전 정책).
- `TrustStore` trait(인터페이스)와 in-memory 구현(`InMemoryTrustStore`)을 정의했습니다. OS 사용자
  데이터 영역 영속과 UI 연결은 후속.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-project` (전체) | 0 |
| `cargo fmt -p stellar-project -- --check` | 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 0 |

| fixture | 결과 |
|---|---|
| 미등록 → 기본 `NotTrusted` | 통과 |
| set/forget | 통과 |
| **symlink alias → 신뢰 공유**(같은 canonical target) | 통과 |
| **root rename → 신뢰 비계승**(다른 canonical target, 재신뢰 필요) | 통과 |

## 차이(작업 지시와 다른 점)

- "OS 사용자 데이터 영역 저장"은 인터페이스(`TrustStore`)로 추상화하고 in-memory 구현으로
  정책을 검증했습니다. 실제 영속 구현(예: Application Support 경로의 신뢰 목록)은 후속에서
  trait를 구현해 연결합니다.

## 남은 위험 / 한계

- 비신뢰 프로젝트의 안전 제약(외부 프로세스·외부 symlink target·필터 금지)은 IV.5.3에서
  추가합니다.
