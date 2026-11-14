# 작업 보고서: IX.1.4 Component Property 평가 값 instance별 캐시

- WBS: IX.1.4
- 작업명: component property의 runtime evaluated value를 instance별로 캐시하고 definition revision과 override
  revision으로 invalidate
- 상태: 완료 (`[x]`)
- 선행: IX.1.2(`[x]`), VIII.6.4(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/property_cache.rs`(신규) — `PropertyRevisions`, `InstancePropertyCache`,
  `EvaluatedProperties`, `PropertyValue`, `ValueOrigin`, `CacheStats`, `PropertyCacheError`.
- `crates/document/src/component/mod.rs` — 모듈 등록.
- `docs/task-reports/IX.1.4.md`(신규), 계획서(IX.1.4 `[ ]`→`[x]`).

## 구현 요약

- **revision**(런타임 전용, 저장하지 않음): 컴포넌트별 definition revision, 인스턴스별 override revision.
  `record_property_command`(IX.1.3 command → 그 컴포넌트 정의 revision)·`record_override_command`(IX.2.4 command →
  그 인스턴스 override revision) 도우미를 제공합니다.
- **캐시**: 인스턴스마다 (컴포넌트, 계산 당시 definition revision, override revision, property별 값·출처)를 저장합니다.
  조회 시 두 revision이 현재와 같으면 scene payload를 디코딩하지 않고 재사용(hit), 다르면 그 인스턴스만 다시
  평가합니다.
- **값 규칙**: 선언 순서 property마다 공개·타입 일치 직접 override가 있으면 override, 아니면 default 평가 값입니다.
  orphan override와 타입이 틀린 파일 값은 무시하고 default로 떨어집니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 327 passed, 통합 7+1+1, doc 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 인스턴스 두 개 중 하나의 override를 IX.2.4 command로 변경 → 그 인스턴스만 stale·재평가, 다른 인스턴스는 hit(평가 수 불변), undo 후에도 동일(`override_change_invalidates_only_that_instance`) | 통과 |
| 다른 컴포넌트 정의 변경은 무영향, IX.1.3 SetDefault 후 같은 컴포넌트 인스턴스 둘 다 재평가·새 default, 이후 hit만(`definition_change_invalidates_all_instances_of_that_component`) | 통과 |
| 타입 틀린 override·orphan override는 default, 비인스턴스 조회 오류(`invalid_or_orphan_overrides_fall_back_to_default`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 VIII.6.4 scheduler는 `stellar-bindings`에 있어 document에서 직접 쓸 수 없습니다(crate 그래프). 이 캐시는
  override·default 기반 property 값만 다루며, binding이 계산한 값의 증분 재평가는 VIII.6.4 scheduler 몫입니다.

## 남은 위험 / 한계

- revision 증가는 호출자 책임입니다. command 계층 연결 시 PropertyCommand·PropertyOverrideCommand 적용 경로에서
  도우미를 호출해야 합니다.
- drilling으로 상위 property에 묶인 nested 인스턴스(가상 subtree) 값은 이 캐시 범위가 아닙니다(scene 인스턴스 단위).
