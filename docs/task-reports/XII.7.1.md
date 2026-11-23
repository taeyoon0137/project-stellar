# 작업 보고서: XII.7.1 창 session base generation·hash 유지와 저장 전 디스크 비교

## WBS

- WBS: XII.7.1
- 작업명: 창 session base generation·hash 유지와 저장 전 디스크 비교
- 상태: 완료 (`[x]`) 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: XII.2.4, V.5.2
- 결정: [ADR-0083](../adr/ADR-0083-window-session-stale-and-external-sync.md)
- lane: `window-session-stale`

## 변경 파일

- `crates/runtime/src/window_session.rs`: `ProjectWindowSession::stale_state`(base hash와 current pointer 비교, writer 출처·다음 할 일)
- `crates/project/src/window_session.rs`: `record_app_window_save`·`writer_provenance`

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --locked` | 0 — lib 239 passed·2 ignored, 통합 테스트 전부 통과 |
| `cargo clippy -p stellar-runtime -p stellar-project -p stellar-ops -p stellar-cli -p stellar-ffi --all-targets --locked -- -D warnings` | 0 — 경고 0 |
| `cargo xtask validate-adr --base d2bf97b3` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: Apple Silicon macOS 로컬 실행(아래 테스트). lane 전체 `check-core` 결과는 lane 결과 보고에 적습니다.

## 테스트

| 테스트 | 결과 |
|---|---|
| `window_session::stale_tests::second_window_keeps_its_base_and_its_save_is_stale` | 통과 — 두 번째 창 save가 `Stale`, current pointer bytes 불변, base hash 유지, 편집 보존 |
| `window_session::tests::save_now_writes_this_windows_document_as_a_new_generation` | 통과(기존) |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음). 판정 비용은 ADR-0083 Performance에 적었습니다.

## 차이

- 없음.

## 남은 위험

- writer 출처는 이 프로세스의 앱 창 저장 기록입니다. 앱 재시작 뒤 처음 보는 다른 창 generation은 외부로 봅니다(ADR-0083 결정 2).
