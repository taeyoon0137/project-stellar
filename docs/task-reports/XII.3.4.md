# 작업 보고서: XII.3.4 연속 pointer move·wheel 이벤트 coalescing

- WBS: XII.3.4
- 작업명: 연속 pointer move/wheel event coalescing — pointer down/up과 key event 순서는 보존
- 상태: 완료 (`[x]`)
- 선행: XII.3.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust 단위 테스트 + 로컬 Xcode Debug 빌드와 headless `--input-check`).

## 변경 파일 목록

- `crates/ffi/src/input_queue.rs`(신규) — `InputQueue`(용량 고정 FIFO, 끝 항목 병합), `PushOutcome`, 병합 규칙, 테스트.
- `crates/ffi/src/lib.rs` — runtime 입력 대기열을 `InputQueue`로 교체, `submit_mac_event`의 병합 반영(`out_queued = 1`),
  10,000 drag FFI 테스트.
- `tools/xtask/src/ffi_header.rs`(헤더 템플릿 주석), `crates/ffi/include/stellar_ffi.h`(재생성) — `out_queued` 의미 주석.
- `apps/macos/StellarMac/InputCheck.swift` — down → 10,000 drag → up 셸 경로 coalescing 검사 추가.
- `scripts/macos/smoke.sh` — 설명 주석 갱신.
- `docs/adr/ADR-0014-ffi-shell-surface.md` — XII.3.4 결정 추가.
- `docs/task-reports/XII.3.4.md`(신규), 계획서(XII.3.4 `[ ]`→`[x]`).

## 구현 요약

- **끝 항목 병합만**: 새 이벤트는 아직 꺼내지 않은 마지막 항목과만 병합합니다. pointer down/up·cancel, key, text
  composition, focus, resize는 병합하지 않으므로 이들의 순서와 앞뒤 move/wheel 사이 순서가 바뀌지 않습니다.
- **move**: 둘 다 `Move`이고 button·device·modifier가 같으면 최신 이벤트(위치·시각)로 교체합니다. hover move와 drag
  move(button 다름), modifier 변화는 병합하지 않습니다.
- **wheel**: 같은 phase(`Changed`·`Momentum`·`Discrete`)·단위·modifier면 delta를 IEEE `f32` 덧셈으로 누적하고 위치·시각은
  최신 값을 씁니다(scroll 거리 보존). `Began`·`Ended`는 gesture 경계로 병합하지 않고, 합이 비유한이면 따로 넣습니다.
- **용량**: 4096은 항목 수입니다. 가득 차도 끝 항목과 병합되는 move/wheel은 받고, 병합할 수 없는 이벤트만
  `STELLAR_QUEUE_FULL`입니다. 병합된 이벤트 수는 `coalesced_count`(진단용)로 셉니다.
- C ABI 시그니처는 바꾸지 않았고 `out_queued`는 병합도 "반영(1)"으로 셉니다(ADR-0014).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ffi` | 49 passed, 0 failed |
| `cargo clippy -p stellar-ffi --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-ffi -- --check` | 통과 |
| `cargo test -p xtask` / `cargo clippy -p xtask --all-targets -- -D warnings` / `cargo fmt -p xtask -- --check` | 115 passed / 경고 0 / 통과 |
| `cargo run -p xtask -- generate-ffi-header` 후 `generate-ffi-header --check` | drift 없음 |
| `cargo run -q -p xtask -- validate-crate-graph` / `validate-dependencies` | 종료 0 / 종료 0 |
| `scripts/macos/smoke.sh` | `BUILD SUCCEEDED`(Swift warning/error 0), `input ok: 60 events (... 10000 drag→1)`, `smoke OK`, 종료 0 |
| `git diff --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **10,000 move→최신 1개, down/up 보존**: down → 10,000 drag → up → key down → 10,000 hover → key up이 `[down, move, up, key-down, move, key-up]` 6개, 남은 move는 각 구간 마지막 위치·시각, 병합 수 19,998(`ten_thousand_moves_collapse_to_latest_and_keep_down_up_order`) | 통과 |
| FFI 경로: `submit_mac_event`로 down → 10,000 `leftMouseDragged` → up 후 poll이 Down·Move(9999, 4999.5)·Up 3개 | 통과 |
| 셸 경로(headless): `CanvasView` 처리기로 down → 10,000 drag → up 후 대기열이 down·최신 drag·up 3개, 좌표 일치 | 통과 |
| 경계: key 사이 move, button·modifier 변화, cancel·down·focus·composition 연속은 병합하지 않고 시각 순서 1..13 그대로 | 통과 |
| wheel: Changed 100개 delta 합 (50, 200)·최신 위치/시각, Began·Ended 경계, Momentum 합, Line/Point 분리, 합 overflow 미병합 | 통과 |
| 용량: 가득 찬 상태에서 병합 move 수용, 병합 불가 이벤트 `Full`, 꺼낸 항목과는 병합하지 않음 | 통과 |
| 검사 민감도: 병합을 임시로 끄면 coalescing 테스트 4개가 실패함을 확인 후 원복 | 확인 |

## 차이(작업 지시와 다른 점)

- wheel은 "최신 1개"가 아니라 같은 phase 안에서 delta를 합칩니다(scroll 거리 보존). 검증 기준의 "최신 1개"는 pointer move에
  적용했습니다.
- coalescing은 셸→코어 입력 대기열(FFI runtime) 단계에서 합니다. UI→DocumentThread command queue(XII.1.2)가 생기면 그
  입력 단계로 옮기거나 재사용합니다.

## 남은 위험 / 한계

- 병합된 move의 중간 위치·개별 시각은 남지 않습니다. 필기·pen 경로처럼 중간점이 필요한 도구가 생기면 별도 정책이
  필요합니다.
- T0 latency trace(XII.3.5)는 병합 전 최초 수신 시각을 어떻게 기록할지 정해야 합니다(현재 병합 결과는 최신 시각).
- resize 연속 이벤트는 병합하지 않습니다(작업 범위 밖).
- 소비자(DocumentThread)가 없어 실제 앱에서 병합 효과는 대기열 크기로만 나타납니다.
