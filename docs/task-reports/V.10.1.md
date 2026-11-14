# 작업 보고서: V.10.1 단일 텍스트 원본 파일 I/O

## WBS

- WBS: V.10.1
- 작업명: 단일 텍스트 원본 파일 I/O
- 상태: 부분 완료 (`[~]`) — 구현·타깃 검증 완료, 전체 Rust·최종 main 통합 타깃 검증 완료, 커밋 증거 확정 중.
- 선행: V.2.5.

## 변경 파일

- `crates/storage/src/source_file.rs`: 제한 handle 읽기, 생성 충돌·기대 hash 검사, 기존 권한 보존, 신규 파일 hard link 게시, cache 교체·Git 제외 marker.
- `crates/storage/src/atomic_file.rs`, `lib.rs`: 기존 API를 유지한 temp handle 권한 전달·무교체 게시 경로와 모듈 등록.
- 전환 WBS·생성 원장, ADR-0059·문법 및 이 보고서.

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

- `cargo test -p stellar-storage source_file --lib`: 5개 통과. 생성 충돌·stale hash·저장 단계 fault·rename 직전 외부 변경·권한·cache marker 보존을 검사합니다.
- `cargo test -p stellar-storage atomic_file --lib`: 10개 통과. 게시 직전 생성된 기존 파일의 덮어쓰기 거부를 포함합니다.

## 차이

- ADR-0059의 첫 수직 단위 V.10.1~V.10.3을 파일 수 정책에 따라 세 WBS로 나눕니다. 추가 의존성·서비스·기본 저장 전환은 없습니다.
- 인계 코드를 검토하고 회귀 검사를 보강했습니다. 계획서가 원본이며 `generate-ledger`로 원장을 재생성합니다. 실제 WBS 커밋 생성 후 상태와 SHA를 갱신합니다.

## 남은 위험

- 기존 파일의 마지막 hash 검사와 rename 사이에 비협조적 writer 경쟁이 남습니다. 앱 연결 전 프로젝트 저장 잠금·외부 변경 경로 검증이 필요합니다.
- 새 파일은 hard link 미지원 파일 시스템에서 오류로 거부합니다. 게시 후 temp unlink·부모 fsync 오류이면 실제 원본을 다시 읽어야 합니다.
- 기본 병렬 Rust 검증의 기존 IPC 타이밍 실패는 별도 한계입니다. 시간 상한을 완화하거나 검사를 제외하지 않았습니다.
