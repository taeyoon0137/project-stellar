# 작업 보고서: XXX.7.11 인스펙터용 Context 조회 API

- WBS: XXX.7.11
- 작업명: 선택 binding의 key·해석 값·provider 경로 조회와 Value Binding 선택기 후보.
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 XXX.7.7, XXX.7.5이 `[x]`가 아닙니다. 그 조건이 풀리면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]` 후보) — 엔진 API까지입니다. CLI·MCP·셸 연결은 하지 않았습니다.
- 결정: ADR-0099 결정 11
- 선행: XXX.7.7, XXX.7.5

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/document/src/context/{query,query_tests}.rs`(신규).

## 구현 요약

- `inspect_binding`: Context를 읽는 key마다 이름, 해석 값(또는 평가 오류), 출처, 소비 지점 → provider 경로, Slot 경유.
- `binding_candidates`: 그 위치에서 소비 가능한 key(다른 문서 scope 제외, 기대 타입 필터), 선언 필요 여부, 해석 출처.
  (이름, StableId) 순입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document --lib context::` | 통과 |

시험:

- `context::query_tests::inspecting_a_context_binding_returns_key_value_and_provider_path`.
- `context::query_tests::candidates_list_consumable_keys_with_declaration_needs_in_stable_order`: 외부 라이브러리 key 포함,
  같은 입력 같은 순서.

## 차이(작업 지시와 다른 점)

- 없음(셸 FFI는 사용자 방향 2026-11-23에 따라 만들지 않음).

## 남은 위험 / 한계

- CLI·MCP 조회 명령은 binding이 문서 원본에 저장되지 않아 headless에서 소비 binding을 읽을 곳이 없어 미뤘습니다.
