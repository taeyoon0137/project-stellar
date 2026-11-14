# 작업 보고서: V.10.4 Text 노드의 텍스트 원본 왕복

## WBS

- WBS: V.10.4
- 상태: 부분 구현 (`[~]`). 단일 View에 Text payload의 전체 필드를 추가한 수직 단위입니다. 전체 node·modifier·component·token·resource, 다중 파일 참조 및 마이그레이션은 완료 처리하지 않습니다.
- 기본 generation 저장 경로와 기존 Frame/Fragment 문법은 유지합니다.

## 변경 파일

- `crates/project/src/document_io/text_codec.rs`: Text 본문, 문단 style index, style run의 start/end/style을 명시적으로 encode/decode합니다. 알 수 없는 키, 누락, 잘못된 타입·범위를 거부합니다.
- `crates/project/tests/text_source.rs`: Text를 포함한 실제 모델·파일 왕복, cache 없음/손상/사용 불가, 잘못된 필드·긴 scalar·역전 범위의 파일 보존.
- `crates/ops/src/test_support.rs`: 기존 generation으로 저장할 Text 테스트 모델.
- `crates/ops/src/text_source.rs`: 기존 미지원 command 결과 테스트를 계속 미지원인 Shape로 교체.
- `tools/stellar-cli/tests/text_source.rs`: 실제 export→rename dry-run/write→check→Git diff, 외부 본문 수정 재열기, 오류 시 원본 보존.
- `docs/spec/text-view-source-v1.md`, ADR-0059, 구현 계획 원본, 생성 원장, V.10.6 보고서의 과거 fixture 설명 및 이 보고서.

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-project --test text_source actual_model_edit_save_reopen_and_readable_git_diff --locked` (구현 전) | 101 — 실제 Text payload 미지원으로 실패 확인 |
| `cargo test -p stellar-project --test text_source --locked` | 0 — Text 포함 원본·캐시·보존 8개 통과 |
| `cargo test --offline -p stellar-cli --test text_source` | 0 — CLI 통합 12개 통과 |
| `cargo test -p stellar-project -p stellar-ops -p stellar-cli --locked` | 0 — 518 passed, 0 failed, 기존 수동 폰트 검사 1 ignored |
| `RUST_TEST_THREADS=4 cargo xtask check-core` | 65 — fmt/clippy 통과, 기존 IPC pending handshake 테스트 1건 TimedOut |
| `cargo test -p stellar-local-ipc listener::tests::pending_handshakes_are_capped_and_resume_when_one_finishes --locked -- --exact` | 0 — 실패한 테스트 단독 실행 통과 |
| `RUST_TEST_THREADS=1 cargo xtask check-core` | 0 — fmt/clippy, workspace 3,877 passed/0 failed/8 ignored, 별도 doctest 54 passed |
| `RUST_TEST_THREADS=4 cargo test -p stellar-project -p stellar-ops -p stellar-cli -p stellar-codegen -p stellar-mcp-core -p stellar-mcp --locked` (bc5b2db2 병합 후) | 0 — 660 passed, 0 failed, 기존 3 ignored |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` (최종 통합) | 0 |
| `cargo fmt --all -- --check` / `bash scripts/macos/smoke.sh` (최종 통합) | 0 — 서명 없는 기본 smoke, 창 세션68·명령56·닫기98·키 매핑101 포함 |
| `cargo xtask generate-ledger` | 0 — 계획서에서 1,152행 재생성 |
| `target/debug/xtask validate-task-status` / `sync-plan` | 0 |
| `target/debug/xtask validate-task-report docs/task-reports/V.10.4.md` / `V.10.6.md` | 0 |
| `target/debug/xtask validate-adr crates/project/src/document_io/text_codec.rs docs/adr/ADR-0059-text-source-files-with-chunk-cache.md` | 0 |
| `target/debug/xtask validate-crate-graph` / `git diff --check` | 0 |

## 환경과 증거

DEV-MAC: macOS 27.2 (26B123), arm64, Rust 1.96.0, Apple Git 2.54.0. 이 전용 작업 트리에서 최신 origin/main `c881c97e86c0a5db6efbc3441129f06db650346a`를 먼저 병합한 `ea4cc68` 위에서 구현했습니다. 초기 실패 로그는 `/tmp/stellar-text-node-red.log`, project 로그는 `/tmp/stellar-text-node-project.log`, 통합 타깃 로그는 `/tmp/stellar-text-node-targeted.log`입니다. 초기 테스트 작성의 import·참조 타입 오류와 빈 CLI 명령 요청 오류는 수정한 뒤 다시 검증했습니다. 스레드 4 전체 검사에서는 기존 `pending_handshakes_are_capped_and_resume_when_one_finishes`가 `listener.rs:1089`에서 TimedOut으로 실패했습니다(IPC 58 passed/1 failed). 단독 실행은 4.53초에 통과했습니다. 실패 로그는 프로젝트 밖 `text-node-validation-20261114/threads-4-failure/`에 보존했으며 IPC 코드·시간 상한·제외 목록은 변경하지 않았습니다. 이후 같은 소스의 전체 직렬 검사는 통과했으며, 원시 로그는 같은 외부 디렉터리의 `threads-1-success/`에 보존했습니다. 이 전체 Rust 검사의 기준은 c881c97e 포함 상태의 제품 커밋 `4670b5a4`입니다. 이후 `origin/main` bc5b2db2를 병합했고 Text codec·project/CLI 회귀 본문은 Git 객체 비교로 동일함을 확인했습니다. 최종 통합은 관련 Rust 6개 crate의 660개 테스트, 전체 clippy·fmt·기본 macOS smoke로 검증했습니다. 로그는 외부 `text-node-validation-20261114/final-bc5b2db2/`에 보존합니다. 최종 병합에서 전체 workspace 검사를 다시 실행했다고 주장하지 않으며 원격 CI는 이번 회차에 실행하지 않았습니다.

## 테스트

- 본문을 정규화하거나 CR/CRLF를 바꾸지 않으며 한글, 조합 문자, emoji ZWJ, NUL, 탭, 인용 부호를 동일 레코드로 재구축합니다. 빈 본문과 `u32::MAX` style index도 보존합니다.
- 문단은 LF 개수+1, style range는 Unicode scalar offset입니다. 뒤섞인 순서·겹치는 run·빈 run을 정렬·병합하지 않습니다. `TextRange` public 필드로 만든 역전 범위도 encode 단계에서 거부합니다.
- Text를 포함한 generation을 CLI export한 뒤 Text 이름을 dry-run/저장합니다. 본문·문단·style run bytes가 그대로 남고 Git diff에는 이름 변경만 나타납니다. 재열기 hash가 저장 결과와 같고 원본 generation 전체 bytes는 보존됩니다.
- 외부에서 본문을 수정한 원본을 check 후 새 hash로 다시 적용하면 무변경 저장이 bytes를 보존합니다. unknown key, 누락, 문단 수 불일치, 역전·초과 범위는 check와 dry-run/write에서 거부하고 원본을 보존합니다.
- cache 없음·손상은 원본 Text에서 재구축하며 cache I/O 실패에도 Text를 포함한 문서를 엽니다. 긴 scalar와 역전 범위 저장 실패는 기존 파일을 바꾸거나 새 파일을 만들지 않습니다.

## 성능

측정하지 않았습니다. 전체 텍스트 파싱·검증 경로이며 T0→T5·빠른 열기·기본 저장 전환 성능 완료를 주장하지 않습니다.

## 차이

사용자의 한 수직 단위 구현 요청에 따라 전체 V.10.4를 Text payload 하나로 나눴습니다. 선행·전체 완료 조건은 그대로 남기고 부분 상태로 기록합니다. 추가 의존성, 서비스, 바이너리 저장 opcode, 기본 저장 변경은 없습니다. 원장은 구현 계획 원본에서 `generate-ledger`로 재생성합니다.

## 남은 위험

- Text 본문 전용 CLI command와 앱 기본 편집·렌더 경로는 이번 변경에 포함하지 않습니다. 본문은 외부 YAML 편집, 기존 CLI 명령은 이름·표시 여부 등에 사용할 수 있습니다.
- style index를 원본 그대로 보존하지만 외부 font/style 정의를 생성하거나 해결하지 않습니다. 추가 chunk가 필요한 프로젝트는 기존 전체 pointer 검사로 export를 거부합니다.
- 1 MiB 파일·32 Ki scanner 선읽기 제한을 유지합니다. 긴 본문 분할은 후속이며 제한을 넘는 출력은 쓰기 전에 거부합니다.
- 기존 단일 파일의 비협조적 writer 경쟁·게시 후 fsync 오류 한계, 여러 파일 journal·전체 의미·기본 앱 전환 미완료는 유지합니다.
- 스레드 4의 전체 검사에서 재현된 기존 IPC timing 실패는 미해결이며 직렬 검증과 구분합니다.
- 원격 push·main checkout·release·배포는 수행하지 않습니다. 원격 CI와 REF 환경 성능은 이번 검증으로 주장하지 않습니다.
