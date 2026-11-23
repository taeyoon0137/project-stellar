# 작업 보고서: XVI.1.5 ring buffer 로컬 trace store

- WBS: XVI.1.5
- 작업명: ring buffer 기반 로컬 trace store, 용량 초과 시 가장 오래된 trace부터 삭제
- 상태: 완료 (`[x]`)
- 결정: ADR-0084 결정 8
- 선행: XVI.1.4(이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 측정 장비는 개발 장비(Apple M3 Max, 36GB)이고 기준 장비(M1 16GB)가 아닙니다.

## 변경 파일 목록

- `crates/diagnostics/src/trace_store.rs`(신규).
- `docs/task-reports/XVI.1.5.md`(신규).

## 구현 요약

- 고정 용량은 기본 4,096개이고 상한은 1,048,576개입니다. `push`는 가득 차면 가장 오래된 trace를 꺼내고 지운 수를 셉니다.
- 항목이 고정 크기 값이라 메모리 상한은 `capacity × size_of::<CompletedTrace>()`입니다. store는 로컬 전용이며 전송하지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-diagnostics` | lib 25 passed, doc 3 passed(compile_fail 포함), 0 failed |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `oldest_traces_are_evicted_and_memory_stays_bounded`: 용량 64에 640개를 넣습니다. 길이는 64를 유지하고, 지운 수 576, 가장 오래된 것부터 지움, 남은 ID 577\~640, 할당 bytes 상한 유지.
- `capacity_is_clamped`.

## 성능

- push는 O(1)입니다.

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 진단 번들 내보내기(사용자 동의·확인 UI)는 이 행 밖입니다.
