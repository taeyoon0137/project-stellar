# 작업 보고서: IV.2.3 ProjectIdentity

- WBS: IV.2.3
- 작업명: `ProjectIdentity`(project StableId + canonical root path + symlink dedup target)
- 상태: 완료 (`[x]`)
- 선행: III.1.1(`StableId`), IV.2.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증; symlink 테스트는 `cfg(unix)`).

## 변경 파일 목록

- `crates/project/src/identity.rs` (신규: `ProjectIdentity`, `parse_project_id`, 테스트)
- `crates/project/src/lib.rs` (`pub mod identity;`)
- `crates/project/Cargo.toml` (`stellar-core-types` 직접 의존 추가)
- `tools/xtask/src/crate_graph.rs` (project 허용 edge에 core-types 추가)
- `docs/architecture/crate-dependency-graph.md` (project 행에 core-types 추가)
- `docs/task-reports/IV.2.3.md`(신규), 계획서(IV.2.3 `[ ]`→`[x]`)

## 구현 요약

- `ProjectIdentity`는 project `StableId`와 **canonical root path**(절대·lexical, symlink 보존)의
  조합입니다. 별도로 **canonical target**(symlink 모두 해석한 실제 경로)을 저장해 중복 open을
  판정합니다.
- `new(project_id, opened_path)`: `root_path = std::path::absolute`(symlink 보존),
  `canonical_target = std::fs::canonicalize`(symlink 해석). `same_target`은 canonical target
  동일 여부로 중복 open을 판정합니다.
- `parse_project_id(uuid_text)`: 매니페스트 `project.id`(UUID `8-4-4-4-12`)를 16바이트
  `StableId`로 매핑합니다(형식 오류는 `None`). ProjectMeta.id 주석의 "StableId 매핑은 IV.2.3"을
  구현.
- project가 `StableId`를 쓰기 위해 **core-types 직접 의존**을 추가하고, 의존성 그래프(doc +
  xtask `crate_graph`)에 `project → core-types` edge를 반영했습니다(둘 다 비보호 경로).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-project` (73 unit) | 0 |
| `cargo fmt -p stellar-project -- --check` | 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 0 |
| `cargo clippy -p xtask --all-targets -- -D warnings` | 0 |
| `cargo run -p xtask -- validate-crate-graph` | 0 |
| `cargo test -p xtask crate_graph` | 0 |

| 검사 | 결과 |
|---|---|
| **같은 target의 두 symlink 경로 → 동일 canonical target**(중복 open) | 통과 |
| 서로 다른 target → 중복 아님 | 통과 |
| 열린 경로(root_path)는 symlink별로 다름 | 통과 |
| UUID→StableId 파싱(정상·형식 오류 거부) | 통과 |

## 차이(작업 지시와 다른 점)

- "canonical root path"는 절대·lexical(symlink 보존) 경로로, "canonical target"은 symlink를
  해석한 실제 경로로 구분 저장했습니다. 둘을 같게 두면 의미가 중복되므로 전자는 사용자가 연
  경로 형태, 후자는 중복 판정용 실경로로 해석했습니다.
- project → core-types 직접 의존을 추가했습니다(StableId 사용). 의존성 그래프 doc·validator를
  함께 갱신해 `validate-crate-graph` 통과를 확인했습니다.

## 남은 위험 / 한계

- `canonicalize`는 경로가 존재해야 성공합니다(IO 오류 전파). 존재하지 않는 경로 식별은 root
  검증(IV.2.2) 이후에 호출하는 흐름을 전제합니다.
- Windows의 symlink/junction 동작은 `cfg(unix)` 테스트로 검증하지 못했습니다. `canonicalize`
  의미는 플랫폼별로 다르며 Windows 검증은 후속 CI-WIN 단계에서 다룹니다.
