# 작업 보고서: XIV.14.3 component/token dependency staging scenario

- WBS: XIV.14.3
- 작업명: component/token dependency staging scenario
- 상태: 완료 (`[x]`) 후보 — 2026-11-24 library-wire, 아래 "작업 (2026-11-24, library-wire)" 절: 컴포넌트 → token set 의존 staging 시나리오를 더했습니다. 원장 판정은 통합 때 정합니다. 이전 기록: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 남은 범위가 있음(아래 본문). 보고 당시 기록: 부분 완료 (`[~]` 후보)
- 선행: XIV.5.4
- 결정: ADR-0080

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, XIV.14.1 fixture: 로컬 bare 저장소, 네트워크 없음).

## 변경 파일 목록

- `crates/ops/src/git.rs`(원본 단위 동반 판정)
- `crates/runtime/src/staging/engine_tests.rs`
- `tools/stellar-cli/tests/git.rs`

## 구현 요약

- 구조 의존: 새 Frame 아래로 기존 node를 옮긴 뒤 위치 변경만 stage하면 `CompanionRequired`(새 Frame 추가 포함)이고 index bytes는 그대로입니다. 필수가 아닌 승인은 `UnexpectedApproval`입니다. 승인하면 성공하고 재검증을 통과합니다.
- 컴포넌트 의존: 새 컴포넌트 원본과 그 인스턴스를 배치한 View에서 `generation`만 stage하면 `CompanionRequired`(단위 `file:design/components/...`, 이유: 정의 id)입니다. 승인하면 두 단위가 staged입니다. 정의 파일만 unstage하면 `generation` 동반이 필요합니다. commit은 경고 없이 성공합니다.
- 정의 삭제: 작업 트리에서 정의와 인스턴스를 함께 지운 뒤 정의 파일만 stage해도 `generation` 동반이 필요합니다(오류가 아니라 승인 목록).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops -p stellar-runtime -p stellar-git-porcelain -p stellar-cli -p stellar-mcp-core` | 모두 통과(실패 0) |
| `cargo clippy -p stellar-ops -p stellar-runtime -p stellar-cli -p xtask --all-targets --features stellar-ops/test-support -- -D warnings` | exit 0 |
| `cargo xtask validate-crate-graph` | exit 0 |
| `cargo check --workspace --all-targets` | exit 0 |

| 검사 | 결과 |
|---|---|
| 구조 의존 차단 → 승인 후 성공 (`moving_into_new_frame_requires_approval_of_the_addition`) | 통과 |
| 컴포넌트 정의 동반 차단 → 승인 후 성공, unstage 동반 (`component_definition_is_a_required_companion_unit_not_auto_included`) | 통과 |
| 작업 트리에서 정의와 인스턴스를 함께 지운 뒤 정의 파일만 stage하면 `generation` 동반 요구 → 승인 후 경고 없이 commit (`staging_definition_removal_requires_generation_companion_not_dangling_error`) | 통과 |
| CLI: `stage generation` 종료 코드 1 `companion-required`, `--with`로 승인 후 commit (`companion_component_is_listed_and_included_only_with_approval`) | 통과 |

## 테스트

- 위 검사 표의 테스트는 `cargo test -p stellar-runtime --lib staging`(19개)과 `cargo test -p stellar-cli --test git`(4개)에 포함되며 모두 통과했습니다.

## 성능

- 성능 예산 대상 경로를 바꾸지 않았고 수치를 측정하지 않았습니다. status·commit 재검증은 HEAD·index를 임시 디렉터리에 materialize합니다(ADR-0080 Performance).

## 차이(작업 지시와 다른 점)

- 셸 FFI와 Swift UI는 사용자 방향(2026-11-23)에 따라 다루지 않았습니다. 엔진은 Rust 테스트와 `stellar git` CLI로 검증했습니다.

## 남은 위험 / 한계

- token은 저장 포맷이 없어(문서 레코드에 token 없음) 저장 프로젝트 scenario를 만들 수 없습니다. token·binding 의존은 semantic-diff dependency graph(XIV.5.2) 단위 테스트로만 검증됩니다. token 저장 포맷이 생기면 같은 scenario를 추가해야 합니다.

## 후속 판정 (2026-11-23, lane `recovery-git-terminal`)

- 판정: `[~]` 유지. 컴포넌트 의존 staging(차단 → 승인 뒤 성공)은 위 엔진·CLI 시험으로 충족합니다.
- token 의존 staging 시나리오는 지금 형식으로 만들 수 없음을 다시 확인했습니다([ADR-0120](../adr/ADR-0120-recovery-probe-git-scenarios-terminal-env.md) 결정 5).
  - 텍스트 원본에는 token set 파일(`design/tokens/<slug>.stellar.yaml`, `stellar_project::document_io::source_v4::token`)이 생겼고
    ops staging도 이 파일을 독립 원본 단위로 다룹니다(`stellar_ops::git::SourceUnit`).
  - 그러나 문서 레코드(`stellar_document::records::DocumentRecords`)에 token 참조가 없어 View가 token set에 기대는 저장 프로젝트가
    생기지 않고, 동반 단위 계산(`stellar_ops::git` `companion_closure`)도 컴포넌트 정의만 봅니다. 그래서 "token 의존 partial
    차단"이 생길 경로가 없습니다.
- 남은 일: token 참조가 문서 형식에 들어올 때 ops 동반 단위 계산에 token set 의존을 더하고, 같은 scenario(View만 stage → token
  set 동반 요구 → 승인 뒤 성공)를 XIV.14.1 harness로 더합니다. 둘 다 이 lane 소유 밖(document·ops)입니다.

## 작업 (2026-11-24, library-wire): token 의존 staging

- 결정: [ADR-0127](../adr/ADR-0127-library-product-paths-token-companion.md) 결정 4. 위 "후속 판정"의 전제(문서 레코드의 token 참조)와
  달리, 저장 형식의 token 의존은 컴포넌트 텍스트 원본의 `local-tokens` → token set 원본(`design/tokens/<slug>.stellar.yaml`)으로 이미
  있습니다. 이 의존으로 시나리오를 만들었습니다.
- 동반 계산을 `crates/ops/src/git/companion.rs`로 옮기고 token 의존을 더했습니다. 후보 컴포넌트의 token이 후보 token set에 없으면 원천에서
  그 token을 담은 token set 파일, 원천이 참조를 뺐으면 그 컴포넌트 파일이 동반입니다. 원천에서도 끊긴 참조(프로젝트 token set 밖)와
  기준 index에서 이미 끊긴 참조는 막지 않습니다. `CompanionReason`에 `token`이 생겼고, token set을 읽지 못하면
  `PrepareStageError::TokenSource`입니다.

### 변경 파일

- `crates/ops/src/git/companion.rs`(신규), `crates/ops/src/git.rs`
- `tests/integration/tests/git_token_staging.rs`(신규), `tests/integration/tests/git_common/mod.rs`(`write_text_project`)

### 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops --features test-support --lib git::companion` | 0 — 3 passed |
| `cargo test -p stellar-integration-tests --test git_token_staging` | 0 — 1 passed |
| 같은 시험을 token 단계를 끈 상태로 실행(일시 변경, 되돌림) | 실패 — 시험이 token 의존을 실제로 판정함을 확인 |

| 검사 | 결과 |
|---|---|
| 새 컴포넌트 Card(`local-tokens` = Brand accent)만 stage → `CompanionRequired`(단위 = Brand token set 파일, 이유 = Card·accent), index bytes 불변. 필수가 아닌 승인은 `UnexpectedApproval` | 통과 |
| token set만 stage·unstage는 동반 없음. 승인하면 두 파일만 staged, `validate_staged` 통과 | 통과 |
| token set만 unstage → Card 파일 동반 | 통과 |
| commit 뒤 작업 트리에서 Brand 삭제 + Card 참조 제거 → Brand 삭제만 stage하면 Card 동반, index 불변 → 승인 뒤 재검증 통과, commit 경고 없음, HEAD에 두 파일 | 통과 |
| 단위 시험: 새 컴포넌트·새 token set, unstage, token set 삭제와 참조 제거, 프로젝트 밖 token·이미 끊긴 참조는 막지 않음 | 통과 |

### 남은 위험 / 한계

- CLI `stellar git` JSON의 `reasons`에는 `token` 필드가 아직 없습니다(CLI 출력 형식은 CLI 소유).
- 문서 레코드·binding이 token을 저장하게 되면 View → token 의존을 같은 모듈에 더해야 합니다.
- staging 파이프라인은 index를 저장 프로젝트로 다시 열므로 generation(`design/current`)이 있는 프로젝트가 대상입니다. 시험도 앱 저장처럼
  텍스트 원본과 generation을 함께 씁니다.
