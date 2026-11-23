# 작업 보고서: XXIV.4.4 `stellar apply`

- WBS: XXIV.4.4
- 작업명: 파일·표준 입력의 CommandTransaction JSON과 base revision을 closed enum으로 검증해 세대 전환 저장으로 원자적으로 기록, `--dry-run`은 semantic diff만 출력
- 상태: `[~]` 유지 — 남은 범위·결함 있음(2026-11-23 재판정, 아래 "재판정" 절). 이전 기록: 부분 완료 (`[~]`) — 적용·검증·세대 전환 저장·dry-run은 끝났으나, "generation metadata에 writer 종류 headless CLI 기록"은 보호 경로 `crates/storage`(generation manifest 포맷) 변경과 ADR 갱신이 필요한데 이 lane의 수정 허용 경로 밖이라 하지 못했습니다.
- 선행: XXIV.4.2(이 lane에서 구현, `[~]`: 남은 일은 문서 반영뿐), VII.1.6(`[x]`), XIV.3.8(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/ops/src/apply.rs`(신규) — `CommandInput`(필드 형식 5종 + canonical 형식), `ApplyRequest`, `ApplyMode`, `ApplyError`, `ApplyOutcome`, `decode_commands`, `apply`.
- `crates/ops/src/diff.rs`(신규) — 문서 레코드 semantic diff(XIV.3) 표시용 `DiffEntry`·`DiffReport`, `diff_records`, `snapshot_of`.
- `crates/ops/src/snapshot.rs` — `document_model`(런타임 모델 읽기) 추가, `document_records`가 이를 사용.
- `crates/ops/src/lib.rs`, `crates/ops/Cargo.toml` — `diff`·`apply` 모듈, `stellar-commands` 직접 의존(ADR-0005 §8 허용 목록 안).
- `tools/stellar-cli/src/apply.rs`(신규) — `stellar-apply` v1 입력 JSON 해석, 종료 코드 분류, 결과 JSON.
- `tools/stellar-cli/src/views.rs` — `diff_entry`·`diff_report` JSON 변환.
- `tools/stellar-cli/src/cli.rs`, `src/lib.rs` — `apply` 등록, 사용법 오류 test 확장.
- `tools/stellar-cli/tests/output_fixtures.rs` — `apply_dry_run_and_write_fixtures`, `apply_rejection_fixtures_write_nothing`.
- `tools/stellar-cli/tests/fixtures/help.txt`, `tests/fixtures/results/apply-*.json`(신규 6개).
- `Cargo.lock`, `docs/task-reports/XXIV.4.4.md`(신규), 계획서(XXIV.4.4 `[ ]`→`[~]`).

## 구현 요약

- **입력**: `stellar apply <경로> <입력 파일|-> [--dry-run]`. 입력은 `{"format":"stellar-apply","format_version":1,"base_revision","document","commands":[...]}`이며 모든 object는 정의된 key만 허용합니다(엄격 reader, 8 MiB·깊이 16 상한, command 10000개 상한).
  - 필드 형식: `set-visible`, `set-locked`, `rename`(VII.2.5 specialization), `delete-subtree`, `move-node`(OrderKey 16진수).
  - 그 밖의 closed enum 종류는 `{"kind":"<CommandKind 이름>","canonical":"<encode_command 16진수>"}`로 받습니다. 이름이 registry(VII.1.2 `CommandKind::ALL`)에 없거나, bytes의 kind가 registry 밖이면 `unknown-command`, 선언 이름과 bytes kind가 다르면 `invalid-transaction`으로 거부합니다. 어느 형식이든 `Command` closed enum으로만 바뀌어 raw patch·script가 실행 경로에 닿지 않습니다.
- **적용 순서(`stellar_ops::apply::apply`)**: command 해석 → snapshot 열기(XXIV.4.2) → base revision(`g<n>:<hash>`) 비교 → 대상 문서 로드 → `DocumentState::execute_transaction`(VII.1.6, 전부 성공 또는 전부 롤백, origin `User`) → 적용 전후 레코드 semantic diff → dry-run이면 종료.
- **세대 전환 저장**: 모든 문서를 다시 직렬화하고(VI.10.2, 결과 문서는 저장 전 validator 통과 필수) 문서가 아닌 chunk pointer는 Clean으로 재사용해 같은 base generation 위의 SavePlan을 만듭니다(V.5.1). 저장 파이프라인(V.5.2 optimistic check → chunk 기록·재검증 → manifest 기록 → current pointer 원자 교체)을 그대로 씁니다. 같은 내용의 chunk는 hash 비교로 재사용되어 예시 fixture에서는 바뀐 subtree chunk 1개만 기록됐습니다.
- **종료 코드**: base revision 불일치 65(`base-revision-mismatch`), 계획 뒤 다른 writer의 교체 65(`concurrent-write`, 처음에는 `base-revision-mismatch`와 같은 코드였으나 저장 잠금 동시 쓰기 test가 둘을 구분하도록 리뷰 보완에서 나눔), 형식 오류 65(`invalid-transaction`·`invalid-json`), 등록되지 않은 종류 65(`unknown-command`), 없는 문서 66, validator 거부 1(`rejected-command`), 결과가 저장 불변식 위반 1(`invalid-result`), 저장 실패 73. 실패하면 current generation은 바뀌지 않습니다.
- **결과 JSON**: `mode`, `document`, `base_revision`, `revision`(write 후 새 generation), `command_count`, `written_chunks`, `changes`(`count`, `by_kind`, canonical 순서 `entries`).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops` | 18 passed, 0 failed |
| `cargo test -p stellar-cli` | unit 19, headless_help 6, json_copy_sync 2, output_fixtures 15 passed, 0 failed |
| `cargo clippy -p stellar-cli -p stellar-ops --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt --all -- --check` | 통과 |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |
| `cargo xtask validate-dependencies` | 종료 코드 0 |
| `cargo test -p xtask --lib crate_graph` | 26 passed |

| 검사 | 결과 |
|---|---|
| dry-run 전후 revision 동일·프로젝트 파일 hash 동일·semantic diff 출력(ops `dry_run_reports_diff_without_changing_revision_or_files`, 명령 `apply-dry-run.json`) | 통과 |
| base revision 불일치 거부(ops, 명령 `apply-base-mismatch.json` exit 65) | 통과 |
| unknown command 거부: 등록되지 않은 이름, 등록된 이름 + registry 밖 kind bytes(ops), JSON의 `run-script`(명령 `apply-unknown-command.json` exit 65) | 통과 |
| 두 번째 command 거부 시 첫 번째도 미적용(ops `Rejected { index: 1 }`, 명령 `apply-rejected.json` exit 1), 거부 전체 후 파일 hash·revision 동일 | 통과 |
| 적용 후 reopen semantic snapshot이 같은 transaction을 메모리에서 적용한 결과와 일치, 다른 문서 불변(`write_switches_generation_and_reopen_matches_the_applied_model`) | 통과 |
| 표준 입력 write 후 revision `g2:`, 결과 revision과 query revision 일치, node 값 변경 확인(`apply_dry_run_and_write_fixtures`) | 통과 |
| canonical 형식 왕복과 종류 이름 위조 거부(`canonical_form_round_trips_every_field_form`) | 통과 |
| writer 종류 headless CLI 기록 확인 | **미수행**(아래 "차이") |

- 성능 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- **writer 종류 기록을 하지 않았습니다.** 현재 `GenerationManifest`(V.4.1)에는 writer 종류 필드가 없고, 추가하려면 보호 경로 `crates/storage`의 manifest 포맷·codec·검증과 ADR(ADR-0010/0011 계열) 갱신이 같은 변경에 필요합니다. 이 lane의 수정 허용 경로에 `crates/storage`가 없어 `[~]`로 두었습니다. 이 때문에 XXIV.4.6(앱이 CLI writer generation을 reconciliation으로 처리)은 아직 writer를 구분할 수단이 없습니다.
- command 입력 JSON의 필드 형식은 자주 쓰는 5종만 두고, 나머지 closed enum 종류는 canonical bytes 형식으로 받습니다. 종류별 JSON schema는 XIX.9.2(xtask가 closed enum에서 schema 생성)의 범위로 남겼습니다.

## 남은 위험 / 한계

- **후속(`[x]`로 올리기 위한 남은 일)**: storage generation manifest에 writer 종류(앱 창 / headless CLI)를 넣는 포맷 변경 + ADR, `stellar_ops::apply::apply`가 headless CLI로 기록, reopen fixture로 확인.
- 쓰기는 모든 문서를 메모리에 올려 다시 직렬화합니다. 내용이 같은 chunk는 쓰지 않지만 대형 프로젝트에서 시간·메모리가 문서 수에 비례합니다.
- 교차 프로세스 저장 잠금은 아직 없습니다(XXIV.4.5). 현재는 V.5.2 optimistic check가 계획과 pointer 교체 사이의 다른 writer를 쓰기 전에 거부하지만, check와 교체 사이의 아주 짧은 구간은 잠금으로 막아야 합니다.
- `CommandOrigin::User`를 씁니다. headless 쓰기는 앱 history에 들어가지 않으므로 origin은 저장되지 않습니다.

## 재판정 (2026-11-23, ledger-verification)

- 판정: `[~]` 유지 — 남은 범위·결함 있음.
- 근거: 적용·dry-run·base revision 비교·closed enum 검증·잠금 판정은 코드에 있습니다. 행 지시의 "generation metadata에 writer 종류 headless CLI를 남김"은 아직 없어 `[~]`를 유지합니다.
- 판정 기준: 행 지시·검증 열의 불변식을 구현 코드에서 확인했고, 테스트 통과는 보조 근거로만 썼습니다. 계획서 §1.2 규칙 4(선행이 모두 `[x]`여야 완료)를 적용했습니다. 기준 커밋은 `main` `ce60bb92`입니다.

### 확인한 코드 위치

- `crates/ops/src/apply.rs`: CommandTransaction JSON → closed enum 해석, base revision 비교, 세대 전환 저장, `--dry-run`은 semantic diff만. 잠금 판정 `locked_change`는 `stellar_commands::lock::{EditIntent, NodeLock}`을 적용 전 레코드로 계산하며 일반 수정은 `EditIntent::Edit`입니다. Codex 검토(48fb4828)가 지적한 `from_records` 실패를 잠금 위반으로 보고하던 경로와 `DISPLAY_NAME` 대리 intent는 지금 코드에 없습니다.
- `crates/storage/src/generation/manifest.rs` `GenerationManifest`: generation·parent·project_id·schema_version·chunk_pointers·root_document_ids뿐이고 writer 종류 필드가 없습니다.

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test --workspace`(DEV-MAC, 샌드박스 해제) | 테스트 binary 142개 모두 ok, 4136 passed, 0 failed |
| `cargo test -p stellar-ops` | 51 passed |
| `cargo test -p stellar-cli` | output_fixtures 26 passed |

### 남은 조건과 finding

- 결함(남은 범위): writer 종류 필드는 보호 경로 `crates/storage`(generation manifest 포맷)와 ADR이 필요합니다. 이 경로를 소유한 첫 배치 lane이 없으므로 backlog finding으로 넘깁니다. 저장 형식 전환(텍스트 원본, ADR-0061) 뒤에는 writer 기록 위치를 generation manifest로 둘지 다시 정해야 합니다.
- XXIV.4.5가 이 행을 선행으로 둡니다.
