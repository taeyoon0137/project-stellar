# 작업 보고서: XXIV.1.3 `validate`·`semantic-diff`·`semantic-merge`·`migrate` 명령

- WBS: XXIV.1.3
- 작업명: headless `stellar`에 프로젝트 검증, 프로젝트 사이 semantic diff, 3-way semantic merge, 저장 스키마 migration 명령을 추가
- 상태: `[~]` 유지 — 구현 판정 통과, 선행 미충족(2026-11-23 재판정, 아래 "재판정" 절). 이전 기록: 부분 완료 (`[~]`) — 네 명령과 fixture corpus는 통과했으나, 선행 "Phase XIV"에 `[ ]` 행이 남아 있습니다(XIV.10.7~XIV.10.9, XIV.12.3, XIV.12.4 등). 계획서 §1.2 규칙 4에 따라 Phase XIV가 모두 `[x]`가 될 때까지 `[~]`로 둡니다. 또 `semantic-merge`는 병합 결과를 프로젝트에 쓰지 못하고(XIV.12.4 없음) `migrate`에는 제품 migration step이 없습니다(아래 "차이").
- 선행: Phase XIV(semantic diff XIV.3·merge XIV.10 엔진 코드 있음, 일부 행 `[ ]`), V.8.6(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/ops/src/validate.rs`(신규) — `validate_project`·`validate_snapshot`, `ValidationReport`·`Finding`.
- `crates/ops/src/compare.rs`(신규) — `semantic_diff_projects`, `semantic_merge_projects`, `ConflictEntry`·`SideView`, `CompareError`.
- `crates/ops/src/migrate.rs`(신규) — `CURRENT_SCHEMA_VERSION`, `product_registry`, `schema_status`, `migrate`(저장 잠금 사용).
- `crates/ops/src/lib.rs`, `crates/ops/Cargo.toml` — 모듈 3개, `stellar-semantic-merge` 직접 의존(ADR-0005 §8 허용 목록 안).
- `crates/ops/src/test_support.rs` — `save_documents_with_schema`, `corrupt_view_tree_chunks`.
- `tools/stellar-cli/src/project_cmds.rs`(신규) — 네 명령, 결과 JSON, 종료 코드 분류, 공용 `lock_error`.
- `tools/stellar-cli/src/output.rs` — `CliError::with_result`: 도메인 실패 envelope에도 판정 근거 `result` object를 담음(기존 오류는 계속 `null`).
- `tools/stellar-cli/src/apply.rs` — 잠금 오류 분류를 `project_cmds::lock_error`로 통일.
- `tools/stellar-cli/src/cli.rs`, `src/lib.rs` — 명령 등록·도움말, 사용법 오류 test 확장.
- `tools/stellar-cli/tests/output_fixtures.rs` — corpus test 3개, 성공 stderr 검사를 "진단 줄만"으로 일반화.
- `tools/stellar-cli/tests/fixtures/help.txt`, `tests/fixtures/results/`(신규 12개: `validate-*` 3, `semantic-diff*` 2, `semantic-merge-*` 2, `migrate-*` 5).
- `Cargo.lock`, `docs/task-reports/XXIV.1.3.md`(신규), 계획서(XXIV.1.3 `[ ]`→`[x]`).

## 구현 요약

- **`stellar validate <경로>`**: snapshot의 모든 문서를 읽어 문서 validator(VI.10.1)를 다시 실행합니다. 문서 chunk를 읽지 못하면 `document-load`, validator 위반은 `document-invalid` 오류, 미해결 참조(III.2.4)는 `unresolved-reference` 경고입니다. 오류가 있으면 종료 코드 1(`validation-failed`)이며 결과 object와 진단 목록이 함께 출력됩니다.
- **`stellar semantic-diff <base> <target>`**: 같은 project id의 두 프로젝트 상태(worktree·checkout 등)를 열어 모든 문서를 semantic snapshot으로 투영하고 XIV.3 diff를 canonical 순서로 출력합니다. project id가 다르면 65(`project-mismatch`).
- **`stellar semantic-merge <base> <ours> <theirs>`**: 세 상태를 XIV.10 엔진으로 병합해 base → 자동 병합 결과 변경과 conflict(종류, 대상, base·ours·theirs 상태)를 출력합니다. conflict가 있으면 종료 코드 1(`unresolved-conflicts`, "앱에서 해결" 안내)이고 결과에 세 상태가 모두 남습니다. 파일은 쓰지 않습니다.
- **`stellar migrate <경로> [--check]`**: generation 스키마를 이 도구의 스키마(현재 1)와 비교합니다. `--check`는 판정만(`up-to-date`·`unsaved` 0, `migration-required` 1, `unsupported-newer` 65). 실행 모드는 저장 잠금(XXIV.4.5)을 잡은 뒤 다시 판정하고, 낮으면 제품 registry 경로로 V.8.3 runner(원본 세대 불변, 실패 시 current 유지)를 실행합니다. 현재 제품 스키마는 1뿐이라 제품 registry에는 step이 없고, 쓰기 경로는 시험용 v1→v2 noop step으로 검증했습니다.
- **envelope 확장**: 도메인 실패(1)와 일부 판정 실패(65)에서 `result`에 판정 근거 object를 담습니다. schema(`result`: object 또는 null)와 기존 fixture는 그대로입니다.
- 모든 명령은 결과 JSON에 경로를 넣지 않습니다. 경로는 stderr 진단 문맥에만 씁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops` | 32 passed, 0 failed |
| `cargo test -p stellar-cli` | unit 19, headless_help 6, json_copy_sync 2, output_fixtures 21 passed, 0 failed |
| `cargo clippy -p stellar-cli -p stellar-ops --all-targets -- -D warnings` | 경고 0(첫 실행에서 `CliError`에 결과를 직접 넣어 `result_large_err` 경고 → `Box`로 바꿔 해결) |
| `cargo fmt --all -- --check` | 통과 |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |
| `cargo xtask validate-dependencies` | 종료 코드 0 |
| `cargo xtask test-migrations` | migration golden fixture 2개 통과 |

| 검사(fixture corpus) | 결과 |
|---|---|
| validate: 문서 3개 프로젝트 0(경고 1 포함)·저장 전 프로젝트 0·ViewTree chunk 손상 1, golden 3종, 파일 변경 0(`validate_corpus_fixtures`) | 통과 |
| validate(ops): 유효 corpus 4문서 통과·손상 chunk `document-load`(`valid_corpus_passes_and_writes_nothing`, `corrupt_document_chunk_is_reported_as_error`) | 통과 |
| semantic-diff: 같은 사본 변경 0, `stellar apply` 1건 후 변경 1, golden 2종(`semantic_diff_and_merge_fixtures`) | 통과 |
| semantic-merge: 겹치지 않는 편집 clean(0)·같은 속성 다른 값 conflict 1(종료 코드 1, ours/theirs 값 보존), golden 2종, 입력 프로젝트 파일 변경 0 | 통과 |
| semantic-diff·merge(ops): 다른 project id 거부(`different_projects_are_rejected`), conflict 세 값(`non_overlapping_edits_merge_cleanly_and_same_property_conflicts`) | 통과 |
| migrate: 스키마 1 `--check`·실행 최신(파일 변경 0), 스키마 2 `--check`·실행 65(파일 변경 0), 저장 전 `unsaved`, golden 5종(`migrate_corpus_fixtures`) | 통과 |
| migrate(ops): v1→v2 경로로 새 generation·reopen 최신, 경로 없음 시 원본 유지, 최신 시 쓰기 0 | 통과 |

- 성능 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- `semantic-merge`는 병합 결과를 프로젝트 파일로 기록하지 않고 결과·conflict만 출력합니다. semantic snapshot을 프로젝트 generation으로 되돌려 쓰는 경로(XIV.12.4)가 아직 없기 때문입니다. Git merge driver(XXIV.2.1)는 그 경로가 필요합니다.
- 제품 스키마가 1뿐이라 `migrate`가 실제로 migration을 수행하는 제품 경로는 아직 없습니다. 판정·잠금·runner 연결은 구현했고 쓰기 경로는 시험용 registry로 확인했습니다.

## 남은 위험 / 한계

- **후속(`[x]`로 올리기 위한 남은 일)**: Phase XIV의 남은 행(특히 merged snapshot 생성 XIV.10.8, 결과를 WorkingTree로 쓰는 XIV.12.4)이 `[x]`가 된 뒤 `semantic-merge`가 병합 결과를 기록하는지 다시 판단하고, fixture corpus를 다시 실행해 `[x]`로 바꿉니다.
- validate·diff·merge는 모든 문서를 메모리에 올립니다. 대형 프로젝트에서 시간·메모리가 문서 크기에 비례합니다.
- semantic snapshot 투영은 문서 레코드(정의·node·modifier)만 다룹니다. token·component·asset은 저장 포맷이 없어 diff·merge 대상이 아닙니다.
- validate 경고 메시지의 참조 종류 표기는 Rust enum 이름(`Node` 등)입니다.

## 재판정 (2026-11-23, ledger-verification)

- 판정: `[~]` 유지 — 구현 판정 통과, 선행 미충족.
- 근거: 네 명령은 행 지시(명령 구현, fixture corpus 통과)를 만족합니다. 선행 "Phase XIV"에 `[ ]` 행 24개가 남아 규칙 4로 `[~]`를 유지합니다. 다만 `semantic-merge`의 clean 판정은 XIV.10.8(merged snapshot full validator)이 없으면 무효 결과를 clean으로 낼 수 있어, 선행을 통째로 없애는 재작성은 권하지 않습니다.
- 판정 기준: 행 지시·검증 열의 불변식을 구현 코드에서 확인했고, 테스트 통과는 보조 근거로만 썼습니다. 계획서 §1.2 규칙 4(선행이 모두 `[x]`여야 완료)를 적용했습니다. 기준 커밋은 `main` `ce60bb92`입니다.

### 확인한 코드 위치

- `crates/ops/src/validate.rs` `validate_project`: 모든 문서를 읽어 문서 validator를 다시 실행하고 오류·경고를 분리합니다.
- `crates/ops/src/compare.rs` `semantic_diff_projects`·`semantic_merge_projects`: 같은 project id만 받고 XIV.3 diff·XIV.10 `merge`를 부릅니다. merge 결과(`result.conflicts`)를 그대로 내보내며 merged snapshot을 검증하지 않습니다(425행 이후). 병합 엔진은 `BTreeMap`·`BTreeSet`으로 순서가 정해져 출력은 결정적입니다.
- `crates/ops/src/migrate.rs`: `CURRENT_SCHEMA_VERSION = 1`, 제품 registry step 없음, 실행 모드는 저장 잠금 안에서 다시 판정합니다.
- `tools/stellar-cli/src/project_cmds.rs`: 종료 코드 분류(1 도메인 실패, 65 판정 실패)와 결과 JSON.

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test --workspace`(DEV-MAC, 샌드박스 해제) | 테스트 binary 142개 모두 ok, 4136 passed, 0 failed |
| `cargo test -p stellar-ops` | 51 passed |
| `cargo test -p stellar-cli` | output_fixtures 26 passed(validate·semantic-diff·semantic-merge·migrate corpus 포함) |

### 남은 조건과 finding

- 선행 재작성안(통합 세션 판단): XXIV.1.3의 "Phase XIV"를 실제로 쓰는 "XIV.3, XIV.10, V.8.6"으로 좁힙니다. XIV.3은 모두 `[x]`이고 XIV.10은 XIV.10.7\~XIV.10.9가 남아 있어 이 행은 여전히 `[~]`입니다. XIV.10.8이 엔진에서 무효 결과를 conflict로 돌리면 CLI 변경 없이 이 행을 `[x]`로 올릴 수 있습니다(semantic-merge lane).
- V.10.3 사슬: V.10.3이 실제로 쓰는 것은 ops snapshot(XXIV.4.2), `apply::ApplyError`, `migrate::CURRENT_SCHEMA_VERSION`뿐이고 validate·diff·merge는 쓰지 않습니다(`crates/ops/src/text_source.rs` import). V.10.3의 선행 XXIV.1.3을 "XXIV.4.2, V.8.6"으로 바꾸는 재작성안을 통합 세션에 넘깁니다. 이번에 XXIV.4.2를 `[x]`로 판정했으므로 이 재작성이 들어가면 V.10.3은 V.10.2와 함께 착수 가능합니다.
- `semantic-merge`가 결과를 파일에 쓰지 않는 것은 행 지시와 충돌하지 않습니다(기록은 XXIV.2.1·XIV.12.4 범위).
