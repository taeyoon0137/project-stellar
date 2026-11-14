# 작업 보고서: IX.10.7 profile 희소 override resolver와 View Variant·Binding 환경 결합

- WBS: IX.10.7
- 작업명: 공통 structure에 profile-specific sparse override를 적용하는 resolver, View Variant·Binding environment 결합
- 상태: 완료 (`[x]`)
- 선행: IX.7.6(`[x]`), VIII.7.3(`[x]`), IX.10.5(이 lane `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/target-profile/src/overrides.rs`(신규) — `CommonValues`, `OverrideScope`, `ProfileOverrides`,
  `ProfileOverrideSet`, `VariantLayer`, `ValueOrigin`, `ResolvedValue`, `ResolvedView`, `resolve_for_profile`.
- `crates/target-profile/src/environment.rs`(신규) — `EnvironmentKeyCode`, `ProfileEnvironment`, `changed_keys`,
  `PLATFORM_ENUM_TYPE`·`PLATFORM_GENERIC`.
- `crates/target-profile/src/lib.rs` — 모듈 등록.
- `docs/task-reports/IX.10.7.md`(신규), 계획서(IX.10.7 `[ ]`→`[x]`).

## 구현 요약

- **희소 override**: 공통값(`CommonValues`, PropertyPath → TypedValue)이 원본이고, profile별로 차이만 저장합니다
  (`ProfileOverrides`: `Always`·`Variant(id)` 범위 값 + StableId 기반 node 제외).
- **우선순위**(뒤가 이김): 공통값 → profile override(`Always`) → View Variant override(IX.7.4~IX.7.6 resolver가 path마다
  정한 승자, `VariantLayer`) → profile × variant override(활성 variant 적용 순서대로). 상태 변화는 모든 profile에 적용되고,
  profile만의 상태 차이는 profile × variant override로 표현합니다. 비활성 variant의 profile × variant 값은 적용하지 않습니다.
  결과 값마다 출처(`ValueOrigin`)를 남깁니다.
- **구조 override**: 제외 node를 root로 하는 값은 결과에서 빠지고, 하위 node 판정은 `ResolvedView::is_excluded`가 parent
  조회를 상한 단계까지만 반복합니다(순환 입력에서도 종료).
- **Binding 환경**: 선택 profile이 VIII.7.1 key `platform`(1)·`os_version`(2)·`api_level`(3) 값을 정합니다(platform enum
  `0xE000_0001`, variant = profile 종류 code, profile 없음 = generic 0). Apple `os_version`은 `major.minor`를 10진 수로 읽은
  값입니다. `changed_keys`는 값이 실제로 바뀐 key만 돌려주어 VIII.7.3 reverse index가 그 key를 참조한 expression만
  무효화하게 합니다(예: iOS 26 → macOS 26은 무효화 없음).
- 프로젝트에 없는 profile의 override는 `unknown_profiles`로 보고합니다(IX.10.8 진단 입력).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-target-profile` | 33 passed, 0 failed |
| `cargo clippy -p stellar-target-profile --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-target-profile -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 우선순위 fixture: Apple+compact에서 fill=공통, radius=profile, padding=variant, label=profile×variant, shadow=profile 전용; Web+compact는 공통+variant만; Apple variant 없음은 profile 값과 공통값(비활성 profile×variant 미적용) (`common_profile_and_variant_priority`) | 통과 |
| 활성 variant 2개(compact → error)에서 뒤 variant의 profile×variant 값이 이김, 다른 profile은 variant 값 (`later_active_variant_wins_in_profile_variant_layer`) | 통과 |
| Android에서 제외 node 값 제거·하위 node 제외 판정·순환 parent 종료, Apple은 유지, profile 전환 환경 key 변화 (`excluded_nodes_and_environment_follow_profile`) | 통과 |
| 환경 key 번호·enum 번호(VIII.7.1과 같음), profile별 값, 바뀐 key만 보고(같은 OS 다른 Apple 플랫폼 없음, OS 버전 변경 `os_version`만, API 변경 `api_level`만) (`environment::tests`) | 통과 |
| 미등록 profile override 보고, 빈 항목 정리, override 지우기 (`override_set_bookkeeping`) | 통과 |

## 차이(작업 지시와 다른 점)

- crate graph상 document·bindings·expressions를 쓸 수 없어 View Variant 결과는 `VariantLayer`(활성 variant 순서 + path별
  승자 값)로, 환경 key는 같은 번호의 지역 상수로 받습니다. 문서 `VariantResolution` → `VariantLayer` 변환과 `changed_keys`
  결과를 bindings `EnvironmentIndex`에 넘기는 연결은 상위 crate 작업입니다.
- 우선순위 결정(variant가 profile override를 이기고, profile × variant가 가장 우선)은 허용 경로 밖인 `docs/adr/` 대신 모듈
  문서와 이 보고서에 기록했습니다.

## 남은 위험 / 한계

- Apple `os_version`을 10진 수로 읽으므로 minor가 두 자리일 때(예: `10.15`) 수 비교 순서가 버전 순서와 다릅니다. VIII.7.1의
  `major.minor` 표기 정의를 따른 것이며, 정확한 버전 비교는 capability 계산(`ProfileLevel`)이 담당합니다.
- override 저장 형식(문서 파일 직렬화)은 이 작업 범위가 아닙니다.
