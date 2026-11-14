# 작업 보고서: XXIV.1.4 지정 View의 정적 접근성 HTML CLI 경로

## WBS

- WBS: XXIV.1.4
- 작업명: 저장 프로젝트에서 지정 문서·View의 정적 접근성 HTML 조각 생성
- 상태: 부분 완료 (`[~]`) — 일반 render/export는 미구현. 이 보고서는 접근성 전용 연결의 증거입니다.
- 선행: Phase XX. 이 좁은 경로는 기존 XXIV.4.2 snapshot·XXII.2.4 HTML/ARIA mapper를 사용합니다.

## 변경 파일

- `crates/ops/src/accessibility_html.rs` (신규) — snapshot 문서 모델·View root 선택, 범위·누락·실패 진단, HTML 생성.
- `crates/ops/src/lib.rs`, `crates/ops/Cargo.toml`, `Cargo.lock` (수정) — module 및 기존 core crate 의존 연결.
- `crates/codegen/src/ir/mod.rs`, `crates/codegen/src/ir/build.rs`, `crates/codegen/src/ir/binding.rs` (수정) —
  기존 IR tree를 분리하고 정적 구조·텍스트·접근성 입력만 받는 `build_accessibility_ir` 추가. 전체 입력의 검증 유지.
- `crates/codegen/src/web/aria.rs` (수정) — 같은 mapper가 tree를 소비. 숨긴 Fragment 하위의 aria-hidden 누락 수정.
- `crates/codegen/src/ir/tests.rs` (수정) — 가상 layout·profile 없는 입력, escaping, 미해결 참조·정적 검사 검증.
- `crates/ops/src/test_support/accessibility.rs` (신규), `crates/ops/src/test_support.rs` (수정) — 두 View 실제 저장 fixture.
- `tools/stellar-cli/src/accessibility_html.rs` (신규), `tools/stellar-cli/src/lib.rs`, `tools/stellar-cli/src/cli.rs` (수정) —
  기존 JSON envelope·종료 코드·stderr 규약으로 명령 연결. dispatch는 등록·분기만 추가.
- `tools/stellar-cli/tests/accessibility_html.rs`, `tools/stellar-cli/tests/fixtures/accessibility/view-a.html`,
  `tools/stellar-cli/tests/fixtures/accessibility/view-b.html` (신규), `tools/stellar-cli/tests/fixtures/help.txt` (재생성) —
  실제 stellar 프로세스·Git 저장소·두 View golden 및 오류 경로.
- `tools/xtask/src/crate_graph.rs`, `docs/architecture/crate-dependency-graph.md`,
  `docs/adr/ADR-0005-agent-tool-layers.md`, `docs/adr/ADR-0049-codegen-crate.md` (수정) — ops의 codegen·accessibility·target-profile
  직접 간선만 추가. CLI→ops와 상위 도구 전이 금지 유지.
- `docs/guides/stellar-cli.md` (수정), `docs/task-reports/XXIV.1.4.md` (신규) — 실제 사용법·지원 범위·진단·검증 증거.

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `cargo check -p stellar-codegen` | 0 |
| `cargo run -q -p stellar-cli -- --help > tools/stellar-cli/tests/fixtures/help.txt` | 0 |
| `cargo fmt -p stellar-codegen -p stellar-ops -p stellar-cli -p xtask` | 0 |
| `cargo xtask generate-ledger` | 0 (원장 최종 diff 없음) |
| `cargo test -p stellar-codegen -p stellar-ops -p stellar-cli` | 0 (139 tests; codegen 42, ops 37, CLI 60) |
| `cargo test -p stellar-cli --test accessibility_html` | 0 (schema 거부 추가 후 5 passed) |
| `cargo test -p stellar-ops accessibility_html` | 0 (1 passed) |
| `cargo clippy -p stellar-codegen -p stellar-ops -p stellar-cli --all-targets -- -D warnings` | 0 (최종 코드, 경고 0) |
| `cargo xtask validate-crate-graph` | 0 |
| `cargo xtask validate-task-status` | 0 |
| `cargo xtask validate-dependencies` | 0 |
| `cargo xtask validate-versions` | 0 |
| `target/debug/xtask validate-task-report docs/task-reports/XXIV.1.4.md` | 0 |
| `cargo fmt --all -- --check` | 0 |
| `cargo test -p stellar-codegen` | 0 (tree 생성자 경계 확인 후 42 passed) |
| `cargo xtask check-core` | 65 (fmt·clippy 통과; workspace test 3756 passed, 1 failed, 8 ignored — 기존 보고서 형식 오류) |
| `cargo test --workspace --locked --doc` | 0 (check-core 중단 뒤 별도 실행) |
| `cargo test -p xtask --test release_profile --test workspace_lints` | 0 (5 passed) |
| `git diff --cached --name-only -z` → `xargs -0 target/debug/xtask validate-adr` | 0 |
| `git diff --check` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: DEV-MAC 로컬 실행(macOS 27.2, rustc 1.96.0). CI-MAC·CI-WIN·브라우저·스크린리더 검증은 실행하지 않았습니다.

## 테스트

- 실제 serializer·save pipeline으로 저장한 두 View를 `stellar` 실행 파일이 읽습니다. 앱·환경 변수·stdin 없이
  실행해 한 줄 JSON, revision, View의 실제 root, HTML golden을 확인합니다.
- View A의 한글·줄바꿈·HTML 문자, 중첩 Fragment 순서, 숨긴 Fragment 하위 aria-hidden을 검증합니다.
  View B만 고르면 B 문자열만 출력됩니다. 같은 실행을 반복하면 stdout·stderr bytes가 같습니다.
- 실제 임시 Git 저장소를 초기화·커밋한 후 전체 파일 hash, Git index·refs·objects, HEAD와 status가 같음을 확인합니다.
- B에 미지원 Image가 있으면 A는 성공하고 B만 node ID·종류 진단, 종료 1, `html: null`과 revision을 냅니다.
- 저장 전·없는 경로·없는 문서·없는 View·손상 chunk·미지원 generation schema·잘못된 UUID·인자 부족/추가·옵션 오류를 검사합니다.
- snapshot을 연 뒤 새 generation을 저장해도 기존 snapshot 내보내기의 HTML·revision·진단이 유지됩니다.
- codegen의 기존 Web·Apple·Android golden을 유지하며, 별도 정적 입력에 layout·capability·상태·token을
  만들지 않는지, 텍스트·속성 escaping과 범위 밖 참조·미선언 상태·매핑/정적 검사 실패가 남는지 확인합니다.

## 성능

- 성능 수치 측정은 하지 않았습니다. 선택 문서에서 부모별 자식 순서를 한 번 구성해 OrderKey·StableId로 정렬하고
  지정 View를 반복 순회합니다. node마다 문서 전체를 훑는 `children_in_order` 반복 호출은 피했습니다.
- 별도 성능 계약 변경은 없습니다. 로드 비용은 기존 `document_model`의 문서 전체 로드 범위를 따릅니다.

## 차이

- `CodegenSources`가 필요한 전체 layout·profile·state·token을 복원하지 못하므로 그 경로를 가상 값으로 채우지
  않았습니다. 실제 저장된 구조·가시성·텍스트와 기본 접근성 snapshot만 받는 좁은 tree 입력으로 기존 mapper를
  재사용합니다. 출력은 HTML 조각이며 일반 전체 코드 내보내기가 아닙니다.
- Frame·Fragment·Text만 지원합니다. Shape·Path·Image는 의미·에셋 복원을 할 수 없어 오류로 거부합니다.
  다른 View와 미지원 데이터·의미는 진단하며, 표시 이름으로 label이나 control을 만들지 않습니다.
- `help.txt`는 실제 CLI 도움말로 재생성했습니다. 저장 스키마·FFI 헤더·README 변경은 없습니다.
- 계획서의 일반 render/export 행과 원장은 미착수 상태를 유지합니다. `[~]`를 원장 생성기에 넣으면
  `reported_done`이 되므로 전체 행의 완료를 암시하지 않도록 이 부분 보고서만 추가했습니다.
  기존 생성기로 원장을 재생성한 최종 결과는 원본과 같습니다. 전체 코드 생성 행도 변경하지 않았습니다.
- text-source apply·기존 Claude 파일·다른 worktree는 수정하지 않았습니다. 에이전트·timer는 시작하지 않았습니다.

## 남은 위험

- 전체 `check-core`는 `xtask::task_report::validate::tests::repository_reports_pass`에서 실패했습니다.
  `docs/task-reports/XII.4.1-blocked.md`의 상태 `[ ]`(차단 보고서는 `[!]` 필요),
  `docs/task-reports/session-takeover-integration.md`의 WBS·환경·차이·위험 절 누락이 원인입니다.
  두 파일은 시작 HEAD `99d7923a`와 같고 이번 diff에 없습니다. 관련 없는 보고서를 고치지 않았습니다.
  새 `XXIV.1.4.md`는 단독 validator와 전체 보고서 검사에서 오류가 없었습니다.
  전체 검사의 마지막 doc-test 단계는 test 실패로 실행되지 않아 별도 명령으로 검증했고 통과했습니다.
  전체 실행 뒤 추가한 schema 거부와 tree 생성자 경계는 위의 별도 테스트·최종 clippy로 재검증했습니다.

- CSS·레이아웃·기하·paint·텍스트 스타일·에셋·modifier를 내보내지 않습니다. 숨김은 접근성의 aria-hidden만
  적용하므로 브라우저 시각적 배치를 재현하지 않습니다.
- control·binding·상태·token·Variant·Context·Motion·프로젝트 target profile capabilities의 영속 데이터 연결은
  후속 작업입니다. mapper의 정적 검사는 브라우저·스크린리더·색 대비·키보드 상호작용의 전체 적합성 검사와 다릅니다.
- 전체 문서 로드 검증은 선택하지 않은 View의 손상에도 실패할 수 있습니다. 선택 문서 밖의 node chunk는 읽지 않습니다.
- CLI는 파일을 쓰지 않습니다. 사용자가 셸로 결과를 저장할 경우 원본 프로젝트 밖 경로를 선택해야 원본 Git에 변화가 없습니다.
