# 작업 보고서: XIX.9.3 headless 읽기 backend

- WBS: XIX.9.3
- 작업명: `stellar-mcp-core`에 headless 읽기 backend 구현(고정 루트 하나, untrusted 전 tool 거부, 읽기·검증·dry-run tool만)
- 상태: 구현 완료, 원장 판정 보류 (`[~]`) — 선행 XIX.9.2(`[~]`, XIX.9.1 경유)와 XXIV.4.3(`[~]`)·XIX.3.3(`[ ]`)이 `[x]`가 아니라서 `[x]`로 올리지 않습니다. 행 범위는 모두 구현했습니다.
- 선행: XIX.9.2(`[~]`), XXIV.4.3(ledger-verification 재판정 `[x]`, 원장 미반영), XVI.7.2 `[x]`, XIX.3.3 `[ ]`

## 필수 실행 환경

- `DEV-MAC` (시스템 Git 필요).

## 변경 파일 목록

- `crates/mcp-core/src/headless.rs` — 목록을 agent-protocol 정의(`Availability::All`)에서 생성, tool dispatch, 실패 종류(`data.kind`), scope 실패 분류, 신뢰 실패 종류.
- `crates/mcp-core/src/headless/operations.rs`(신규) — CLI 동등성 tool 7개.
- `crates/mcp-core/src/headless/resources_extra.rs`(신규) — `tokens`·`target-profiles`·`git-status` resource.
- `crates/mcp-core/src/headless/comparison.rs` — diff JSON helper를 같은 backend 안에서 재사용하도록 `pub(super)`.
- `crates/ops/src/mcp_merge.rs`(신규) — 읽기 전용 `MergeGit`(gate를 거친 실행기)와 `semantic_conflicts(root)`.
- `crates/ops/src/lib.rs` — 모듈 등록 한 줄.
- `crates/agent-protocol/src/catalog/tools.rs` — CSS 선언 블록 상한을 CLI와 같은 16 KiB로.
- `tools/stellar-mcp/tests/support/mod.rs`(신규), `tools/stellar-mcp/tests/headless_tools.rs`(신규), `tools/stellar-mcp/tests/headless_read.rs`(목록 기대값), `tools/stellar-mcp/tests/fixtures/catalog.json`.

## 구현 요약

| tool·resource | 부르는 ops 함수 | CLI |
|---|---|---|
| `document.propose_commands` | `apply::apply(.., DryRun, LockPolicy::None)` | `stellar apply --dry-run` |
| `document.copy_css` | `css::copy` | `stellar css copy` |
| `document.propose_css_paste` | `css::paste(.., DryRun, LockPolicy::None)` | `stellar css paste --dry-run` |
| `document.export_web_frame` | `web_frame::export_web_frame_node`(세션 snapshot) | `stellar export-web [--frame]` |
| `document.source_span` | `text_source::source_span(<고정 루트>/design, ..)` | `stellar query <design> --source-span` |
| `git.status`, `stellar://git/status` | `git::GitSession::open_for_os_user` + `status` | `stellar git status` |
| `git.semantic_conflicts` | `mcp_merge::semantic_conflicts` → `candidate::session::ConflictSession::open` | `stellar merge-driver resolve`(`--apply` 없음) |
| `stellar://tokens` | `query::tokens` | `stellar query . tokens` |
| `stellar://target-profiles` | snapshot manifest | `stellar inspect` |

- 기존 `document.semantic_diff`·`document.validate`와 구조·문서·컴포넌트 resource는 그대로입니다.
- 목록: `tools/list`·`resources/list`·`resources/templates/list`는 정의에서 headless 제공 항목만 고릅니다. 수기 목록을 지웠습니다. 쓰기 tool은 목록에 없고, 이름으로 불러도 `unknown-tool`입니다.
- dry-run은 `base_revision`이 세션 snapshot revision과 다르면 ops를 부르기 전에 `stale`로 거부하고, ops 결과 revision도 다시 비교합니다.
- Git 읽기는 신뢰 확인한 루트에서만, `GatedGitRunner`(trusted만 실행)로만 합니다. `mcp_merge`의 포트는 `add`·`set_local_config`를 실행하지 않고 실패합니다.
- 실패는 고정 문구와 `data.kind`(ADR-0085 결정 7의 이름)를 갖습니다. 결과 envelope과 종료 코드 대응은 XIX.9.6·XIX.9.9에서 마무리합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-mcp --test headless_tools` | 4 passed |
| `cargo test -p stellar-mcp -p stellar-mcp-core -p stellar-agent-protocol -p stellar-ops` | 실패 0 |
| `cargo clippy -p stellar-mcp -p stellar-mcp-core -p stellar-agent-protocol -p stellar-ops --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| untrusted fixture 전 tool 거부: 신뢰 등록 없으면 시작하지 않음(종료 77), 세션 중 철회 뒤 9개 tool 모두 `not-trusted`, resource·`tools/list`도 오류(`untrusted_project_rejects_every_tool_and_never_starts`) | 통과 |
| 다른 경로 요청 거부: 9개 tool에 `project`·`path`·`root`·`design`·`base_path` 인자를 더하면 모두 `invalid-arguments`, 다른 프로젝트 문서 ID는 `out-of-scope`(`write_tools_are_unknown_and_path_arguments_are_rejected`) | 통과 |
| headless tools/list의 쓰기 tool 0: 목록 9개 모두 `readOnlyHint: true`, 쓰기 tool 8개 이름은 `unknown-tool`(같은 test, `approved_snapshot_lists_only_implemented_resources_and_pages_without_writes`) | 통과 |
| 실행 전후 프로젝트 파일 hash 동일: dry-run 두 종류와 Git status·conflict 읽기를 포함한 전 tool 실행 전후 임시 루트 전체(작업 트리·`.git`·신뢰 저장소·HOME) digest 동일(`every_listed_tool_runs_read_only_through_ops`) | 통과 |
| stale `base_revision`과 closed enum 밖 종류 거부(`stale_base_revision_is_rejected_before_dry_run`) | 통과 |

## 차이(작업 지시와 다른 점)

- 행은 "`stellar-ops`의 읽기·검증·dry-run tool"을 노출하라고 합니다. 1차 배치 CLI 기능(css·git·export-web·source-span·merge 결과)의 읽기 짝을 ADR-0085 결정 8대로 함께 노출했습니다. 쓰기 짝은 attached 전용 정의만 있습니다.
- `stellar merge-driver resolve`의 시스템 Git adapter는 CLI 소유라 그대로 두고, ops에 gate를 거친 읽기 전용 포트를 새로 두었습니다(ADR-0081 결정 7이 예고한 `GatedGitRunner` 기반 구현의 읽기 부분).

## 남은 위험 / 한계

- `git.semantic_conflicts`는 진행 중 merge가 없는 fixture로만 실행 확인했습니다. 실제 컴포넌트 원본 conflict가 있는 저장소로 MCP 경로를 검증하는 test는 없습니다(같은 `ConflictSession`은 ops·CLI test가 검증).
- `document.source_span`은 고정 루트의 `design` 폴더만 봅니다. 텍스트 원본이 없는 fixture에서 `unavailable`만 확인했습니다.
- `document.export_web_frame`은 오류 진단이 있으면 `exported: false`·`html: null`인 정상 결과입니다. CLI는 같은 경우 종료 1입니다.
