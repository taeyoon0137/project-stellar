# 작업 보고서: XXX.7.4 Context 해석기

- WBS: XXX.7.4
- 작업명: instance expansion view 기준 가장 가까운 조상 provision 해석, slot 작성 scope 규칙, runtime index 캐시.
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 XXX.7.3이 `[x]`가 아닙니다. 그 조건이 풀리면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]` 후보)
- 결정: ADR-0099 결정 4, ADR-0025 결정 3.5·3.11
- 선행: XXX.7.3, IX.2.2, IX.3.2

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/document/src/context/{resolve,resolve_tests}.rs`(신규), `crates/document/src/context/fixtures.rs`(공용 시험 문서),
  `crates/document/src/context/set.rs`(`ContextKeys`).

## 구현 요약

- site는 (인스턴스 chain, node)이고 `ExpandedNodeId`에서 그대로 만듭니다(`From<&ExpandedNodeId>`).
- 작성 부모를 따라 올라가며 정의 root는 그 instance node로 이어 갑니다. slot override 콘텐츠의 부모가 instance이면 Slot 공개
  목록에 있는 key만 slot host부터 내부를 찾고(내부에 없으면 정의 root를 거쳐 instance node로), 나머지 key는 내부를 건너뜁니다.
- provision 값 식의 같은 key 재읽기는 provider 바깥부터, 다른 key는 provider 자신부터 찾습니다.
- 결과(출처, 지나간 site, 공개·비공개로 지난 Slot, 가린 provision)는 (site, key, 시작 포함)별로 캐시합니다.
- `ContextEvaluator`가 해석 결과로 값을 계산하고 provision 읽기 순환을 오류로 냅니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document --lib context::` | 통과 |

시험:

- `context::resolve_tests::nearest_ancestor_provision_shadowing_default_and_same_key_reread`: 중첩·shadowing·provision 없음·같은 key 재읽기 값.
- `context::resolve_tests::slot_content_resolves_in_authoring_scope_and_exposed_keys_see_internal_provisions`: 공개하지 않은 key는
  instance 바깥 값, 공개한 key는 내부 provision 값, 공개했지만 내부 provision이 없는 key, nested slot.
- `context::resolve_tests::resolution_is_cached_and_deterministic_and_matches_expansion_ids`: 같은 입력 같은 결과, 캐시 재사용,
  `InstanceExpansion::walk`의 모든 node에서 해석.

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 캐시는 해석기 수명 동안입니다. 문서가 바뀌면 새 해석기를 만들고, 어떤 소비가 다시 해석될지는 XXX.7.6 footprint가 고릅니다.
