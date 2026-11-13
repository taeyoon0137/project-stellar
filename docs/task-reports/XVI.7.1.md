# 작업 보고서: XVI.7.1 비신뢰 모드 capability matrix

- WBS: XVI.7.1
- 작업명: untrusted mode capability matrix를 코드로 정의(문서 제한 읽기 허용, Git/terminal/future
  MCP/external filter/external symlink read 금지)
- 상태: 완료 (`[x]`)
- 선행: IV.5.3, XIII.4.5(이 lane에서 완료), XIV.1.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증; symlink fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/project/src/capability.rs` (신규: `ProjectCapability`, `CapabilityLevel`, `CapabilityRule`,
  `CAPABILITY_MATRIX`, `capability_level`, `check_capability`, `CapabilityDenied`, `git_trust`, 테스트)
- `crates/project/src/path_resolver.rs` (`ExternalAccess::from_trust`가 표의 외부 symlink 읽기 행을 사용)
- `crates/project/src/lib.rs` (`pub mod capability;` 추가)
- `docs/task-reports/XVI.7.1.md`(신규), 계획서(XVI.7.1 `[ ]`→`[x]`).

## 구현 요약

- 표 하나(`CAPABILITY_MATRIX`)가 6개 capability의 비신뢰·신뢰 수준을 정의합니다.

  | capability | 비신뢰 | 신뢰 |
  |---|---|---|
  | `read-document` | restricted(프로젝트 내부 내용만) | allowed |
  | `run-git` | denied | allowed |
  | `terminal` | denied | allowed |
  | `mcp` | denied | allowed |
  | `external-filter` | denied | allowed |
  | `read-external-symlink-target` | denied | allowed |

- 표에 없는 capability는 금지가 기본값입니다. `check_capability`는 금지면 `CapabilityDenied`를
  반환합니다.
- **차단 지점이 표를 사용**: Git 실행 gate 상태는 `git_trust(trust)`(표의 `run-git` 행)로,
  resolver의 외부 target 정책은 `ExternalAccess::from_trust`(표의 `read-external-symlink-target` 행)로
  얻습니다. 외부 filter·diff/merge driver는 시스템 Git만 실행하므로 Git gate가 막습니다. 터미널·MCP는
  아직 구현이 없어 표에만 정의되어 있으며, 구현 시 이 표를 사용합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 199 passed, 0 failed; fuzz 통합 4 passed) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **capability table golden**: 표 렌더링이 고정 텍스트와 일치(`capability_table_matches_golden`) | 통과 |
| 모든 capability가 정확히 한 행(`every_capability_has_exactly_one_rule`) | 통과 |
| 비신뢰에서 문서 읽기만 restricted, 나머지 denied·오류 메시지에 코드 포함, 신뢰 후 전부 allowed(`untrusted_allows_only_restricted_document_reads`) | 통과 |
| Git gate가 표를 따름: 비신뢰 `ProjectNotTrusted`·spawn 0, 신뢰 시 spawn 시도(`git_gate_follows_the_table`) | 통과 |
| resolver가 표를 따름: 내부 문서는 양쪽 다 읽힘, 외부 symlink 내용은 비신뢰에서 거부·신뢰에서 읽힘(`document_and_external_reads_follow_the_table`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 터미널·MCP는 로드맵 기능이라 표의 행과 검사 함수만 있고 실제 차단 지점은 해당 기능 구현
  때 연결합니다.

## 남은 위험 / 한계

- 표를 쓰지 않고 `std::fs`나 `GitRunner`를 직접 쓰는 새 코드는 정책을 우회할 수 있습니다. 새 실행
  경로는 `GatedGitRunner`·`PathResolver::for_project`를 거쳐야 합니다.
- 신뢰 상태 변경(grant/revoke) 후 이미 만든 gate·resolver를 갱신하는 것은 호출자 책임입니다
  (`GatedGitRunner::set_trust`, `PathResolver::with_trust`).
