# 작업 보고서: IX.10.4 profile registry 버전과 project pin 분리

- WBS: IX.10.4
- 작업명: Stellar 앱 업데이트만으로 project profile version을 바꾸지 않게 registry version과 project pin 분리
- 상태: `[~]` 유지 — 구현 판정 통과, 선행 미충족(2026-11-23 재판정, 아래 "재판정" 절). 이전 기록: 부분 완료 (`[~]`) — pin 분리와 pin 기준 해석은 구현·검증했지만, "project bytes"는 형식 중립
  `ManifestNode` canonical bytes로만 검증했습니다. 실제 `stellar.yaml`은 `registry_version` pin을 아직 저장할 수
  없습니다(IX.10.3 `[~]`, 아래 상태 정정).
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

## 상태 정정(리뷰 반영, 2026-11-14)

- `[x]` → `[~]`: 리뷰(TP-1)에서 IX.10.3 형식이 실제 `stellar.yaml`(project crate 파서)에 저장되지 않는다는 점이 확인되었습니다.
  project 파서는 `target_profiles[i].registry_version`을 `UnknownKey`로 거부하므로, 이 행의 검증 열 "registry update 후 project
  bytes unchanged"는 실제 프로젝트 파일 bytes가 아니라 매니페스트 최상위 노드의 canonical bytes에서만 증명되었습니다(위 차이 절에
  적은 한계와 같음).
- `[x]`로 되돌리려면 IX.10.3의 project 연결 후, 실제 `stellar.yaml` 텍스트를 이전 앱 release와 새 앱 release로 열고 다시 기록해
  bytes·해석 결과가 같음을 확인해야 합니다.

## 재판정 (2026-11-23, ledger-verification)

- 판정: `[~]` 유지 — 구현 판정 통과, 선행 미충족.
- 근거: registry version과 project pin 분리, pin 기준 해석은 코드에 있습니다. "project bytes unchanged"는 실제 `stellar.yaml` 연결(IX.10.3 남은 범위)이 있어야 확인할 수 있고 선행 IX.10.3이 `[~]`라 `[~]`를 유지합니다.
- 판정 기준: 행 지시·검증 열의 불변식을 구현 코드에서 확인했고, 테스트 통과는 보조 근거로만 썼습니다. 계획서 §1.2 규칙 4(선행이 모두 `[x]`여야 완료)를 적용했습니다. 기준 커밋은 `main` `ce60bb92`입니다.

### 확인한 코드 위치

- `crates/target-profile/src/`: `RegistryCatalog`, project pin 기준 해석.

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test --workspace`(DEV-MAC, 샌드박스 해제) | 테스트 binary 142개 모두 ok, 4136 passed, 0 failed |
| `cargo test -p stellar-target-profile` | 42 passed |

### 남은 조건과 finding

- IX.10.3이 실제 manifest에 연결되면 이전 release와 새 release로 같은 `stellar.yaml`을 열고 다시 써 bytes가 같은지 확인합니다.
