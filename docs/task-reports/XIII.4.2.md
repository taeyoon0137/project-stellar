# 작업 보고서: XIII.4.2 symlink cycle·최대 hop 검사

- WBS: XIII.4.2
- 작업명: symlink cycle과 최대 hop을 검사해 무한 순회를 차단
- 상태: 완료 (`[x]`)
- 선행: XIII.4.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증; symlink 생성 fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/project/src/path_resolver.rs` — `MAX_SYMLINK_HOPS`, `ResolveError::SymlinkCycle`,
  `ResolveError::TooManySymlinkHops`, 상태 반복 검출, `PathResolver::walk_tree`, `TreeWalk`,
  `WalkEntry`, `WalkIssue`, fixture 테스트.
- `docs/task-reports/XIII.4.2.md`(신규), 계획서(XIII.4.2 `[ ]`→`[x]`).

## 구현 요약

- **해석(cycle·hop)**: resolver는 OS `canonicalize`의 ELOOP에 의존하지 않고 symlink를 직접
  따라갑니다. 링크를 만날 때마다 (링크 위치, 남은 컴포넌트) 상태를 기록하고, 같은 상태가 다시
  나오면 `SymlinkCycle { cycle }`(처음 반복된 링크부터 다시 만난 같은 링크까지, 예: self
  `[a, a]`, two-node `[a, b, a]`)을 반환합니다. 같은 링크를 **다른 남은 경로로** 다시 지나는 것은
  유한하므로 순환으로 보지 않습니다(`revisiting_a_link_with_a_different_suffix_is_not_a_cycle`).
- **최대 hop**: 경로 하나당 hop 상한 `MAX_SYMLINK_HOPS = 32`(macOS 커널 `MAXSYMLINKS`와 동일 —
  OS가 거부하는 체인을 받아들이지 않음). 상한에 이른 뒤 링크를 또 만나면
  `TooManySymlinkHops`입니다. 상한보다 긴 순환과 남은 경로가 계속 늘어나 상태가 반복되지 않는
  순환(`grow -> grow/x`)도 여기서 멈춥니다. 비순환 컴포넌트는 남은 목록을 줄이기만 하므로 반복
  횟수는 유한합니다.
- **트리 순회**: `walk_tree(include)`는 디렉터리 symlink(내부·외부)를 따라 내려가되 현재 경로의
  조상 실제 디렉터리를 다시 가리키는 디렉터리(`d/up -> ../..`, 외부 디렉터리 안에서 root로
  돌아오는 링크)는 들어가지 않고 `WalkIssue::DirectoryCycle`로 표시합니다. 해석할 수 없는 항목은
  `WalkIssue::Unresolvable`로 표시하고 순회를 계속합니다. 결과는 경로 순으로 정렬됩니다. 링크는
  수정·삭제하지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 187 passed, 0 failed; fuzz 통합 4 passed) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |

| fixture | 결과 |
|---|---|
| self cycle `self -> self` → `SymlinkCycle [self, self]`, `read`·`read_dir`도 같은 오류(`self_cycle_is_an_error`) | 통과 |
| two-node `a -> b -> a` → `SymlinkCycle [a, b, a]`, 순환을 거치는 하위 경로도 오류(`two_node_cycle_is_an_error`) | 통과 |
| deep cycle: 진입 체인 8개 뒤 내부·외부 링크로 이루어진 순환 → 순환 구간만 정확히 보고(`deep_cycle_behind_a_chain_is_an_error`) | 통과 |
| 정확히 32 hop 체인 허용·읽기, 33 hop·40개 순환·성장 순환 → `TooManySymlinkHops{limit: 32}`(`hop_limit_bounds_long_chains_and_long_cycles`) | 통과 |
| 같은 링크를 다른 남은 경로로 재방문 → 정상 해석(`revisiting_a_link_with_a_different_suffix_is_not_a_cycle`) | 통과 |
| 트리 순회: 조상 디렉터리 링크·자기 순환·외부에서 root로 돌아오는 링크가 있어도 종료, 문제 항목 표시, 링크 보존(`tree_walk_terminates_on_directory_and_link_cycles`) | 통과 |
| 트리 순회 include 필터(`tree_walk_respects_include_filter`) | 통과 |

## 차이(작업 지시와 다른 점)

- 작업 지시의 "무한 순회 차단"을 경로 해석과 트리 순회 두 곳 모두에 적용했습니다. 트리 순회
  (`walk_tree`)는 후속 작업(broken link diagnostic, watcher target 등록, rescan)이 같은 차단 규칙을
  쓰도록 이 작업에서 추가했습니다.
- hop 상한 32는 기술 아키텍처에 수치가 없어 macOS `MAXSYMLINKS`에 맞췄습니다.

## 남은 위험 / 한계

- 트리 순회는 항목마다 root부터 다시 해석하므로(정확성 우선) 매우 큰 트리에서는 비용이 큽니다.
  같은 실제 디렉터리를 여러 링크로 가리키는 DAG는 링크 수만큼 다시 방문합니다(유한).
- 해석과 실제 읽기 사이에 링크가 바뀌는 경쟁은 막지 않습니다(읽기는 해석된 실제 경로를 엽니다).
- Windows symlink·junction fixture는 실행하지 않았습니다(ADR-0006, `cfg(unix)` 테스트만 실행).
