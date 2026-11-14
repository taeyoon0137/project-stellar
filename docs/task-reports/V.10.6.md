# 작업 보고서: V.10.6 단일 텍스트 원본 CLI 명령 저장

## WBS

- WBS: V.10.6
- 상태: 부분 구현 (`[~]`). 이번 단위는 opt-in 단일 View의 `text-source apply`입니다. 선행 V.10.5·XIII.3.1과 전체 파일 감시·dirty 재로드·병합은 완료 처리하지 않습니다.
- 필수 환경: DEV-MAC.
- 기본 프로젝트 열기·generation 저장 경로는 유지합니다.

## 변경 파일

직접 작성한 production 파일은 아래 5개이며 새 의존성은 없습니다.

- `crates/ops/src/text_source.rs`: 원본 hash·문서 ID·필수 저장 잠금, 기존 command transaction 및 semantic diff, 조건부 저장. export/check에 document ID 응답 추가.
- `crates/project/src/document_io/text_source.rs`: 파일 I/O 없이 실제 codec으로 결과 모델·크기를 검사하는 API.
- `tools/stellar-cli/src/apply.rs`: generation apply와 텍스트 apply가 입력 제한·엄격한 JSON envelope·closed command 파서를 공유.
- `tools/stellar-cli/src/text_source.rs`: 별도 `stellar-text-apply` 입력, 파일/stdin, dry-run/write와 JSON 결과·오류 연결.
- `tools/stellar-cli/src/cli.rs`: 실제 명령 도움말.
- `tools/stellar-cli/tests/text_source.rs`, `tests/fixtures/help.txt`: CLI 프로세스·임시 Git 저장소 통합 검증과 도움말 fixture.
- 문법·ADR-0059·기술 아키텍처·구현 계획·생성 원장 및 이 보고서.

## 테스트

- 실제 기존 generation을 export하고 `check`에서 얻은 source hash와 문서 ID로 두 명령(rename, visible)을 요청합니다. 파일 입력 dry-run → stdin 쓰기 → check 재열기 → 실제 `git diff`에서 이름·visible 속성 변경을 검사합니다. 기존 generation 전체 bytes는 유지합니다.
- dry-run은 semantic diff를 반환하지만 source·generation·cache·저장 잠금 파일을 생성하지 않습니다. source_hash는 변경하지 않은 원본 bytes hash입니다.
- 쓰기는 문서 ID + canonical 파일 경로의 기존 OS 저장 잠금을 사용합니다. 잠금 뒤 원본을 열고 expected hash와 ID를 확인합니다. 저장 전 재해석·hash 확인과 temp/fsync/rename은 기존 저장 계층을 재사용합니다.
- 같은 값 적용은 주석·bytes·hash를 보존합니다. 주석만 외부 변경해도 stale hash는 거부합니다. unknown 원본 필드, 잘못된 문서, unknown command, 중간 명령 실패는 원본을 보존합니다.
- canonical InsertNode로 미지원 Text 노드가 생기는 경우 dry-run/write 모두 거부합니다. 이름 변경의 semantic diff와 저장 후 재구축 모델이 일치하는지 ops 테스트로도 검사합니다.
- 저장 잠금 경합은 write를 거부하고 dry-run은 성공합니다. 잠금을 제공하지 않은 write도 거부합니다.

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
| `git diff --check` | 0 |

## 환경과 증거

- DEV-MAC, 이 전용 작업 트리에서 실행. CLI 로그: `/tmp/stellar-text-apply-cli.log`; IPC 직렬 로그: `/tmp/stellar-text-apply-ipc-serial.log`.
- 이전 회차의 전체 검사 실패는 `target/xtask-logs/test.log`에서 IPC `slow_authenticated_peer_does_not_block_other_handshakes`, `trickling_hello_times_out_as_a_whole`, `pending_handshakes_are_capped_and_resume_when_one_finishes`의 시간 초과로 확인했습니다. 이번 병렬 재현에서 각각 2.574초, 기한 400ms 대비 1.774초, TimedOut이었습니다. 코드·시간 상한·테스트 제외 목록을 수정하지 않았습니다.
- 새 테스트 최초 컴파일의 borrow/fixture 생성자·비교 타입 오류와 보고서 최초 검사의 필수 절 명칭 누락을 수정한 뒤 타깃·전체 검증과 보고서 검사를 통과했습니다.
- 통합 전 전체 Rust 검증의 원시 로그는 `target/xtask-logs/{fmt,clippy,test,doc-test}.log`에서 별도 외부 인계 경로로 보존했습니다. 943efb19 통합 후에도 전체 검증을 재실행해 통과했습니다. 로그는 `/tmp/stellar-text-apply-check-core-integrated.log`와 `target/xtask-logs/`에 있으며 원본 사본은 저장소 밖 인계 경로에 보존했습니다. Swift smoke, 원격 CI, LAB·REF 성능은 이 단위의 검증 주장에 포함하지 않습니다.

## 성능

측정하지 않았습니다. 텍스트 전체 파싱·검증·직렬화 경로이며 T0→T5나 기본 저장 전환의 성능 완료를 주장하지 않습니다.

## 차이

사용자가 요청한 이번 수직 단위에 따라 V.10.6의 단일 파일 명령 경로를 먼저 연결했습니다. 전체 전환 선행 조건·완료 기준을 삭제하거나 완화하지 않으며 원장은 부분 상태입니다. V.10.1~V.10.3의 기존 원본 모델과 잠금·command·validator를 재사용하고, 프로덕션 파일 5개 범위를 지킵니다.

## 남은 위험

- V.10.6 전체의 선행·완료 기준이 남아 있어 verified로 승격하지 않습니다. 전체 의미 지원, 다중 파일 journal, 앱 기본 저장, clean/dirty 외부 변경 재로드·병합은 후속입니다.
- 실제 의미 편집 시 v1 결정론 writer는 주석·서식을 다시 쓰므로 보존하지 않습니다. 무변경 저장만 원본 bytes를 보존합니다.
- 같은 잠금을 사용하지 않는 외부 writer의 마지막 hash 검사와 rename 사이 경쟁은 기존 API의 한계입니다. 부모 fsync 실패는 게시 후 반환될 수 있어 모든 I/O 실패에서 미저장을 단정하면 안 됩니다. 오류 후 실제 파일을 다시 읽어야 합니다.
- 기본 병렬 IPC 검증 실패는 별도 추적 대상입니다. 직렬 검증 통과와 구분합니다.
- 직접 push·main checkout·release·배포는 수행하지 않습니다.
