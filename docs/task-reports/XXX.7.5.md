# 작업 보고서: XXX.7.5 Context validator

- WBS: XXX.7.5
- 작업명: 선언 없는 소비·타입 불일치 오류, orphan, 쓰지 않는 requirement 경고, default·shadowing 정보, K1↔K2 cycle.
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 XXX.7.4이 `[x]`가 아닙니다. 그 조건이 풀리면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]` 후보)
- 결정: ADR-0099 결정 5, ADR-0025 결정 3.7
- 선행: XXX.7.4, VIII.6.3

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/document/src/context/{validate,validate_tests}.rs`(신규).

## 구현 요약

- 소비 binding은 문서 밖 resource라 `ContextConsumer`(binding, site, 기대 타입, source)로 받습니다.
- 소비 node를 소유한 정의(`owner_of`)가 있으면 그 정의의 requirement가 필요합니다. 정의 안에서 작성한 slot 콘텐츠도 그 정의
  소유라 같은 규칙입니다. View 최상위 scene node는 선언이 필요 없습니다. 정의 소유 provision 값 식의 읽기도 같은 규칙입니다.
- 타입 불일치는 key 타입으로 source·provision 값을 컴파일해 판정합니다. orphan은 없는 key·Slot·컴포넌트를 가리키는 항목입니다.
- cycle은 모든 site(`all_sites`: scene node와 펼친 가상 node)의 provision 값을 평가해 순환을 찾습니다. 같은 Context를 binding
  graph로 만들면 VIII.6.3 cycle 검사가 같은 경우를 거부합니다(XXX.7.3 시험).
- 진단은 정렬·중복 제거되어 같은 입력에서 같습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document --lib context::` | 통과 |

시험:

- `context::validate_tests::diagnostics_cover_declaration_types_orphans_unused_and_info`: 진단 전체 목록 일치, View 최상위 소비
  허용, View 최상위에서 작성한 slot 콘텐츠의 바깥 값 해석, 정의 안에서 작성한 slot 콘텐츠의 선언 없는 소비 오류.
- `context::validate_tests::clean_sample_world_has_no_errors_and_mutual_key_reads_are_a_cycle`: K1↔K2 cycle, 조상·자손 사이
  읽기가 default에서 끝나면 cycle이 아님.

## 차이(작업 지시와 다른 점)

- "저장 전 거부"는 validator 오류와 binding graph 생성 거부로 제공합니다. 문서 저장 경로에 validator를 거는 일은 저장 경로가
  컴포넌트 library와 소비 binding을 함께 알게 될 때의 후속입니다(binding 저장 형식 없음).

## 남은 위험 / 한계

- XXX.2.5(그래프 node·socket 진단 매핑)의 Context 진단 연결은 하지 않았습니다(prototype lane 몫).
