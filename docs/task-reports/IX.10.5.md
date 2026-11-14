# 작업 보고서: IX.10.5 capability 상태와 element·property·contract별 registry

- WBS: IX.10.5
- 작업명: capability 상태 Native·Approximated·Unsupported·Deprecated와 element/property/semantic contract별 registry 구현
- 상태: 완료 (`[x]`)
- 선행: IX.10.2(이 lane `[x]`), IX.6.7(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/target-profile/src/capability.rs`(신규) — `CapabilityStatus`, `ElementKind`, `ContractCode`,
  `CapabilityPropertyId`, `CapabilitySubject`, `ProfileLevel`, `StatusStep`·`StatusLadder`, `CapabilityRule`,
  `CapabilityTable`, `CapabilityTableError`, `LadderProblem`.
- `crates/target-profile/src/builtin.rs`(신규) — 내장 release 1(`registry_v1`, `catalog`, `rules_v1`, `defaults_v1`),
  대표 속성 `properties`, `missing_subjects`, `property_name`, 테스트 공용 profile 생성기.
- `crates/target-profile/src/registry.rs` — `ProfileRegistry`에 capability 표 추가.
- `crates/target-profile/src/lib.rs` — 모듈 등록.
- `docs/task-reports/IX.10.5.md`(신규), 계획서(IX.10.5 `[ ]`→`[x]`).

## 구현 요약

- **상태**: Native=1, Approximated=2, Unsupported=3, Deprecated=4(안정 code·이름).
- **subject**: crate graph상 문서 계층 타입을 쓸 수 없어 지역 numeric code로 표현합니다. element는 문서 `NodeKind`
  번호(1~7), contract는 문서 `ContractKind` 번호(1~5, IX.6.7 registry 순서)와 같고, 속성은 상위 16bit 출처
  namespace(0 공통·1 CSS·2 Apple·3 Android)를 가진 `CapabilityPropertyId`입니다.
- **규칙**: subject마다 Web·Apple·Android 상태 계단(`StatusLadder`)이 모두 필수이고(누락을 추정하지 않음) Apple 플랫폼별
  override 계단(예: SingleSelectionGroup은 iOS 근사, macOS Native)과 Deprecated 대체 subject를 가집니다. profile
  수준(`ProfileLevel`)은 Apple 최소 OS(major·minor·patch), Android API level, Web Baseline 연도입니다.
- **표 검사**: subject 중복, 빈 계단·`MIN`에서 시작하지 않는 계단·비오름차순 계단, Apple 플랫폼 override 중복, 표에 없거나
  자기 자신인 대체 subject를 생성 시 거부합니다. 등록되지 않은 subject의 상태는 `None`입니다.
- **내장 release 1**: element 7종·contract 5종 전부와 대표 속성 15개(공통 5, CSS 4, Apple 3, Android 3). 예: Modal은
  Web Baseline 2022 이전 근사, `css.backdrop_filter`는 Baseline 2024부터 Native, `common.layer_blur`·
  `android.dynamic_color`는 API 31부터 Native, `android.status_bar_color`는 API 35부터 Deprecated, `css.clip_rect`는
  Deprecated(대체 `common.clip_path`).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-target-profile` | 25 passed, 0 failed |
| `cargo clippy -p stellar-target-profile --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-target-profile -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 상태 matrix fixture: 내장 release 1의 27개 subject × 7개 profile(web-2022·web-2024·ios-26·macos-26·android-27·android-31·android-35) 모든 칸 기대값 일치, 표와 matrix의 subject 집합 일치, 네 상태 모두 등장 (`release_one_status_matrix_is_golden`) | 통과 |
| 표가 알려진 element·contract·대표 속성을 빠짐없이 포함, 속성 이름 namespace 일치 (`builtin_registry_covers_known_subjects`) | 통과 |
| 상태 code golden, 문서 계층 번호(NodeKind 1~7·ContractKind 1~5)와 같음 (`status_codes_are_golden`) | 통과 |
| 계단 조회·수준 비교(`17.10` > `17.4`), 수준 표기 (`ladder_lookup_and_levels`) | 통과 |
| 표 구조 오류 6종 거부 (`table_structure_is_checked`), Apple 플랫폼 override 우선·미등록 `None` (`apple_platform_override_wins`) | 통과 |

## 차이(작업 지시와 다른 점)

- element·contract 번호는 문서 crate 상수를 직접 참조하지 않고 같은 값을 지역 상수로 둡니다(crate graph: target-profile →
  core-types만). 두 번호 체계의 교차 검사는 두 crate를 함께 쓰는 상위 crate에서 추가해야 합니다.
- 기술 아키텍처 17.2의 `fallback` 상태는 계획서 지시(네 상태)에 따라 넣지 않았습니다.
- 내장 데이터의 플랫폼 사실(Baseline 연도, API level)은 대표 fixture용 초기값이며 제품 출시 전 검토가 필요합니다. 결정
  사항은 허용 경로 밖인 `docs/adr/` 대신 모듈 문서와 이 보고서에 기록했습니다.

## 남은 위험 / 한계

- 문서 속성(PropertyPath)과 `CapabilityPropertyId`의 대응은 문서 계층이 제공해야 합니다(IX.10.8에서 trait로 받음).
- Web `capability_set_version`은 현재 상태 계산에 쓰지 않습니다(Baseline 연도만 사용).
