# 작업 보고서: XIII.4.5 비신뢰 프로젝트의 외부 symlink target 내용 차단

- WBS: XIII.4.5
- 작업명: 신뢰되지 않은 프로젝트에서는 외부 symlink target을 읽지 않고 target path metadata만
  제한적으로 표시
- 상태: 완료 (`[x]`)
- 선행: IV.5.3, XIII.4.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증; symlink·FIFO fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/project/src/path_resolver.rs` — `ExternalAccess`, `ExternalTargetInfo`,
  `ResolveError::ExternalTargetNotTrusted`, `WalkIssue::ExternalNotTraversed`,
  `PathResolver::for_project`·`with_trust`·`external_access`·`external_target_info`, 내용 접근 단일
  지점(`open_content`·`list_content`)과 테스트 전용 접근 기록, fixture 테스트.
- `docs/task-reports/XIII.4.5.md`(신규), 계획서(XIII.4.5 `[ ]`→`[x]`).

## 구현 요약

- **정책**: `ExternalAccess::from_trust(TrustStatus)` — 신뢰됨 `Allowed`, 비신뢰 `MetadataOnly`.
  열린 프로젝트의 resolver는 `PathResolver::for_project(identity, trust)`로 만듭니다.
  (`PathResolver::new`/`from_identity`는 XIII.4.1 동작 그대로 `Allowed`.)
- **내용 차단**: 파일 bytes 읽기와 디렉터리 목록 읽기는 `open_content`·`list_content` 두 함수로만
  일어나고, 그 직전에 `check_content_access`가 최종 target이 프로젝트 밖이면
  `ExternalTargetNotTrusted { path, target }`으로 거부합니다. 트리 순회는 외부 디렉터리를 항목으로만
  넣고 목록을 읽지 않으며 `ExternalNotTraversed`로 표시합니다. 외부 링크를 거쳐 다시 내부 파일로
  돌아오는 경로는 최종 target이 내부이므로 읽습니다.
- **제한 metadata**: `external_target_info`는 요청 경로, 위치, 종류, 해석된 target 경로, 첫 링크의
  원문 target만 돌려줍니다(내용·크기·시간 없음). 해석에는 `lstat`·`readlink`·`stat`·`realpath`만
  씁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 194 passed, 0 failed; fuzz 통합 4 passed) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **target content access 0**: 비신뢰 resolver의 외부 파일 `read`·외부 디렉터리 `read_dir`·외부 디렉터리 하위 파일 `read`·트리 순회 후 내용 접근 기록에 외부 경로 0건, 같은 동작을 신뢰 resolver로 하면 외부 접근이 기록됨(기록 장치 검증)(`untrusted_project_never_reads_external_target_content`) | 통과 |
| 비신뢰에서도 내부 파일·외부를 거쳐 내부로 돌아오는 링크는 읽힘, 외부 디렉터리는 항목만·하위 없음 | 통과 |
| 제한 metadata 표시(경로·위치·종류·target·원문 target), 조회 중 내용 접근 0(`untrusted_project_shows_limited_target_metadata`) | 통과 |
| **syscall 수준 확인**: 외부 FIFO target — 열면 막히는 FIFO에 대해 metadata 조회·읽기가 10초 안에 반환(열지 않음), 권한 `000` 외부 파일 — 권한 오류가 아닌 정책 거부(열기 시도 없음)(`untrusted_reads_do_not_open_fifo_or_unreadable_targets`) | 통과 |
| 신뢰 상태 → 정책 매핑(`trust_maps_to_external_access_policy`) | 통과 |

## 차이(작업 지시와 다른 점)

- "target content access syscall 없음"은 OS syscall 추적(dtrace·fs_usage — 권한 필요) 대신 (1) 내용
  접근 단일 지점의 테스트 전용 기록, (2) 열면 막히는 FIFO와 권한 `000` 파일로 열기 시도가 없음을
  관찰하는 방식으로 검증했습니다.

## 남은 위험 / 한계

- 정책은 resolver를 거치는 접근에만 적용됩니다. 다른 모듈이 `std::fs`로 프로젝트 경로를 직접 읽으면
  우회됩니다. 이후 문서·에셋 읽기 경로는 `PathResolver::for_project`를 통해야 합니다(XVI.7.1 capability
  matrix에서 함께 고정).
- 외부 target 감시(XIII.4.4)에서 비신뢰 프로젝트의 외부 target을 감시할지는 그 작업에서 정합니다.
- Windows symlink·junction fixture는 실행하지 않았습니다(ADR-0006, `cfg(unix)` 테스트만 실행).
