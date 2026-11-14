# 작업 보고서: V.10.4 Text 왕복과 컴포넌트별 텍스트 원본

## WBS

- WBS: V.10.4
- 상태: 부분 구현 (`[~]`). 단일 View의 Text payload 전체 왕복과 빈 공개 인터페이스·중첩 Free Frame 컴포넌트의 독립 파일을 지원합니다. 전체 node·modifier·component·token·resource, 다중 파일 참조 및 마이그레이션은 완료 처리하지 않습니다.
- 기본 generation 저장 경로와 기존 Frame/Fragment 문법은 유지합니다.

## 변경 파일

- `crates/project/src/document_io/text_codec.rs`: Text 본문, 문단 style index, style run의 start/end/style을 명시적으로 encode/decode합니다. 알 수 없는 키, 누락, 잘못된 타입·범위를 거부합니다.
- `crates/project/tests/text_source.rs`: Text를 포함한 실제 모델·파일 왕복, cache 없음/손상/사용 불가, 잘못된 필드·긴 scalar·역전 범위의 파일 보존.
- `crates/ops/src/test_support.rs`: 기존 generation으로 저장할 Text 테스트 모델.
- `crates/ops/src/text_source.rs`: 기존 미지원 command 결과 테스트를 계속 미지원인 Shape로 교체.
- `tools/stellar-cli/tests/text_source.rs`: 실제 export→rename dry-run/write→check→Git diff, 외부 본문 수정 재열기, 오류 시 원본 보존.
- `docs/spec/text-view-source-v1.md`, ADR-0059, 구현 계획 원본, 생성 원장, V.10.6 보고서의 과거 fixture 설명 및 이 보고서.

컴포넌트 독립 원본 통합 추가 파일:

- `crates/project/src/document_io/component_source.rs`: 이름/StableId 파일명, 제한 YAML codec, 실제 `ComponentLibrary` 재조립, 단일 파일 조건부 저장. 기존 `text_codec.rs`의 검증 helper를 공유합니다.
- `crates/ops/src/component_source.rs`: 디렉터리 저장 잠금과 create/check/resize.
- `tools/stellar-cli/src/component_source.rs` 및 각 crate 모듈 연결·CLI 도움말: 명시적 opt-in 진입점.
- `tools/stellar-cli/tests/component_source.rs`: 실제 프로젝트 generation과 두 정의 파일을 사용하는 Git 병합·재열기·원본 보호 검증.
- `docs/spec/component-source-v1.md`, CLI 가이드, ADR-0059, 구현 계획 원본과 생성 원장: 지원 범위·호환 절차.

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

컴포넌트 독립 원본 통합 검증(64761f58 + 75e21d08):

| 명령 | 결과 |
|---|---|
| `CARGO_BUILD_JOBS=4 CARGO_NET_OFFLINE=true cargo test -p stellar-project -p stellar-ops -p stellar-cli --locked` | 0 — 527 passed, 0 failed, 기존 2 ignored; 최초 lane 테스트 2개 포함 |
| `cargo test -p stellar-cli --test component_source --locked` (보강·독립 재검토 후) | 0 — 실제 프로젝트 Git 흐름·오류 보존 3개 통과 |
| `CARGO_BUILD_JOBS=4 RUST_TEST_THREADS=4 CARGO_NET_OFFLINE=true cargo xtask check-core` | 65 — fmt/clippy 통과, 기존 IPC `trickling_hello_times_out_as_a_whole` elapsed 상한 실패(58 passed/1 failed) |
| `cargo test -p stellar-local-ipc listener::tests::trickling_hello_times_out_as_a_whole --locked -- --exact` | 0 — 단독 실행 1개 통과 |
| `CARGO_BUILD_JOBS=4 RUST_TEST_THREADS=1 CARGO_NET_OFFLINE=true cargo xtask check-core` | 0 — fmt/clippy 경고 0, workspace 3,921 passed/10 ignored, 별도 doctest 54 passed |
| `CARGO_BUILD_JOBS=4 CARGO_NET_OFFLINE=true ./scripts/macos/smoke.sh` | 0 — smoke OK; window-session 68, session-commands 82, window-close 98, key-mapping 101 |
| `target/debug/xtask sync-plan --check` / `validate-task-status` / `validate-adr` / `validate-crate-graph` / `validate-task-report docs/task-reports/V.10.4.md` / `git diff --check` | 0 |

## 환경과 증거

이전 Text 단위의 검증 기록: DEV-MAC, macOS 27.2 (26B123), arm64, Rust 1.96.0, Apple Git 2.54.0. 이 전용 작업 트리에서 최신 origin/main `c881c97e86c0a5db6efbc3441129f06db650346a`를 먼저 병합한 `ea4cc68` 위에서 구현했습니다. 초기 실패 로그는 `/tmp/stellar-text-node-red.log`, project 로그는 `/tmp/stellar-text-node-project.log`, 통합 타깃 로그는 `/tmp/stellar-text-node-targeted.log`입니다. 초기 테스트 작성의 import·참조 타입 오류와 빈 CLI 명령 요청 오류는 수정한 뒤 다시 검증했습니다. 스레드 4 전체 검사에서는 기존 `pending_handshakes_are_capped_and_resume_when_one_finishes`가 `listener.rs:1089`에서 TimedOut으로 실패했습니다(IPC 58 passed/1 failed). 단독 실행은 4.53초에 통과했습니다. 실패 로그는 프로젝트 밖 `text-node-validation-20261114/threads-4-failure/`에 보존했으며 IPC 코드·시간 상한·제외 목록은 변경하지 않았습니다. 이후 같은 소스의 전체 직렬 검사는 통과했으며, 원시 로그는 같은 외부 디렉터리의 `threads-1-success/`에 보존했습니다. 이 전체 Rust 검사의 기준은 c881c97e 포함 상태의 제품 커밋 `4670b5a4`입니다. 이후 `origin/main` bc5b2db2를 병합했고 Text codec·project/CLI 회귀 본문은 Git 객체 비교로 동일함을 확인했습니다. 최종 통합은 관련 Rust 6개 crate의 660개 테스트, 전체 clippy·fmt·기본 macOS smoke로 검증했습니다. 로그는 외부 `text-node-validation-20261114/final-bc5b2db2/`에 보존합니다. 최종 병합에서 전체 workspace 검사를 다시 실행했다고 주장하지 않으며 원격 CI는 이번 회차에 실행하지 않았습니다.

컴포넌트 단위는 DEV-MAC(macOS 27.2/26B123, Rust 1.96.0, Apple Git 2.54.0)에서 검증했습니다. fetch한 원격 `64761f58`과 전용 브랜치가 같음을 확인하고 원본 lane `75e21d08`을 이력 보존 병합했습니다. 원래 lane은 clean이며 수정하지 않았습니다. 최초 보강 검사의 root ID 문자열 치환은 YAML 인용 표기를 잘못 가정하여 assertion에 실패했고, fixture를 고친 뒤 3개 모두 통과했습니다. 원시 로그는 저장소 밖 `component-source-integration-20261114-1930/`에 보관합니다. macOS smoke는 기존 셸 회귀 검사이며 컴포넌트 원본을 실제 앱에서 편집한 증거는 아닙니다. 전체 스레드 4 검사는 기존 IPC 테스트가 400ms timeout의 3배인 1.2초 관측 상한을 넘어 1.484002초를 기록하여 실패했습니다. `HandshakeFailed(TimedOut)`과 I/O thread 확인은 통과한 뒤 elapsed 검사에서 실패했고 단독 실행은 통과했습니다. 측정은 connect 직후부터 시작하지만 deadline은 인증 완료 후 설정되므로 인증·스케줄링 시간이 섞입니다. 이번 로그만으로 지연의 정확한 원인을 확정하지 않습니다. IPC 소스는 원격 기준과 동일하며 timeout·인증 정책이나 테스트 상한을 바꾸지 않았습니다. 실패 로그는 외부 검증 폴더의 `threads-4-failure/`에 보존합니다. 최종 보강 테스트를 포함한 전체 직렬 검사는 종료 코드 0이며 원시 fmt/clippy/test/doc-test 로그는 `threads-1-success/`에 보관합니다.

## 테스트

- 본문을 정규화하거나 CR/CRLF를 바꾸지 않으며 한글, 조합 문자, emoji ZWJ, NUL, 탭, 인용 부호를 동일 레코드로 재구축합니다. 빈 본문과 `u32::MAX` style index도 보존합니다.
- 문단은 LF 개수+1, style range는 Unicode scalar offset입니다. 뒤섞인 순서·겹치는 run·빈 run을 정렬·병합하지 않습니다. `TextRange` public 필드로 만든 역전 범위도 encode 단계에서 거부합니다.
- Text를 포함한 generation을 CLI export한 뒤 Text 이름을 dry-run/저장합니다. 본문·문단·style run bytes가 그대로 남고 Git diff에는 이름 변경만 나타납니다. 재열기 hash가 저장 결과와 같고 원본 generation 전체 bytes는 보존됩니다.
- 외부에서 본문을 수정한 원본을 check 후 새 hash로 다시 적용하면 무변경 저장이 bytes를 보존합니다. unknown key, 누락, 문단 수 불일치, 역전·초과 범위는 check와 dry-run/write에서 거부하고 원본을 보존합니다.
- cache 없음·손상은 원본 Text에서 재구축하며 cache I/O 실패에도 Text를 포함한 문서를 엽니다. 긴 scalar와 역전 범위 저장 실패는 기존 파일을 바꾸거나 새 파일을 만들지 않습니다.

컴포넌트 추가 검증:

- 실제 저장 generation을 가진 프로젝트에서 동일 표시 이름의 두 정의를 CLI로 만들고 각 브랜치에서 한 root의 크기만 바꿉니다. `git diff --name-only`와 다른 파일의 직접 bytes 비교로 독립 편집을 확인합니다. 일반 `git merge --no-ff` 후 새 clone에서 cache 없이 두 정의의 전체 component/root ID·크기·source hash가 재현되고 기존 프로젝트 설정과 generation 파일 hash가 모두 유지됩니다.
- Frame의 transform·색 공간·테두리·모서리·표시·잠금은 텍스트 왕복 전후 실제 library canonical bytes 비교로 확인합니다.
- 같은 ID·root와 component/root 교차 충돌, stale hash, 이름/접미사 불일치를 거부합니다. 무변경 resize는 주석을 포함한 원본 bytes를 유지합니다.
- component 내부 Property·Slot·state·token, root_node에 직접 삽입한 미정의 필드, root ID 불일치와 parent 참조는 `check`·`resize`·새 정의 생성에서 거부합니다. 현재 source hash로 요청해도 거부하며 폴더 전체 파일 hash가 같습니다.

## 중첩 Frame 정의 파일

`component-source add-frame`은 기존 hash를 확인한 다음 지정한 부모에 마지막 자식 Frame을 추가합니다. 기존 root-only 파일은 v1 bytes를 유지하고 실제 자식 Frame이 있는 파일만 v2가 됩니다. 모든 자식의 id·parent·order_key와 payload를 복원해 실제 `ComponentLibrary`로 조립합니다. 생성과 크기 편집은 파일 하나에 적용합니다.

- `cargo test -p stellar-cli --test component_source`로 v1 병합, v2 형제·손자 생성과 이름 변경 후 새 Git clone, 라이브러리 ID 중복·미지원 입력 보존을 검사합니다.
- `cargo test -p stellar-project frame_and_component_library_bytes_survive_a_source_round_trip`으로 YAML 왕복 전후 라이브러리 binary와 재사용 인스턴스 두 개의 자식 순서·손자 payload를 비교합니다.

## 성능

측정하지 않았습니다. 전체 텍스트 파싱·검증 경로이며 T0→T5·빠른 열기·기본 저장 전환 성능 완료를 주장하지 않습니다.

## 차이

이번 컴포넌트 단위는 검증 가능한 기존 lane `75e21d08`을 이력 보존 통합하고 실제 프로젝트 보존·미지원 레코드 회귀를 보강했습니다. 아래의 이전 Text 구현 증거를 보존하며 전체 V.10.4 상태는 올리지 않습니다.

사용자의 한 수직 단위 구현 요청에 따라 전체 V.10.4를 Text payload 하나로 나눴습니다. 선행·전체 완료 조건은 그대로 남기고 부분 상태로 기록합니다. 추가 의존성, 서비스, 바이너리 저장 opcode, 기본 저장 변경은 없습니다. 원장은 구현 계획 원본에서 `generate-ledger`로 재생성합니다.

## 남은 위험

- Text 본문 전용 CLI command와 앱 기본 편집·렌더 경로는 이번 변경에 포함하지 않습니다. 본문은 외부 YAML 편집, 기존 CLI 명령은 이름·표시 여부 등에 사용할 수 있습니다.
- style index를 원본 그대로 보존하지만 외부 font/style 정의를 생성하거나 해결하지 않습니다. 추가 chunk가 필요한 프로젝트는 기존 전체 pointer 검사로 export를 거부합니다.
- 1 MiB 파일·32 Ki scanner 선읽기 제한을 유지합니다. 긴 본문 분할은 후속이며 제한을 넘는 출력은 쓰기 전에 거부합니다.
- 기존 단일 파일의 비협조적 writer 경쟁·게시 후 fsync 오류 한계, 여러 파일 journal·전체 의미·기본 앱 전환 미완료는 유지합니다.
- 앞선 Text 구현 당시 스레드 4의 IPC timing 실패는 위 역사적 증거로 보존합니다. 이후 `757f246b`에서 대기 슬롯 검사를 상태 기반으로 수정한 이력이 이번 기준 `64761f58`에 포함됩니다.
- 원격 push·main checkout·release·배포는 수행하지 않습니다. 원격 CI와 REF 환경 성능은 이번 검증으로 주장하지 않습니다.

컴포넌트 범위의 남은 한계:

- 빈 공개 인터페이스와 Free Frame tree를 지원합니다. 정의 사이의 instance 참조·Property·Slot·토큰·상태 머신과 기존 generation의 전체 라이브러리 변환은 제공하지 않습니다. 미지원 내용을 생략하는 변환 경로도 없습니다.
- 이 opt-in 폴더를 기본 앱 라이브러리로 여는 경로와 컴포넌트 CLI dry-run은 없습니다. `resize`는 기대 hash에 대한 단일 파일 저장만 제공합니다. V.10.5 journal·여러 파일 rename 원자성·V.10.7 기본 전환은 별도입니다.
- cache를 쓰지 않고 각 파일을 해석하므로 cache 가속이나 T0→T5 성능 완료를 주장하지 않습니다. 파일당 1 MiB·폴더당 4,096개 상한과 기존 비협조적 외부 writer 경쟁·게시 후 fsync 실패 한계를 유지합니다.
- 이번 전체 스레드 4 실행의 IPC 벽시계 상한 실패는 잔여 검증 한계입니다. 단독·전체 직렬 결과와 구분하며 병렬 안정성 완료로 표시하지 않습니다.
