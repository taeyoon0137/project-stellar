# 작업 보고서: V.10.3 기존 프로젝트의 명시적 텍스트 export와 CLI 검사

## WBS

- WBS: V.10.3
- 작업명: 기존 프로젝트의 명시적 텍스트 export와 CLI 검사
- 상태: 부분 완료 (`[~]`) — 구현·DEV-MAC 검증·커밋은 완료했지만 선행 XXIV.1.3이 `[~]`이므로 verified 승격을 보류합니다.
- 구현 커밋: `0a26ffe3f3fa6022a096bb5e05c8130d22fe623c`.
- 선행: V.10.2, XXIV.1.3.

## 변경 파일

- `crates/ops/src/{text_source,lib}.rs`: 실제 generation의 전체 pointer 집합 비교와 export/check 결과.
- `tools/stellar-cli/src/{text_source,cli,lib}.rs`: ops를 통한 명령·JSON·종료 코드 연결.
- `tools/stellar-cli/tests/text_source.rs`, `tests/fixtures/help.txt`: 실제 binary 정상·실패 경로와 도움말.
- 전환 WBS·생성 원장 및 이 보고서.

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-storage -p stellar-project -p stellar-ops -p stellar-cli --locked` | 0 — 768 passed, 0 failed, 수동 폰트 검사 1 ignored (a026e41c 기준) |
| `cargo test -p stellar-project --test text_source --locked` | 0 — symlink 수정 저장 거부 보강 후 5개 통과 |
| `cargo test -p stellar-cli --test text_source --locked` | 0 — 미지원 다중 문서 export 거부 포함 3개 통과 |
| `cargo xtask check-core` | 65 — a026e41c 기준 IPC 타이밍 1개 실패, fmt·clippy 통과 |
| `cargo test -p stellar-local-ipc --locked` | 101 — 58 passed, 동일 IPC 타이밍 1개 실패 |
| `cargo test -p stellar-local-ipc listener::tests::trickling_hello_times_out_as_a_whole --locked -- --exact` | 0 — 단독 검사 통과 |
| `RUST_TEST_THREADS=1 cargo xtask check-core` | 0 — eac4531b에서 fmt·clippy·workspace·doctest 전체 통과 |
| `RUST_TEST_THREADS=1 cargo test -p stellar-history -p stellar-project -p stellar-ops -p stellar-cli --locked` | 0 — a0313d2a 최종 통합 539 passed, 1 ignored |
| `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 — a0313d2a 최종 통합 상태 |
| `target/debug/xtask generate-ledger` | 0 — 계획서 원본에서 1,152행 생성 |
| `target/debug/xtask validate-task-status` / `sync-plan` | 0 |
| `target/debug/xtask validate-adr <storage 변경 경로> docs/adr/ADR-0059-text-source-files-with-chunk-cache.md` | 0 |
| `target/debug/xtask validate-task-report docs/task-reports/V.10.N.md` (N=1,2,3) | 0 |
| `target/debug/xtask verify-task V.10.3` | 65 — missing-dependency: XXIV.1.3이 verified가 아님 |
| `git diff --check` | 0 |

## 환경과 증거

- 필수 환경: DEV-MAC. macOS 27.2 (26B123), arm64, Rust 1.96.0, Apple Git 2.54.0.
- 기존 미커밋 구현을 보존하고 `a026e41c`, 이후 `eac4531b`와 `a0313d2a`까지 fast-forward했습니다. 이전 Git·IPC·FSEvents 권한 실패는 이번 실행에서 재현되지 않았습니다.
- 타깃 로그: `/tmp/stellar-text-source-targeted-20261114.log`, `/tmp/stellar-text-source-symlink-20261114.log`, `/tmp/stellar-text-source-cli-20261114.log`.
- 전체 검증 로그: `target/xtask-logs/{fmt,clippy,test,doc-test}.log`, `/tmp/stellar-text-source-check-core-integrated-20261114.log`, `/tmp/stellar-text-source-final-integration-20261114.log`.
- eac4531b 전체 검증: workspace 단계 3,700 passed·8 ignored, 별도 doctest 54 passed. a0313d2a의 추가 변경은 history·수동 CI·계획 문구이므로 해당 crate와 영향받는 project/ops/CLI를 다시 검증합니다.
- 기본 병렬 실행은 IPC `trickling_hello_times_out_as_a_whole`의 1.2초 상한에서 1.362초, 재실행 1.264초로 실패했습니다. 단독 실행은 통과했습니다. IPC 코드·기준·테스트 제외 목록은 변경하지 않았습니다.
- Swift smoke·원격 CI·REF 성능은 이번 변경의 검증 증거에 포함하지 않습니다.

## 성능

- 측정하지 않았습니다. T0→T5 또는 빠른 열기 성능 완료로 표시하지 않습니다.

## 테스트

- 실제 저장된 4노드 문서를 CLI export→check로 열어 source hash·node 수와 기존 generation 전체 bytes 불변을 확인합니다.
- 기존 출력 충돌 73, 없는 파일 66, 손상 텍스트 65, 미지원 다중 문서 69를 검사합니다. 실패 시 입력 bytes와 기존 출력은 보존되고 새 출력은 생성되지 않습니다.
- ops 회귀는 unknown optional·unused known chunk, pointer schema/ID·root·generation schema 불일치 6가지 사례를 export 전에 거부하는지 확인합니다.
- 새 CLI 미지원 입력 검사는 결정론적 두 문서를 실제 generation으로 저장한 뒤 binary를 실행합니다.

## 차이

- ADR-0059의 첫 수직 단위 V.10.1~V.10.3을 파일 수 정책에 따라 세 WBS로 나눕니다. 추가 의존성·서비스·기본 저장 전환은 없습니다.
- 인계 코드를 검토하고 회귀 검사를 보강했습니다. 계획서가 원본이며 `generate-ledger`로 원장을 재생성합니다. 구현 커밋과 DEV-MAC 검증은 이 보고서에 기록하지만, 선행 XXIV.1.3이 미완료여서 원장은 reported_done으로 유지합니다. 사용자 지시에 따라 인계 구현을 검증·커밋했고 선행 완료 정책이나 의존 관계는 변경하지 않았습니다.

## 남은 위험

- 원장 승격은 XXIV.1.3의 선행 Phase XIV와 Semantic Merge 결과 저장 경로가 완료된 뒤 별도 확인합니다. 이 회차에서 선행 작업을 완료 처리하거나 범위를 확장하지 않았습니다.

- CLI는 export/check만 제공합니다. 텍스트 파일의 apply/dry-run 연결은 V.10.6 후속 범위입니다.
- 기본 generation 저장은 유지하며 프로젝트 metadata·asset·dependency 전체 이전 기능은 아닙니다.
- 기본 병렬 Rust 검증의 기존 IPC 타이밍 실패는 별도 한계입니다. 시간 상한을 완화하거나 검사를 제외하지 않았습니다.
