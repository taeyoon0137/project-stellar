# 작업 보고서: XVI.8.1 로컬 diagnostic store schema

- WBS: XVI.8.1
- 작업명: 로컬 diagnostic store에 app crash, renderer error, performance trace, feature failure count schema 정의
- 상태: 완료 (`[x]`) — 구현과 검증 기준(schema round-trip)을 끝냈습니다. 선행 XVI.1.5는 원장에서 `[~]`(XVI.1.2 사슬)이라 원장 판정은 통합 때 정합니다.
- 결정: ADR-0109 결정 1\~3
- 선행: II.3.2, XVI.1.5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/diagnostics/src/store.rs`(신규).
- `crates/diagnostics/src/lib.rs`(module 등록 한 줄).
- `docs/adr/ADR-0109-local-diagnostics-and-telemetry-consent.md`(신규).
- `docs/task-reports/XVI.8.1.md`(신규).

## 구현 요약

- record 네 종류(`crash`, `renderer_error`, `perf`, `feature_failure`)와 `meta dropped=`를 `RecordSchema`로 정의했습니다. field는 정수, 닫힌 enum 이름, 숫자 version뿐이고 런타임 문자열 field는 없습니다.
- 성능 record는 완료 trace(XVI.1.2)에서 상호작용 종류, 문서·가시 bucket, cache 상태, Logical Interaction Latency, GPU 시간만 남깁니다(`PerformanceSample::from_trace`). trace ID와 stage 시각은 남기지 않습니다.
- 줄 codec(`encode_line`, `decode_line`, `parse_value`)은 store, 진단 번들, scanner 구조 검사가 함께 쓰는 하나뿐인 경로입니다. 정수는 앞자리 0 없는 10진수만 받습니다.
- `DiagnosticStore`: event 기본 2,048개(상한 65,536개, 가장 오래된 것부터 지움), 실패 counter는 `(기능, 코드)`당 하나로 합치고 1,024쌍 상한, 넘친 수는 `dropped`. 보존 정책 기본 30일(`prune`).
- 파일: `stellar.diagnostic-store v1` 머리 줄, `meta` 줄, event 줄(넣은 순서), counter 줄(key 오름차순)의 정규형 하나입니다. 소유자 전용(`0600`, 디렉터리 `0700`)으로 임시 파일에 쓰고 fsync 뒤 rename합니다. 이 쓰기 함수(`write_private_file`)를 동의 상태·전송 대기열도 씁니다.
- 형식 오류는 줄 번호와 정적 종류만 냅니다. 줄 내용을 되풀이하지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-diagnostics --lib store` | 12 passed(store 10개 포함), 0 failed |
| `cargo clippy -p stellar-diagnostics --all-targets -- -D warnings` | 경고 0 |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `store_bytes_are_golden`: 네 종류 record와 합쳐진 counter의 파일 bytes golden.
- `schema_round_trips_every_record_kind_and_value`: 검증 기준(schema round-trip). 고정 store와, 결정적 무작위 4,000개 event·모든 기능×코드(0, 1, `u32::MAX`) counter·경계 정수를 넣은 store가 bytes → store → bytes로 같습니다. 줄 단위 round-trip과 모든 enum 이름 출현도 봅니다.
- `trace_summary_keeps_only_buckets_and_durations`: 완료 trace 요약 줄에 bucket과 지연만 있습니다.
- `malformed_lines_are_rejected_without_echo`: 연속·끝 공백, 제어 문자, 모르는 tag(`terminal`), field 수·이름(`path=`), 앞자리 0, 대문자 enum, u32 초과, count 0, 시각 역순, 모르는 기능(`terminal`)을 거부하고 오류에 내용이 없습니다.
- `store_files_reject_non_canonical_layouts`: version 불일치, 끝 줄바꿈 없음, meta 없음·중복, counter 뒤 event, counter 중복, 비UTF-8, 용량 초과.
- `capacity_and_counters_are_bounded`, `retention_prunes_old_records_only`, `version_accepts_digits_only`, `files_are_written_owner_only_and_round_trip`(권한 `0600`·`0700`, 임시 파일 없음).

## 성능

- event 추가 O(1), counter O(log n). 메모리는 용량으로 묶입니다. 상호작용 경로에는 들어가지 않습니다.

## 차이(작업 지시와 다른 점)

- 렌더러 backend는 `metal`만 둡니다(ADR-0006 Windows 지원 보류).

## 남은 위험 / 한계

- 앱 연결(충돌 handler, 렌더러 오류 보고, 기능 실패 집계)은 UI 재설계 때 합니다. 지금은 Rust API와 CLI로만 씁니다.
- 닫힌 enum이라 새 값은 schema 변경이고 이전 앱은 새 값이 든 store를 읽지 못합니다(ADR-0109 Consequences).
