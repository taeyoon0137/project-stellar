# 작업 보고서: XIII.4.1 symlink 허용 path resolver

- WBS: XIII.4.1
- 작업명: 파일·디렉터리 symlink를 그대로 허용하는 path resolver(내부·외부 target 모두 허용)
- 상태: 완료 (`[x]`)
- 선행: IV.2.3(ProjectIdentity)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증; symlink 생성 fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/project/src/path_resolver.rs` (신규: `PathResolver`, `ResolvedPath`, `SymlinkHop`,
  `TargetLocation`, `TargetKind`, `ResolveError`, `InvalidPathReason`, `split_project_path`, fixture 테스트)
- `crates/project/src/lib.rs` (`pub mod path_resolver;`)
- `docs/task-reports/XIII.4.1.md`(신규), 계획서(XIII.4.1 `[ ]`→`[x]`)

## 구현 요약

- `PathResolver`는 lexical root(symlink 보존)와 canonical root(실경로)를 가집니다.
  `PathResolver::new(root)` 또는 `PathResolver::from_identity(&ProjectIdentity)`(IV.2.3의 root path·
  canonical target 재사용)로 만듭니다.
- `resolve(project_path)`:
  - 프로젝트 상대 경로(`/` 구분)를 검증합니다. 빈 경로·절대 경로·`\`·빈 컴포넌트·`.`/`..`는
    `InvalidPath`로 거부합니다. `..`로 밖을 가리키는 요청은 금지하지만 **symlink target이 밖을
    가리키는 것은 허용**합니다.
  - lexical 체인의 각 컴포넌트를 `symlink_metadata`(대상 미추종)로 검사해 만난 symlink를
    `SymlinkHop { link(프로젝트 상대), raw_target(read_link 원문) }`으로 기록합니다.
  - `canonicalize`로 실제 target을 구하고, canonical root 안이면 `Internal`, 밖이면 `External`로
    분류합니다. 어느 쪽이든 거부하지 않습니다.
- `read(project_path)`는 파일 target(내부·외부)의 bytes를, `read_dir(project_path)`는 디렉터리
  target(디렉터리 symlink 포함)의 항목 이름을 정렬해 반환합니다.
- resolver는 링크를 수정·삭제하지 않고 외부 대상을 프로젝트로 복사하지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 127 passed, 0 failed; 신규 11개 포함) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |

| fixture | 결과 |
|---|---|
| 일반 파일(링크 없음) → `Internal`, 읽기 | 통과 |
| **내부 파일 링크**(상대 target `data/real.txt`) → `Internal`, hop 기록, 읽기 | 통과 |
| 내부 링크(`../real.txt`, 프로젝트 안에 머묾) → `Internal`, 읽기 | 통과 |
| **외부 파일 링크**(절대 target, 상대 `../outside/...` target) → `External`, 읽기 | 통과 |
| **내부 디렉터리 링크** → 링크 경유 파일 읽기·`read_dir` | 통과 |
| **외부 디렉터리 링크** → 링크 경유 파일 읽기·`read_dir`, `External` | 통과 |
| 읽기 후 링크 원문 target 불변·프로젝트에 복사본 없음·외부 원본 불변 | 통과 |
| alias(symlink) root로 연 identity → canonical root 기준 내부 판정 | 통과 |
| 잘못된 프로젝트 경로(빈·절대·`\`·`//`·끝 `/`·`.`·`..`) 거부 | 통과 |
| 없는 경로 → `Io{NotFound}`, 파일/디렉터리 종류 불일치 오류 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 내부·외부 파일/디렉터리 링크 읽기 fixture가 통과합니다.

## 남은 위험 / 한계

- cycle·최대 hop 검사는 아직 OS `canonicalize`의 ELOOP 오류에 의존하며 `Io`로만 보고됩니다.
  명시 hop 제한과 cycle 오류는 XIII.4.2에서 추가합니다.
- broken link는 현재 `Io{NotFound}`로 반환됩니다. diagnostic 표시(삭제 없음)는 XIII.4.3 소관입니다.
- 비신뢰 프로젝트의 외부 target 읽기 차단은 XIII.4.5에서 resolver 위에 정책으로 추가합니다(현재
  resolver는 신뢰 상태와 무관하게 허용).
- Windows symlink(권한·junction) fixture는 이 환경에서 실행하지 않았습니다(`cfg(unix)` 테스트만 실행).
