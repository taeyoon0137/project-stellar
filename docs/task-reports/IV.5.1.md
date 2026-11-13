# 작업 보고서: IV.5.1 ProjectProbe

- WBS: IV.5.1
- 작업명: 열기 시 root 검증·YAML parse·Git root 확인만 하는 `ProjectProbe`(문서 미독)
- 상태: 완료 (`[x]`)
- 선행: IV.2.2(root 검증), IV.1.7

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/probe.rs` (신규: `ProjectProbe`, `probe_project`, `ProbeError`, 테스트)
- `crates/project/src/lib.rs` (`pub mod probe;`)
- `docs/task-reports/IV.5.1.md`(신규), 계획서(IV.5.1 `[ ]`→`[x]`)

## 구현 요약

- `probe_project(path)`는 **가벼운 검사만** 수행합니다:
  1. `validate_project_root`(IV.2.2): `stellar.yaml`+`.git`이 같은 경로에 있는지(Git root 확인).
  2. `stellar.yaml` 읽기.
  3. `load_restricted`(IV.1.2): 제한 YAML parse.
  4. `Manifest::from_yaml`(IV.1.1): 매니페스트 스키마 검증.
- 성공 시 `ProjectProbe { root, git_link, manifest }`를 반환합니다. **문서 청크(`design/` 등)는
  읽지 않습니다.**
- 실패는 단계별로 `Root`/`ManifestRead`/`ManifestSyntax`/`ManifestSchema`로 구분합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-project` (전체) | 0 |
| `cargo fmt -p stellar-project -- --check` | 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 0 |

| fixture | 결과 |
|---|---|
| **정상**(유효 매니페스트+`.git`) → `Ok`, 문서 미독 | 통과 |
| **손상 YAML**(anchor) → `ManifestSyntax` | 통과 |
| 잘못된 스키마(필수 필드 누락) → `ManifestSchema` | 통과 |
| **Git 불일치**(매니페스트만, `.git` 없음) → `Root(MissingGit)` | 통과 |
| 매니페스트 누락 → `Root(MissingManifest)` | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. root·YAML·Git만 검사하고 문서 청크는 읽지 않습니다.

## 남은 위험 / 한계

- 신뢰 상태 저장은 IV.5.2, 비신뢰 probe의 안전 제약(외부 프로세스·symlink·필터 금지)은 IV.5.3에서
  추가합니다. 본 작업은 경량 probe에 한정합니다.

## 후속 수정(보안 리뷰 반영): `stellar.yaml` 읽기를 loader 상한에 묶음

- **문제**: `probe_project`가 `std::fs::read_to_string`으로 `stellar.yaml` 전체를 읽은 뒤에야
  loader의 `MAX_INPUT_BYTES`(1 MiB) 검사가 돌았습니다. runtime·FFI 프로젝트 열기 경로가 이 probe를
  셸에 노출하므로, 거대한(sparse 포함) 매니페스트를 가진 폴더를 열면 document thread가 GiB 단위로
  할당·읽기를 하며 뒤의 job을 막고, 할당 실패 시 release(`panic = abort`)에서 process가 끝날 수
  있었습니다.
- **수정**: 매니페스트는 연 handle의 크기가 상한을 넘으면 내용을 읽지 않고 `input-too-large`
  (`ManifestSyntax`, runtime `ManifestCorrupt`)로 거부합니다. 크기 확인 뒤 파일이 자라도
  `Read::take(상한 + 1)`로 읽기·할당량을 상한에 묶습니다. 상한 이하의 동작(UTF-8 오류 →
  `ManifestRead(InvalidData)`, loader 이후 단계)은 그대로입니다.
- **검증**: `oversized_manifest_is_rejected_before_reading_its_body`(상한 64배 sparse 파일 끝의
  비 UTF-8 바이트를 읽지 않음, 수정 전 `ManifestRead(InvalidData)`로 실패),
  `manifest_at_exact_limit_is_not_rejected_as_too_large`(경계값),
  `bounded_read_stops_on_an_endless_source`(끝없는 입력), runtime 통합 테스트
  `oversized_manifest_is_rejected_without_reading_its_body`(수정 전 `ManifestUnreadable`로 실패).
