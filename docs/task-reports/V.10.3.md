# 작업 보고서: V.10.3 기존 프로젝트의 명시적 텍스트 export와 CLI 검사

## WBS

- WBS: V.10.3
- 작업명: 기존 generation의 명시적 export/check 연결, 전체 pointer 집합 비교로 미지원 chunk 거부
- 상태: 부분 완료 (`[~]`) — 검증 열을 다시 실행했습니다. `[x]` 판정은 선행 XXIV.1.3의 `[x]` 여부(ledger-verification lane)와 check-core의 무관한 타이밍 실패 정리에 따릅니다. 차단 보고서 [V.10.3-blocked.md](V.10.3-blocked.md)의 재개 조건 가운데 검증 열 재실행에 해당합니다.
- 선행: V.10.2 `[x]`, XXIV.1.3(`reported_done`)
- 관련 커밋: `58709614`(쓰기 버전 v4), `ae935137`(다중 파일 export·check). 기본 열기·저장은 바꾸지 않았습니다.

## 필수 실행 환경

- `DEV-MAC` (macOS 27.2.1, Rust 1.96.0, Apple Git 2.54.0).

## 변경 파일

- `crates/ops/src/text_source.rs`: 단일 View export는 그대로이고 다중 파일 `export_project`·`check_project`를 더했습니다.
- `tools/stellar-cli/src/text_source.rs`: `text-source export-project|check-project`.

## 실행한 명령 / 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-cli --test text_source` | 통과: 실제 CLI export→check→기존 출력 덮어쓰기 거부, 프로젝트 digest 불변 |
| `cargo test -p stellar-cli --test text_project` | 2 passed: 다중 파일 export→check 같은 hash, 기존 폴더·프로젝트 안 출력 거부 |
| `cargo test -p stellar-ops --lib text_source` | 통과: unknown optional·미사용 chunk, pointer metadata 변경, root 문서 없음, schema 변경 모두 쓰기 없이 거부 |
| `cargo xtask validate-adr --base ce60bb92` | 0 |
| `cargo xtask generate-ffi-header --check` | 0(drift 없음) |
| `STELLAR_CHECK_BASE=ce60bb92 cargo xtask check-core`(샌드박스 해제) | 65 — fmt·clippy 통과, test 단계에서 `stellar-local-ipc` 타이밍 테스트 1개 실패 |
| `cargo test --workspace --locked --no-fail-fast`(샌드박스 해제) | 101 — 4,168 passed, 1 failed(같은 local-ipc 테스트) |

## 테스트

- 이 행의 검증 테스트는 모두 통과했습니다. 남은 경고 수: clippy `-D warnings` 0.

## 성능

- 측정하지 않았습니다.

## 차이

- 작업 지시와 다른 점은 없습니다. 실패한 검증은 아래 "남은 위험"에 분리했습니다.

## 남은 위험

- `stellar-local-ipc` `listener::tests::slow_authenticated_peer_does_not_block_other_handshakes`는 병렬 부하에서 시간 상한을 넘었고 단독 실행은 통과했습니다. 이 lane은 `crates/local-ipc`를 바꾸지 않았습니다. `RUST_TEST_THREADS=1`로 다시 실행한 check-core에서는 `stellar-runtime` `app::node_paste::tests::clipboard_rejects_same_revision_selection_changes_before_queued_work`가 한 번 실패했고, 단독 세 번 실행은 모두 통과했습니다.
