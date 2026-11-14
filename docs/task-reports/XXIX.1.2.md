# 작업 보고서: XXIX.1.2 에이전트 도구 계층 외부 crate 승인과 의존성 검사 범위 확장

- WBS: XXIX.1.2
- 작업명: XXIV.4·XIX.9·XXIX에 필요한 외부 crate를 승인 목록에 등록(ADR-0020)하고 `validate-dependencies`를 workspace 전체 직접 의존으로 확장
- 상태: 완료 (`[x]`)
- 선행: 없음

## 필수 실행 환경

- `DEV-MAC` (순수 Rust xtask와 설정 파일, 로컬 검증).

## 변경 파일 목록

- `config/dependencies.yaml` (수정) — `serde`, `serde_json`, `libc`, `security-framework` approved 항목 추가(`planned_for` 포함), 머리 주석에 검사 범위와 선택 필드 설명.
- `docs/adr/ADR-0020-agent-tool-dependencies.md` (신규) — 승인 결정, JSON-RPC 프레이밍·표시 서식 해석 판단, 사용 범위, 검사 범위, `planned_for` 의미.
- `tools/xtask/src/dependencies.rs` (수정) — `planned_for` 필드 해석·검증, `PlanProgress`, `validate_dependencies_with`, `parse_workspace_direct_deps`(cargo tree 출력 해석), test 추가.
- `tools/xtask/src/commands.rs` (수정) — `validate-dependencies`가 workspace 전체 `cargo tree`와 계획서 진행 상태를 사용, 위반 메시지에 상세 내용 포함.
- `docs/task-reports/XXIX.1.2.md`(신규), 계획서(XXIX.1.2 `[ ]`→`[x]`).

## 구현 요약

- **승인 항목**(ADR-0020 Decision 1, 2026-11-14 crates.io 확인): `serde` 1.0.229, `serde_json` 1.0.154, `libc` 0.2.191, `security-framework` 3.7.0. 모두 MIT OR Apache-2.0이고 공개일이 2026-10-16 이전입니다. 아직 어떤 member도 쓰지 않으므로 도입 예정 행을 `planned_for`로 적었습니다.
- **JSON-RPC 프레이밍**은 별도 crate를 승인하지 않았습니다. MCP stdio transport가 줄바꿈 구분 JSON-RPC 메시지이므로 표준 라이브러리 줄 단위 읽기 + 크기 상한 + `serde_json`으로 충분합니다. **표시 서식 해석**은 서식 범위를 정하는 XXIX.4.3에서 필요할 때 추가하도록 했습니다(ADR-0020 Decision 2·3).
- **검사 범위 확장**: `cargo tree --workspace -e normal,build,dev --target all --depth 1 --prefix depth --no-dedupe` 출력을 `parse_workspace_direct_deps`로 해석합니다. 깊이 0 행(workspace member)과 이름·버전·출처가 같은 깊이 1 행은 내부 path 의존으로 제외하고, 나머지 이름을 정렬·중복 제거해 승인 목록과 비교합니다. `--no-dedupe`가 없으면 이미 출력한 member가 `(*)`로 접혀 그 member의 외부 의존이 빠지므로 필수입니다(실측). 깊이 접두어가 없거나 깊이 2 이상, 이름이 비정상인 행, member가 없는 출력은 `tree-format`으로 거부합니다.
  - 후속 수정(2026-11-14 리뷰 반영 `Fix:`): 위 명령에 `--all-features`를 더했습니다. 없으면 기본 feature로 켜지지 않는 optional 외부 의존(`foo = { optional = true }`와 `x = ["dep:foo"]`)이 출력에서 빠져 승인 검사를 건너뜁니다. 현재 명령은 `cargo tree --workspace --all-features -e normal,build,dev --target all --depth 1 --prefix depth --no-dedupe`이고, 인자는 `tools/xtask/src/dependencies.rs`의 `CARGO_TREE_ARGS`에 있습니다(ADR-0020 Decision 5). 회귀 test는 `direct_dependencies_include_optional_dependencies`입니다.
- **`planned_for`**: approved 항목에만 허용하고, 쉼표로 구분한 각 값은 WBS ID여야 합니다(`invalid-planned-for`, `planned-for-not-approved`). 명령은 계획서에서 행 존재와 `[x]` 상태를 읽어, 계획서에 없는 행(`unknown-planned-for`)과 도입 행이 모두 완료됐는데도 쓰이지 않는 승인(`stale-planned-approval`)을 거부합니다. `planned_for`가 없는 미사용 승인은 기존대로 `unused-approval`입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p xtask` | lib 153 passed, 0 failed(통합 test 2+2 passed), 종료 코드 0 |
| `cargo clippy -p xtask --all-targets -- -D warnings` | 경고 0, 종료 코드 0 |
| `cargo fmt -p xtask -- --check` | 통과, 종료 코드 0 |
| `cargo xtask validate-dependencies` (현재 workspace) | 종료 코드 0 |
| `cargo xtask validate-adr config/dependencies.yaml docs/adr/ADR-0020-agent-tool-dependencies.md tools/xtask/src/dependencies.rs` | 종료 코드 0 |
| 기준 커밋 `78a574a` xtask를 scratch 복사본에서 빌드하고 `crates/project`에 미등록 `hashbrown = "=0.16.1"` 직접 의존 추가 후 `validate-dependencies` | 이전 검사기 종료 코드 0(xtask 의존만 보므로 놓침) |
| 같은 scratch 복사본에 새 `tools/xtask/src`와 `config/dependencies.yaml`을 넣어 실행 | 종료 코드 65, `unknown-dependency` |

| 검사 | 결과 |
|---|---|
| 미등록 의존 추가 fixture 실패(`unregistered_dependency_in_any_member_fails`, `unknown_direct_dependency_fails`) | 통과 |
| workspace tree에서 외부 직접 의존만 추출(`workspace_tree_yields_external_direct_dependencies_only`), 형식 이상 거부(`unexpected_tree_format_fails_closed`) | 통과 |
| `planned_for` 통과·만료·오타 판정(`planned_approval_passes_until_its_rows_are_verified`, `malformed_planned_for_fails`) | 통과 |
| 현재 승인 목록과 계획서 통과(`real_dependencies_file_matches_real_plan`, `real_dependencies_file_parses`) | 통과 |

- 성능 작업이 아니며 자동 생성 파일 변경은 없습니다. crate 정보 확인에 `cargo info`와 crates.io 공개 API를 읽기 전용으로 사용했습니다(저장소 파일 변경 없음).

## 차이(작업 지시와 다른 점)

- 작업 지시가 예로 든 "JSON-RPC 프레이밍"과 "필요하면 표시 서식 해석"용 crate는 등록하지 않았습니다. 앞의 것은 승인한 `serde_json`과 표준 라이브러리로 충분하다고 판단했고, 뒤의 것은 서식 범위를 XXIX.4.3이 정하기 때문입니다. 판단 근거는 ADR-0020 Decision 2·3에 있습니다.
- 승인 목록에 선택 필드 `planned_for`를 추가했습니다. 기존 `unused-approval` 규칙 때문에, 도입 전 승인 항목을 표시할 방법이 없으면 "현재 workspace 통과"를 만족할 수 없었습니다.

## 남은 위험 / 한계

- `validate-dependencies`는 직접 의존만 검사합니다. 도입 작업은 추가되는 전이 의존(예: `serde_json`의 `itoa`·`memchr`·`ryu`, `security-framework`의 `core-foundation` 계열)의 라이선스를 `Cargo.lock` 기준으로 확인해야 합니다. 이 작업에서는 아직 manifest에 추가하지 않았으므로 전이 의존을 확인하지 않았습니다.
- ADR-0020 Decision 4의 사용 범위(어느 crate가 쓸 수 있는지)는 기계 검사하지 않습니다.
- 내부 path 의존 제외는 깊이 0 행과의 문자열 일치에 기대므로, cargo tree 출력 형식이 바뀌면 `tree-format` 오류나 내부 crate를 외부로 오인한 `unknown-dependency`로 실패합니다(통과 쪽으로는 틀리지 않음).
- 승인 버전은 2026-11-14 기준입니다. 도입 시점에 다른 버전을 쓰면 `version` 필드를 같은 변경에서 갱신해야 합니다.
