# 작업 보고서: IX.5.2 EventKind

- WBS: IX.5.2
- 작업명: EventKind에 Activate, Input, Focus, Blur, Key, SelectionChange, Open, Close 정의
- 상태: 완료 (`[x]`)
- 선행: IX.5.1(이 lane에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/event.rs`(신규) — `EventKind`, `UnknownEventKind`, tag·이름·payload 타입.
- `crates/document/src/component/mod.rs` — 모듈 등록.
- `docs/task-reports/IX.5.2.md`(신규), 계획서(IX.5.2 `[ ]`→`[x]`).

## 구현 요약

- 닫힌 enum 8종과 안정 numeric tag(0x01~0x08, 저장 형식 계약), 진단용 안정 이름, 모르는 tag 거부
  (`from_tag` → `UnknownEventKind`).
- 이벤트별 payload 타입: `Input`(입력 후 전체 텍스트)·`Key`(정규화된 key 식별자)·`SelectionChange`(선택된
  item identity key)는 `String`, 나머지는 payload 없음. `accepts_payload`가 runtime dispatch(IX.5.5)의 입력
  검사에 쓰입니다.
- Enabled·Disabled 전환은 이벤트가 아니라 `disabled` 상태 값(IX.6.2)으로 표현한다고 모듈 문서에 적었습니다
  (기술 아키텍처 15.3의 초기 이벤트 범위 중 상태로 표현되는 항목).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 268 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 8종 tag·이름·payload 타입 golden 표 일치, tag round-trip, 미지 tag(0x00·0x09·0x7F·0xFF) 거부(`kind_golden_table`) | 통과 |
| payload 수용 규칙(`payload_acceptance`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- key 식별자 정규화 규칙(플랫폼 key code → 문자열)은 셸 입력 계층(Phase XII)이 정합니다.
