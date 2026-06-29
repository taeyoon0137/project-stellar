# 작업 보고서: IV.4.1 ProjectCreateRequest 사전 검증

- WBS: IV.4.1
- 작업명: `ProjectCreateRequest` 정의 + 사전 검증(존재 경로·금지 이름·권한)
- 상태: 완료 (`[x]`)
- 선행: IV.2.1(폴더 이름 검증), IV.3.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증; 권한 테스트는 `cfg(unix)`).

## 변경 파일 목록

- `crates/project/src/create.rs` (신규: `ProjectCreateRequest`, `validate_create_request`, `CreateValidationError`, 테스트)
- `crates/project/src/lib.rs` (`pub mod create;`)
- `docs/task-reports/IV.4.1.md`(신규), 계획서(IV.4.1 `[ ]`→`[x]`)

## 구현 요약

- `ProjectCreateRequest { parent_dir, folder_name, project_name }` + `target_path()`.
- `validate_create_request`는 **사전 검증만** 수행하고 프로젝트를 만들지 않습니다:
  - 프로젝트 이름 비어 있음 → `EmptyProjectName`.
  - 폴더 이름 규칙 위반(IV.2.1 `validate_folder_name`) → `InvalidFolderName`.
  - 부모가 디렉터리 아님 → `ParentNotADirectory`.
  - 대상 경로 이미 존재 → `TargetExists`.
  - 부모 쓰기 권한 없음 → `ParentNotWritable`(임시 probe 파일로 확인 후 즉시 삭제).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-project` (전체) | 0 |
| `cargo fmt -p stellar-project -- --check` | 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 0 |

| fixture | 결과 |
|---|---|
| 정상 요청 | `Ok` |
| **존재하는 대상 경로** | `TargetExists` |
| **금지 이름**(CON, `a/b`) | `InvalidFolderName` |
| 부모가 디렉터리 아님(없음·파일) | `ParentNotADirectory` |
| 빈 프로젝트 이름 | `EmptyProjectName` |
| **권한 없음**(0555 부모) | `ParentNotWritable`(쓰기 차단 시) |

## 차이(작업 지시와 다른 점)

- 쓰기 권한은 임시 probe 파일 생성/삭제로 확인합니다(메타데이터 권한 해석보다 신뢰성 높음).
  사전 검증 중 transient 파일이 생겼다 사라지는 부수효과가 있으나 실제 프로젝트는 만들지
  않습니다.
- 권한 테스트는 root 실행 시 0555가 우회되므로, 실제 쓰기 차단 여부를 직접 확인한 경우에만
  단언합니다.

## 남은 위험 / 한계

- 실제 폴더 생성·롤백(filesystem transaction)은 IV.4.2, scaffold 생성은 IV.4.3, git init은
  IV.4.4 이후에서 다룹니다. 본 작업은 사전 검증에 한정합니다.
- git 가용성 검사는 IV.4.1 검증 기준에 없어 포함하지 않았습니다(생성 단계 IV.4.4에서 git 사용).
