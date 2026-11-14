# 작업 보고서: V.10.6 단일 텍스트 원본 CLI 명령 저장

## WBS

- WBS: V.10.6
- 상태: 부분 구현 (`[~]`). 이번 단위는 opt-in 단일 View의 `text-source apply`입니다. 선행 V.10.5·XIII.3.1과 전체 파일 감시·dirty 재로드·병합은 완료 처리하지 않습니다.
- 필수 환경: DEV-MAC.
- 기본 프로젝트 열기·generation 저장 경로는 유지합니다.
- 후속 단위: Frame 크기·local 변환을 사람이 읽는 JSON 숫자로 지정하고 동일한 명령·저장 경로에서 왕복합니다. 전체 상태는 `[~]`를 유지합니다.

## 변경 파일

첫 CLI 연결 단위의 production 파일은 아래 5개이며 새 의존성은 없습니다.

- `crates/ops/src/text_source.rs`: 원본 hash·문서 ID·필수 저장 잠금, 기존 command transaction 및 semantic diff, 조건부 저장. export/check에 document ID 응답 추가.
- `crates/project/src/document_io/text_source.rs`: 파일 I/O 없이 실제 codec으로 결과 모델·크기를 검사하는 API.
- `tools/stellar-cli/src/apply.rs`: generation apply와 텍스트 apply가 입력 제한·엄격한 JSON envelope·closed command 파서를 공유.
- `tools/stellar-cli/src/text_source.rs`: 별도 `stellar-text-apply` 입력, 파일/stdin, dry-run/write와 JSON 결과·오류 연결.
- `tools/stellar-cli/src/cli.rs`: 실제 명령 도움말.
- `tools/stellar-cli/tests/text_source.rs`, `tests/fixtures/help.txt`: CLI 프로세스·임시 Git 저장소 통합 검증과 도움말 fixture.
- 문법·ADR-0059·기술 아키텍처·구현 계획·생성 원장 및 이 보고서.

Frame 후속 단위는 `crates/ops/src/apply.rs`의 typed 입력 두 개와 `tools/stellar-cli/src/apply.rs`의 공용 JSON 파서를 확장합니다. 기존 `SetProperty`의 size·transform으로 바꾸므로 저장 opcode·binary 포맷은 유지합니다. `crates/ops/src/text_source.rs`와 `tools/stellar-cli/tests/text_source.rs`에는 실제 저장·재열기 테스트를 추가하고 CLI 안내·문법·계획서 원본과 생성 원장을 갱신합니다.

## 테스트

- 실제 기존 generation을 export하고 `check`에서 얻은 source hash와 문서 ID로 두 명령(rename, visible)을 요청합니다. 파일 입력 dry-run → stdin 쓰기 → check 재열기 → 실제 `git diff`에서 이름·visible 속성 변경을 검사합니다. 기존 generation 전체 bytes는 유지합니다.
- dry-run은 semantic diff를 반환하지만 source·generation·cache·저장 잠금 파일을 생성하지 않습니다. source_hash는 변경하지 않은 원본 bytes hash입니다.
- 쓰기는 문서 ID + canonical 파일 경로의 기존 OS 저장 잠금을 사용합니다. 잠금 뒤 원본을 열고 expected hash와 ID를 확인합니다. 저장 전 재해석·hash 확인과 temp/fsync/rename은 기존 저장 계층을 재사용합니다.
- 같은 값 적용은 주석·bytes·hash를 보존합니다. 주석만 외부 변경해도 stale hash는 거부합니다. unknown 원본 필드, 잘못된 문서, unknown command, 중간 명령 실패는 원본을 보존합니다.
- canonical InsertNode로 미지원 Text 노드가 생기는 경우 dry-run/write 모두 거부합니다. 이름 변경의 semantic diff와 저장 후 재구축 모델이 일치하는지 ops 테스트로도 검사합니다.
- 저장 잠금 경합은 write를 거부하고 dry-run은 성공합니다. 잠금을 제공하지 않은 write도 거부합니다.
- Frame 후속 단위: 실제 CLI 프로세스에서 JSON 정수·소수·지수 입력 → dry-run → write → check → 실제 Git diff의 size·transform 값 → 새 hash로 동일 값 재적용(no-op)을 검증합니다. dry-run은 잠금 디렉터리도 만들지 않으며 기존 generation은 전 과정에서 보존합니다.
- 음수 크기, f32 overflow, 문자열·불리언, 필수 키 누락·추가 키를 두 모드 모두에서 거부합니다. 정상 Frame 변경 뒤 Fragment에 size 또는 transform을 지정하면 전체 transaction을 저장하지 않습니다.
- 같은 typed 명령을 텍스트와 기존 generation에 각각 저장한 뒤 실제 loader로 재구축한 전체 문서 레코드가 기대값과 같습니다. 크기·변환 외의 의미와 StableId도 함께 비교합니다. 기존 canonical command codec과의 왕복 테스트도 두 새 필드 입력을 포함합니다.

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `git fetch origin main` / `git merge --ff-only origin/main` | 0 — 시작 원격 a0313d2a, 이미 조상 |
| `git merge --ff-only 4d9a7b29` | 0 — 검토된 주석 보존 4052b5c6·hex 검증 4d9a7b29를 기존 commit 그대로 통합 |
| `git fetch origin main` / `git merge --ff-only --autostash origin/main` | 0 — 후속 원격 943efb19를 fast-forward, 미커밋 변경 자동 복원·충돌 없음. production·CLI 테스트 bytes는 통합 전과 같음 |
| `cargo test -p stellar-ops text_source --locked` | 0 — 3 passed |
| `cargo test -p stellar-project --test text_source --locked` | 0 — 6 passed |
| `cargo test -p stellar-cli --test text_source --locked` | 0 — 7 passed |
| `cargo test -p stellar-local-ipc --lib` | 101 — 기존 병렬 타이밍 3건 실패, 56 passed |
| `RUST_TEST_THREADS=1 cargo test -p stellar-local-ipc --lib --locked` | 0 — 59 passed |
| `RUST_TEST_THREADS=1 cargo xtask check-core` (4d9a7b29 위 변경) | 0 — fmt·clippy·workspace 3,709 passed/8 ignored·별도 doctest 54 passed |
| `cargo test -p stellar-cli --test text_source --locked` (943efb19 통합 후) | 0 — 7 passed |
| `RUST_TEST_THREADS=1 cargo xtask check-core` (943efb19 통합 후) | 0 — fmt·clippy·workspace 3,835 passed/8 ignored·별도 doctest 54 passed |
| `cargo xtask generate-ledger` | 0 — 계획서 원본에서 1,152행 생성, V.10.6은 reported_done으로 기록 |
| `target/debug/xtask sync-plan` / `validate-task-status` / `validate-task-report docs/task-reports/V.10.6.md` / `validate-adr` | 0 — 최신 통합 원장·보고서·ADR 검사 |
| `git merge --no-commit origin/main` (7f046069) | 1 — 도움말 fixture의 정렬 폭 충돌. 합쳐진 CLI의 실제 `--help` 출력으로 재생성해 해결 |
| `cargo run --quiet --locked -p stellar-cli -- --help` | 0 — text-source apply와 export-accessibility를 모두 포함한 도움말 재생성 |
| `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets --locked -- -D warnings` (7f046069 통합 후) | 0 |
| `RUST_TEST_THREADS=1 cargo test -p stellar-project -p stellar-ops -p stellar-cli --locked` (7f046069 통합 후) | 0 — 510 passed/1 ignored |
| `./scripts/macos/smoke.sh` (7f046069 통합 후) | 0 — 서명 없는 Debug 빌드·기본 smoke 전체 통과, 창 세션 68·메뉴 명령 56·키 매핑 101 검사 포함 |
| `git diff --check` / `git diff --cached --check` | 0 |

### Frame 숫자 편집 후속 검증

기준은 기존 `5d53de48` 위의 Frame 크기·변환 변경입니다.

| 명령 | 결과 |
|---|---|
| `git fetch origin main` / `git merge --no-edit origin/main` | 0 — 시작 시 7f046069가 이미 조상 |
| `cargo test -p stellar-ops frame_geometry --locked` | 0 — 텍스트/generation 전체 레코드 왕복 1 passed |
| `cargo test -p stellar-cli --test text_source --locked` | 0 — 실제 CLI/Git 경로 9 passed |
| `cargo test -p stellar-ops --locked` | 0 — 42 passed |
| `RUST_TEST_THREADS=1 cargo xtask check-core` | 0 — fmt·clippy·workspace 3,852 passed/8 ignored·별도 doctest 54 passed |
| `cargo xtask generate-ledger` / `cargo fmt --all -- --check` | 0 — 원본에서 1,152행 재생성, V.10.6 상태 유지 |
| `target/debug/xtask sync-plan --check` / `validate-task-status` / `validate-task-report docs/task-reports/V.10.6.md` / `validate-adr` / `validate-crate-graph` | 0 |
| `git fetch origin main` / `git merge --no-commit origin/main` (dfb11566) | fetch 0 / merge 1 — 같은 위치에 추가한 CLI 테스트 충돌. Frame 2개와 upstream 원본 rename 테스트 본문을 모두 그대로 보존해 해결 |
| `cargo test -p stellar-cli --test text_source --locked` (dfb11566 통합) | 0 — 원본 rename 포함 10 passed |
| `RUST_TEST_THREADS=1 cargo test -p stellar-ops -p stellar-cli -p stellar-mcp-core -p stellar-mcp --locked` (dfb11566 통합) | 0 — 204 passed, 0 failed, 0 ignored |
| `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets --locked -- -D warnings` (dfb11566 통합) | 0 |
| `./scripts/macos/smoke.sh` (dfb11566 통합) | 0 — 서명 없는 Debug 빌드와 기본 smoke 전체 통과, 창 세션 68·메뉴 명령 56·키 매핑 101 검사 포함 |

## 환경과 증거

- DEV-MAC, 이 전용 작업 트리에서 실행. CLI 로그: `/tmp/stellar-text-apply-cli.log`; IPC 직렬 로그: `/tmp/stellar-text-apply-ipc-serial.log`.
- 이전 회차의 전체 검사 실패는 `target/xtask-logs/test.log`에서 IPC `slow_authenticated_peer_does_not_block_other_handshakes`, `trickling_hello_times_out_as_a_whole`, `pending_handshakes_are_capped_and_resume_when_one_finishes`의 시간 초과로 확인했습니다. 이번 병렬 재현에서 각각 2.574초, 기한 400ms 대비 1.774초, TimedOut이었습니다. 코드·시간 상한·테스트 제외 목록을 수정하지 않았습니다.
- 새 테스트 최초 컴파일의 borrow/fixture 생성자·비교 타입 오류와 보고서 최초 검사의 필수 절 명칭 누락을 수정한 뒤 타깃·전체 검증과 보고서 검사를 통과했습니다.
- 통합 전 전체 Rust 검증의 원시 로그는 `target/xtask-logs/{fmt,clippy,test,doc-test}.log`에서 별도 외부 인계 경로로 보존했습니다. 943efb19 통합 후에도 전체 검증을 재실행해 통과했습니다. 로그는 `/tmp/stellar-text-apply-check-core-integrated.log`와 `target/xtask-logs/`에 있으며 원본 사본은 저장소 밖 인계 경로에 보존했습니다. 후속 7f046069 통합은 fmt·clippy·관련 Rust 전체 타깃과 기본 macOS smoke로 재검증했습니다. 최종 로그는 `/tmp/stellar-text-apply-final-{clippy,targets,macos}.log`입니다. 원격 CI와 LAB·REF 성능은 이번 검증 주장에 포함하지 않습니다.
- Frame 후속 착수 때의 17:37 전체 검사 실패도 같은 IPC 3건이었습니다(5.474초·3.289초·TimedOut). 당시 로그는 저장소 밖 `frame-geometry-validation-20261114/previous-failure/`에 보존했습니다. 새 타깃 로그는 `/tmp/stellar-frame-geometry-{ops,ops-all,cli}.log`입니다. IPC 코드·시간 상한·제외 목록은 변경하지 않았습니다.
- Frame 구현 커밋 `8043883b` 이후 원격 dfb11566을 병합했습니다. 두 production 입력 adapter 파일은 8043883b와 bytes가 같고, 충돌한 세 테스트 본문은 각 원본 Git 객체와 비교해 보존을 확인했습니다. 전체 Rust 검증 사본은 외부 `frame-geometry-validation-20261114/before-main-integration/`, 최종 통합 타깃·clippy·smoke 로그는 `/tmp/stellar-frame-geometry-integrated-*.log`입니다.

## 성능

측정하지 않았습니다. 텍스트 전체 파싱·검증·직렬화 경로이며 T0→T5나 기본 저장 전환의 성능 완료를 주장하지 않습니다.

## 차이

사용자가 요청한 수직 단위에 따라 V.10.6의 단일 파일 명령 경로를 먼저 연결하고 Frame 숫자 편집으로 확장했습니다. 전체 전환 선행 조건·완료 기준을 삭제하거나 완화하지 않으며 원장은 부분 상태입니다. V.10.1~V.10.3의 기존 원본 모델과 잠금·command·validator를 재사용합니다.

## 남은 위험

- V.10.6 전체의 선행·완료 기준이 남아 있어 verified로 승격하지 않습니다. 전체 의미 지원, 다중 파일 journal, 앱 기본 저장, clean/dirty 외부 변경 재로드·병합은 후속입니다.
- 실제 의미 편집 시 v1 결정론 writer는 주석·서식을 다시 쓰므로 보존하지 않습니다. 무변경 저장만 원본 bytes를 보존합니다.
- 같은 잠금을 사용하지 않는 외부 writer의 마지막 hash 검사와 rename 사이 경쟁은 기존 API의 한계입니다. 부모 fsync 실패는 게시 후 반환될 수 있어 모든 I/O 실패에서 미저장을 단정하면 안 됩니다. 오류 후 실제 파일을 다시 읽어야 합니다.
- 기본 병렬 IPC 검증 실패는 별도 추적 대상입니다. 직렬 검증 통과와 구분합니다. Frame 후속 단위의 전체 workspace 검증 기준은 7f046069를 포함한 5d53de48 위 변경이며, 최종 dfb11566 병합은 관련 Rust 204개·전체 clippy·macOS smoke로 검증했습니다. 최종 병합에서 전체 workspace 검사를 다시 실행한 것으로 보고하지 않습니다.
- 직접 push·main checkout·release·배포는 수행하지 않습니다.
