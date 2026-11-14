# 작업 보고서: IX.10.4 profile registry 버전과 project pin 분리

- WBS: IX.10.4
- 작업명: Stellar 앱 업데이트만으로 project profile version을 바꾸지 않게 registry version과 project pin 분리
- 상태: 완료 (`[x]`)
- 선행: IX.10.3(이 lane `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/target-profile/src/registry.rs`(신규) — `ProfileRegistryVersion`, `ProfileRegistry`, `NewProfileDefaults`,
  `NewProfileKind`, `RegistryCatalog`, `ResolvedTargetProfile`, `CatalogError`, `ResolveError`.
- `crates/target-profile/src/spec.rs` — `TargetProfile.registry_version`(필수 키 `registry_version`) 추가.
- `crates/target-profile/src/manifest.rs` — 테스트 생성자 갱신.
- `crates/target-profile/src/lib.rs` — 모듈 등록.
- `docs/task-reports/IX.10.4.md`(신규), 계획서(IX.10.4 `[ ]`→`[x]`).

## 구현 요약

- **registry 쪽**: `ProfileRegistry`는 앱이 내장한 정의의 한 release(버전별 내용 고정)이고, `RegistryCatalog`는 앱이 가진
  release 모음입니다. 앱 업데이트는 release를 추가할 뿐 기존 release 내용을 바꾸지 않는 계약입니다(IX.10.5에서 capability
  표가 release에 들어갑니다).
- **project 쪽**: 각 `target_profiles[i]`에 `registry_version` pin을 필수로 기록합니다(1 이상, canonical 위치는
  `profile_schema_version` 뒤). pin은 사용자가 승인한 upgrade(IX.10.9·IX.10.10)로만 바뀝니다.
- **해석**: `RegistryCatalog::resolve`는 항상 pin한 release로 해석하고 profile을 바꾸지 않습니다. 최신 release로 대체하지
  않으며, 앱에 없는 버전(더 새 앱에서 만든 프로젝트, 중간 버전 누락 포함)은 `RegistryVersionUnavailable`로 거부합니다.
- **최신 release의 역할**: 새로 만드는 profile의 기본값과 pin에만 쓰입니다(`new_profile`).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-target-profile` | 19 passed, 0 failed |
| `cargo clippy -p stellar-target-profile --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-target-profile -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| registry update 후: 이전 앱(v1)에서 만든 프로젝트를 새 앱(v1+v2, 더 새 기본값)으로 열고 다시 기록한 매니페스트 canonical bytes 동일, 모든 profile의 해석 결과(profile + pin release) 이전 앱과 동일, pin v1 유지; 새 profile만 v2 기본값·pin (`app_update_keeps_project_bytes_and_resolved_profiles`) | 통과 |
| v2 pin 프로젝트를 v1 앱으로 열면 거부, 중간 버전이 빠진 catalog에서도 다른 버전으로 대체하지 않음 (`pinned_release_missing_from_app_is_rejected_not_replaced`) | 통과 |
| catalog 빈 목록·중복 버전 거부, 버전 정렬, 버전 파싱(0·선행 0 거부) (`catalog_rules`) | 통과 |
| `registry_version` 누락·0을 모든 종류에서 경로와 함께 거부 (`spec::tests::required_fields_are_validated_per_kind`) | 통과 |

## 차이(작업 지시와 다른 점)

- pin은 프로젝트 전체가 아니라 profile entry마다 둡니다. profile별 upgrade(IX.10.9)가 다른 profile의 해석을 바꾸지 않게
  하기 위해서입니다. 이 결정은 허용 경로 밖인 `docs/adr/` 대신 모듈 문서와 이 보고서에 기록했습니다.
- "project bytes"는 매니페스트 최상위 노드의 canonical bytes로 검증했습니다. `stellar.yaml` 텍스트 bytes 연결은 project
  crate가 이 노드를 IV.1.5 writer로 기록하는 작업에서 확인해야 합니다.

## 남은 위험 / 한계

- 앱 release 내용의 불변성(기존 버전을 수정하지 않음)은 계약이며 이 crate가 강제할 수 없습니다. 내장 release의 golden
  검사(IX.10.5 상태 matrix)로 변경을 감지합니다.
