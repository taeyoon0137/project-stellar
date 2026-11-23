# 작업 보고서: XXV.2.2 진단 필드와 schema round-trip

- WBS: XXV.2.2
- 작업명: diagnostic에 severity, source StableId, property path, remediation action ID 포함
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 XXV.2.1이 `[x]`가 아닙니다. 선행이 끝나면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]` 후보). 선행 XXV.2.1 판정을 따릅니다.
- 결정: ADR-0095 결정 2·3

## 필수 실행 환경

- `DEV-MAC`(Rust 단위 test).

## 변경 파일 목록

- `crates/integrity/src/codec.rs`(신규): 진단 목록 저장 schema v1(`SDIX`) encode·decode, cursor용 key decode.
- `crates/integrity/src/lib.rs`: 모듈 등록.
- `crates/integrity/src/tests.rs`: 전체 fixture 진단 round-trip 검사 추가.

## 구현 요약

- 진단 한 건은 key(문서·종류·source StableId·property path·생산자 code·subject·origin), 심각도, 설명, remediation action ID
  목록입니다. property path는 core-types canonical bytes를 그대로 씁니다.
- decode는 다음을 거부합니다: 모르는 종류·심각도·action 값, 선택 값 표시 오류, 잘린 입력, 문자열·key·action 상한 초과, 뒤에
  남은 byte. 진단 수가 남은 입력으로 만들 수 있는 수보다 크면 할당 전에 거부합니다.
- 진단은 Git 원본에 쓰지 않습니다(파생 데이터).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-integrity codec` | 통과(3 passed) |
| `cargo test -p stellar-integrity` | 통과(17 passed) |

| 검사 | 결과 |
|---|---|
| schema round-trip: 무작위 진단 0·1·2·17·300개 목록이 같은 값으로 돌아오고, 다시 쓰면 같은 bytes | 통과(`codec::tests::schema_round_trip_is_lossless_and_canonical`) |
| 모든 필드(심각도·source·property path·remediation) 보존 | 통과(`codec::tests::every_field_survives_round_trip`) |
| 모든 위치에서 자른 입력, 모르는 값, 거대 개수, 남은 byte 거부 | 통과(`codec::tests::malformed_input_is_rejected`) |
| 생산자 8종 전체 fixture 진단 목록 round-trip | 통과(`tests::fixture_counts_are_exact_per_kind_and_severity`) |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- schema를 바꾸면 version을 올리고 decode 분기를 더해야 합니다. 지금은 v1만 읽습니다.
