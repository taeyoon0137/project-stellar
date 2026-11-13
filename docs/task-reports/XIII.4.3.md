# 작업 보고서: XIII.4.3 broken symlink diagnostic

- WBS: XIII.4.3
- 작업명: broken symlink는 삭제하지 않고 diagnostic으로 표시
- 상태: 완료 (`[x]`)
- 선행: XIII.4.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증; symlink 생성 fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/project/src/path_resolver.rs` — `ResolveError::BrokenSymlink`, 컴포넌트 출처(`Origin`)
  추적, `SymlinkDiagnostic`, `SymlinkDiagnosticKind`(고정 코드), `PathResolver::symlink_diagnostics`,
  fixture 테스트.
- `docs/task-reports/XIII.4.3.md`(신규), 계획서(XIII.4.3 `[ ]`→`[x]`).

## 구현 요약

- **깨진 링크 판정**: 해석 중 각 컴포넌트가 요청 경로에서 왔는지, 어느 hop의 원문 target에서
  왔는지 기록합니다. hop이 만든 컴포넌트가 없으면(`NotFound`·`NotADirectory`) 그 hop의 링크가 깨진
  것이므로 `BrokenSymlink { link, raw_target }`을 반환합니다. 체인 중간 링크가 깨지면 그 중간
  링크를 보고합니다. 링크가 없는 일반 경로의 부재와 정상 링크 아래의 없는 항목은 기존처럼
  `Io{NotFound}`입니다.
- **진단 표시**: `symlink_diagnostics(include)`는 트리 순회(XIII.4.2) 문제를 표시용
  `SymlinkDiagnostic { kind, path, link, target }`으로 모읍니다. 종류는 깨진 링크·순환·hop 초과·
  조상 디렉터리 순환이며 고정 코드(`symlink-broken` 등)를 가집니다. symlink와 무관한 I/O 문제는
  진단에 넣지 않습니다.
- **삭제 없음**: resolver·순회·진단 어느 경로도 링크를 삭제·수정하지 않습니다. 깨진 링크가 있어도
  프로젝트 probe(IV.5.1·IV.5.3), 다른 경로의 해석·읽기, 트리 순회는 그대로 동작합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 190 passed, 0 failed; fuzz 통합 4 passed) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |

| fixture | 결과 |
|---|---|
| 파일 링크·디렉터리 링크·파일을 디렉터리로 가리키는 링크·체인 중간 깨짐 → 정확한 깨진 링크와 원문 target 보고, 일반 부재는 `Io{NotFound}` 유지(`broken_links_are_reported_with_the_broken_hop`) | 통과 |
| **broken link가 있는 프로젝트 열기**: `probe_project`·`safe_probe` 성공, 다른 파일 읽기·트리 순회 정상, 내부·외부 깨진 링크와 순환이 진단으로 표시, 링크·디렉터리 목록 불변(`project_with_broken_links_opens_and_other_data_is_intact`) | 통과 |
| 진단 코드 고정, symlink 무관 I/O 문제 제외(`diagnostic_kinds_have_stable_codes`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 진단은 core 데이터(`SymlinkDiagnostic`)까지 제공하며, 셸 UI 표시 연결은 셸 작업에서
  합니다.

## 남은 위험 / 한계

- 진단은 호출 시점의 트리 순회 결과입니다. 링크가 나중에 깨지거나 복구되는 것은 파일 감시기
  (XIII.2·XIII.4.4) 이벤트로 다시 진단해야 합니다.
- Windows symlink·junction fixture는 실행하지 않았습니다(ADR-0006, `cfg(unix)` 테스트만 실행).
