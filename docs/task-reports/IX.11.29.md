# 작업 보고서: IX.11.29 CLI `stellar style-defaults list|show|set`

- WBS: IX.11.29
- 작업명: CLI `stellar style-defaults`의 `list`(내장·사용자 세트 목록), `show`(평탄화한 세트 표), `set`(활성 세트 변경)
- 상태: 부분 완료 (`[~]`) — 2026-11-24 통합 판정: 이 행의 구현은 끝났지만 선행 IX.11.26, IX.11.27, XXIV.4.5이 `[x]`가 아닙니다. 그 조건이 풀리면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]`) — 완료 기준 세 가지를 확인했습니다. `set`이 `ops::apply`를 거치지 않는 차이와 그 이유는 아래
  "차이"에 있습니다. 선행 IX.11.26·IX.11.27·XXIV.4.5의 표기는 통합 때 선행 판정을 따릅니다.
- 결정: ADR-0115 결정 4(신규), ADR-0087 결정 2, ADR-0086 결정 11
- 선행: IX.11.26, IX.11.27, XXIV.4.5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `tools/stellar-cli/src/style_defaults.rs`
  - `set <경로> <세트> [--base <revision>] [--dry-run]`: 교차 프로세스 저장 잠금 → 다시 열기 → base revision 검사 →
    `prepare_selection(root, 연 시점 이름, 세트)` → `commit`. 결과에 `mode`·`base_revision`·`revision`을 더했습니다.
  - `show`: layout 표(`values`)에 더해 payload longhand 표(`payload`, registry 항목)를 냅니다.
  - `resolve`: `ops::query::node_layout` 결과를 그대로 냅니다(CLI 자체 해석 제거).
- `tools/stellar-cli/src/cli.rs`, `tools/stellar-cli/tests/fixtures/help.txt`: 도움말 한 줄.
- `tools/stellar-cli/tests/style_defaults.rs`: 통합 test 2개 추가, 사용법 오류 test 보강.
- `docs/spec/style-defaults.md` 3장·5장: `set` 옵션과 `resolve`·`query node`가 같은 결과라는 설명.

## 구현 요약

- `list`·`show`는 기존 경로이고, `show`에 payload 표가 더해졌습니다.
- `set`은 ops 쓰기 경로와 같은 순서(잠금을 기다리지 않고 잡음 → 다시 열기 → base revision 비교)를 지킵니다. base가
  다르면 `base-revision-mismatch`(exit 65)이고 아무 파일도 쓰지 않습니다. `--base`가 없으면 지금 revision을 씁니다.
- 쓰기는 ADR-0087 결정 2의 두 단계 쓰기입니다. 준비가 세트 평탄화(없는 세트 `unknown-style-defaults`, 순환
  `style-defaults-cycle` 등)와 연 뒤 선택 변경(`style-defaults-stale`)을 거부하고, commit은 읽은 bytes hash가 그대로일
  때만 씁니다. 바꿀 것이 없으면 쓰지 않습니다.
- 문서 레코드는 바뀌지 않으므로 generation과 revision은 그대로입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-cli --test style_defaults` | 통과(5). 목록·표(payload 포함)·`set` 뒤 다시 열기, base 불일치 거부와 파일 불변, dry-run 불변, 바뀐 파일이 `stellar.yaml` 하나·한 줄, 되돌리면 원래 bytes, `resolve`와 `query node`의 `layout.values` 동일 |
| `cargo test -p stellar-cli --test headless_help` | 통과(6). 도움말 golden |
| `cargo test -p stellar-cli --test layout_provenance` | 통과(1) |

## 완료 기준 대비

| 기준 | 상태 |
|---|---|
| CLI 통합 test(목록·표·`set` 뒤 재열기) | `list_and_show_include_user_sets_and_reject_cycles`, `show_lists_payload_longhands_from_the_registry_table`, `set_goes_through_the_ops_write_path_and_changes_one_manifest_line` |
| base revision 불일치 거부 | 같은 test: `--base g0`이면 exit 65 `base-revision-mismatch`, 프로젝트 파일 전부 그대로 |
| 도움말 golden | `headless_help` |

## 차이(작업 지시와 다른 점)

- 행 설명은 "`set`(`SetStyleDefaults` 적용)은 ops 쓰기 경로를 거친다"입니다. `set`은 `ops::apply`에 canonical
  `SetStyleDefaults`를 넣지 않고, 그 command의 영속 결과(`stellar.yaml` 한 줄)를 같은 두 단계 쓰기 API로 씁니다.
  - 이유: `ops::apply`는 문서 레코드가 그대로인 transaction에도 generation을 새로 씁니다. 직접 확인한 결과
    `design/current`와 새 `design/generations/*.starmanifest`가 바뀌어 Git diff가 세 파일이 됩니다. ADR-0082 결정 4의
    "Git diff에서 한 줄"에 어긋납니다.
  - `ops::apply`는 이 작업 소유 밖이라 고치지 않았습니다. mirror만 바뀐 transaction의 generation 저장을 건너뛰게 고치면
    `set`을 `ops::apply` 하나로 바꿀 수 있고 결과 bytes는 같습니다(ADR-0115 결정 4).
- base revision은 generation revision입니다. `stellar.yaml`만 바뀌는 `set` 뒤에도 revision은 그대로이고, 동시에 다른
  writer가 선택을 바꾼 경우는 준비 단계의 연 시점 이름 확인과 commit의 hash 확인이 막습니다.

## 남은 위험 / 한계

- 앱·`stellar apply`로 `SetStyleDefaults`를 적용하면 여전히 generation 파일 두 개가 함께 바뀝니다(`ops::apply` 소유 작업).
- `show`의 payload 표는 내장 세트 값입니다. 사용자 세트는 payload longhand를 두지 않습니다(ADR-0115 결정 1).

## 후속 작업 (2026-11-24, `lane/style-apply-revision`, ADR-0128)

- `set`을 `stellar_ops::apply` 하나로 옮겼습니다. 위 "차이"에 적은 두 단계 쓰기 별도 경로가 없어졌습니다.
  - `ops::apply`는 결과 문서 레코드와 Component Context가 적용 전과 같으면 원본·cache를 쓰지 않습니다(ADR-0128 결정 1).
    canonical `SetStyleDefaults` 하나만 담은 transaction의 영속 결과는 `stellar.yaml` 한 줄입니다. `stellar apply`·MCP로
    세트만 바꿔도 같습니다.
  - CLI `set`은 canonical `SetStyleDefaults` 하나를 root 문서 순서의 첫 문서에 적용합니다. 잠금·다시 열기·base 검사·세트
    준비·쓰기 순서는 `stellar apply`와 같습니다. 저장된 문서가 없으면 `no-document`입니다(결정 3).
  - 세트 파일 오류 code(`unknown-style-defaults` 등)는 적용 전에 같은 판정 함수(`flatten_named`)로 알립니다.
- 프로젝트 revision에 `stellar.yaml` 세트 선택을 넣었습니다(ADR-0128 결정 2,
  `ProjectRevision::with_style_defaults`). 세트만 바꾸는 쓰기도 revision을 바꾸므로, 다른 writer의 세트 변경을 모든 ops
  쓰기의 `--base` 비교가 잡습니다. 기본 세트 `css` 프로젝트의 표기는 그대로입니다.
  - 그래서 `set` 결과의 `revision`은 이제 base와 다릅니다(generation 번호는 같음). 위 "차이"의 마지막 항목("`set` 뒤에도
    revision은 그대로")은 이 결정으로 바뀌었습니다.

### 변경 파일

- `crates/ops/src/apply.rs`: 문서가 그대로인 transaction의 저장 생략, 새 세트 선택으로 결과 revision 계산. 단위 test 4개.
- `crates/ops/src/revision.rs`: `with_style_defaults`, `g0:<hash>` 해석. 단위 test.
- `crates/ops/src/snapshot.rs`, `crates/ops/src/snapshot/save.rs`: `revision`(세트 선택 포함)·`storage_revision`,
  `save_documents_selecting`. 단위 test.
- `tools/stellar-cli/src/style_defaults.rs`: `set`을 `ops::apply`로.
- `tools/stellar-cli/tests/style_defaults.rs`: `set` 뒤 revision이 바뀌고 옛 base가 거부되는 검사로 고침(소유 밖, 동작
  변경에 따른 수정).
- `tools/stellar-cli/tests/fixtures/results/semantic-merge-{clean,conflict}.json`: fixture의 이미 잠긴 node에 대한
  `set-locked` apply가 generation을 쓰지 않아 theirs revision이 바뀜(소유 밖, `STELLAR_CLI_BLESS`로 갱신, revision 필드만
  바뀜).

### 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops --lib` | 통과(163) |
| `cargo test -p stellar-cli --test style_defaults` | 통과(5). 바뀐 파일 `stellar.yaml` 하나·한 줄, 옛 base 거부, 새 revision으로 되돌리면 bytes·revision 원래대로 |
| `cargo test -p stellar-cli --test output_fixtures` | 통과(golden 2개 갱신 뒤) |

### 남은 위험 / 한계

- runtime `agent_edit::project_revision`은 세트 선택을 더하지 않습니다(소유 밖). 세트가 `css`가 아닌 프로젝트에서
  창 revision과 headless revision 표기가 다릅니다. 활성 세트 거울 연결 작업이 `with_style_defaults`를 불러야 합니다.
- `ops::migrate` 결과 revision도 세트 선택을 더하지 않습니다(소유 밖). 다시 열어 얻은 revision은 맞습니다.
- `docs/spec/style-defaults.md` 3장은 ADR-0115 기준 설명입니다(결과는 같음, 소유 밖).
- 행 상태는 선행 IX.11.26·IX.11.27·XXIV.4.5 판정을 따릅니다.

## 검토 보완 (2026-11-24, `lane/style-apply-revision` 검토)

- revision에 세트 선택을 넣은 뒤 같은 뜻을 따르지 않던 두 곳을 맞췄습니다(ADR-0128 결정 2 보완).
  - `ops::migrate` 결과 revision에 세트 선택을 더했습니다. 세트가 `css`가 아닌 프로젝트에서도 migrate 결과가 다시 연
    revision과 같아 바로 다음 쓰기의 base로 쓸 수 있습니다. 위 "남은 위험"의 migrate 항목은 해소됐습니다.
  - `compare`의 HEAD 재사용 판정은 `storage_revision`의 hash를 HEAD manifest hash와 대조합니다. 세트를 고른 프로젝트가
    빠른 경로에서 빠지던 성능 회귀를 없앴습니다(결과는 전과 같음).
- CLI `set`은 없는 세트 같은 세트 파일 오류를 문서 확인보다 먼저 알립니다(빈 프로젝트에서 `unknown-style-defaults`).
- 텍스트 원본 프로젝트에서 세트만 바꾸는 쓰기 test를 더했습니다. 원본·cache 짝(Cache)과 원본만 바뀐 상태(FromText)
  모두 `stellar.yaml`만 바뀌고, 연 방식과 다시 연 revision이 쓰기 결과와 같습니다.

### 변경 파일

- `crates/ops/src/migrate.rs`, `crates/ops/src/compare.rs`(소유 밖, 이 작업이 바꾼 revision 뜻에 따른 한 줄씩).
- `crates/ops/src/snapshot/text_tests.rs`, `tools/stellar-cli/src/style_defaults.rs`,
  `tools/stellar-cli/tests/style_defaults.rs`.

### 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops --lib` | 통과(165) |
| `cargo test -p stellar-cli --test style_defaults` | 통과(6) |

## 후속 작업 (2026-11-24, `lane/style-revision-2`, ADR-0135)

- ADR-0128 Consequences의 한계 세 가지와 revision 표기 계약을 정리했습니다. 행 범위는 그대로입니다.
  - revision hash 이름: `ProjectRevision::Saved`의 필드·메서드와 CLI·MCP JSON revision object의 키를 `manifest_hash`에서
    `hash`로 바꿨습니다(결정 1). 원본에서 읽은 상태와 세트를 고른 프로젝트에서는 manifest hash가 아니기 때문입니다.
  - 사용자 세트 파일: 사용자 세트를 고르면 `flatten_named`로 푼 사슬과 값의 hash를 revision에 더합니다
    (`revision::StyleDefaultsKey`, 결정 2). 세트 파일 편집 뒤 낡은 base 쓰기는 `base-revision-mismatch`입니다. 뜻이 같은
    편집과 사슬 밖 세트 파일은 revision을 바꾸지 않습니다. MCP snapshot 재사용 판정도 세트 파일 변경을 봅니다.
  - 창 revision: runtime `agent_edit::project_revision`이 같은 디스크 상태의 headless revision과 같은 값을 냅니다
    (결정 3). 세트 선택과 사용자 세트 내용을 더하고, 원본이 cache보다 앞선 동안은 같은 `text_revision`입니다.
  - schema: MCP resource revision과 CommandTransaction `base_revision` pattern이 `g0:<hash>`를 받습니다(결정 4).
- 위 "남은 위험"의 runtime `project_revision` 항목은 해소됐습니다.

### 변경 파일

- 소유: `crates/ops/src/revision.rs`, `crates/ops/src/snapshot.rs`, `crates/ops/src/snapshot/save.rs`,
  `crates/ops/src/snapshot/text_tests.rs`, `crates/runtime/src/agent_edit.rs`, `crates/runtime/src/agent_edit_tests.rs`,
  `crates/mcp-core/src/headless.rs`.
- 소유 밖(이름 변경·signature 변경에 따른 최소 수정): `crates/ops/src/apply.rs`, `crates/ops/src/migrate.rs`,
  `crates/ops/src/compare.rs`, `crates/agent-edit/src/attached/edit_tests.rs`, `tools/stellar-cli/src/views.rs`,
  `crates/runtime/src/window_session.rs`(`saved_style_defaults` 접근자 하나), `crates/agent-protocol/src/catalog/resources.rs`,
  `tools/xtask/src/command_schema.rs`, `schemas/command-transaction.schema.json`(생성), CLI 결과 fixture 25개와 MCP
  fixture 2개(`STELLAR_CLI_BLESS`·`STELLAR_MCP_BLESS`로 갱신, 키 이름과 pattern만 바뀜을 스크립트로 대조).
- `docs/adr/ADR-0135-project-revision-hash-name-user-sets-and-window-parity.md`.

### 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops --lib` | 통과(183) |
| `cargo test -p stellar-runtime --lib agent_` | 통과(32, 창·headless revision 일치 test 2개 포함) |
| `cargo test -p stellar-cli --test output_fixtures` | 통과(26) |
| `cargo test -p stellar-mcp --test catalog_fixtures --test trust_boundary` | 통과 |
| `cargo xtask generate-command-schema --check`, `cargo xtask generate-ffi-header --check` | drift 없음 |
| `cargo xtask validate-adr --base eb09bb43` | 통과(exit 0) |
| `STELLAR_CHECK_BASE=eb09bb43 cargo xtask check-core` | 통과(exit 0) |

### 남은 위험 / 한계

- 창 저장이 원본은 썼지만 cache stamp를 쓰지 못한 상태(`cache_stale`)는 창 메모리에 표시가 없어 창과 headless
  revision이 다릅니다. 다음 저장이 맞춥니다.
- 창은 사용자 세트 내용을 반영하지 않으므로(활성 세트 거울 연결 작업 범위), 외부 세트 편집 뒤 창 revision은 바뀌지만
  창의 생략값 해석은 그대로입니다.
- JSON 키 변경으로 CLI·MCP fixture가 한 줄씩 바뀝니다. 같은 fixture를 바꾼 다른 작업과 통합할 때 다시 bless합니다.
