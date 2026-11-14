# 작업 보고서: IX.10.2 profile 종류별 필드와 필수 필드 검증

- WBS: IX.10.2
- 작업명: Apple(플랫폼 종류·최소 OS), Android(API level·UI 시스템 버전), Web(브라우저 기준선·capability 집합 버전) 필드 정의
- 상태: 완료 (`[x]`)
- 선행: IX.10.1(이 lane `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/target-profile/src/spec.rs`(신규) — `DottedVersion`, `ApplePlatform`, `AndroidApiLevel`, `WebBaseline`,
  `CapabilitySetVersion`, `AppleProfile`·`AndroidProfile`·`WebProfile`, `ProfileSpec`, `TargetProfile`.
- `crates/target-profile/src/node.rs` — `ValueError`에 `OutOfRange`·`InvalidVersion`·`UnknownApplePlatform` 추가.
- `crates/target-profile/src/lib.rs` — 모듈 등록.
- `docs/task-reports/IX.10.2.md`(신규), 계획서(IX.10.2 `[ ]`→`[x]`).

## 구현 요약

- **종류별 필드**(모두 필수):
  - Apple: `apple_platform`(`ios`·`ipados`·`macos`·`watchos`·`tvos`·`visionos`, code 1~6), `minimum_os`.
  - Android: `api_level`(1~65535), `ui_system_version`.
  - Web: `baseline`(Baseline 연도 2000~9999), `capability_set_version`(1 이상).
- **version**: `DottedVersion`은 `major[.minor[.patch]]`(각 `u16`, 선행 0 금지)이고 (major, minor, patch) 사전순으로
  비교합니다(`17.4` < `17.10`). 텍스트는 정규화합니다(`17` → `17.0`, `17.4.0` → `17.4`).
- **검증**: 종류별 필수 필드 누락은 `MissingField`(경로 포함), 다른 종류의 필드가 섞이면 `UnknownKey`로 거부합니다
  (조용히 무시하지 않음). 값 오류는 `InvalidValue`로 경로와 규칙만 담습니다.
- **`TargetProfile`**: 종류는 `ProfileSpec`에서 정해지므로 종류와 필드가 어긋난 상태를 만들 수 없습니다. canonical 키
  순서는 머리(`id`, `platform`, `profile_schema_version`) 다음 종류별 표 순서입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-target-profile` | 11 passed, 0 failed |
| `cargo clippy -p stellar-target-profile --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-target-profile -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 세 종류 canonical 노드 golden, 모델→노드→모델·노드→모델→노드 동일 (`every_kind_round_trips_with_canonical_keys`) | 통과 |
| profile별 required field validation: 종류마다 필수 필드를 하나씩 뺀 입력은 경로가 있는 `MissingField`, 다른 종류 필드가 섞인 입력은 `UnknownKey` (`required_fields_are_validated_per_kind`) | 통과 |
| 값 검증(Apple 플랫폼 이름 대소문자, version 형식 8종, API level 0·65536·선행 0, baseline 1999·`modern`, 집합 버전 0)과 version 정규화 (`field_values_are_validated`) | 통과 |
| version 사전순 비교, Apple 플랫폼 code 1~6과 이름 역변환 (`version_ordering_and_platform_codes`) | 통과 |

## 차이(작업 지시와 다른 점)

- Web `baseline`은 Baseline 연도 정수로 정했습니다. 기술 아키텍처 20.3 예시의 `baseline: modern`처럼 이름 기준선은
  읽지 않습니다(결정론적 비교를 위해). 기존 project `TargetProfile`(문자열 `baseline`·`minimum_os`)을 이 모델로 바꾸는
  연결은 project crate 작업 범위입니다.
- 결정 사항은 허용 경로 밖인 `docs/adr/` 대신 모듈 문서와 이 보고서에 기록했습니다.

## 남은 위험 / 한계

- Android `ui_system_version`이 가리키는 UI 시스템(예: Compose·Material 버전)의 의미는 capability registry(IX.10.5)가
  해석합니다. 이 작업은 형식만 고정합니다.

## 후속 수정(리뷰 반영): Apple `minimum_os` 범위

IX.10.7 Binding 환경 `os_version` 인코딩(`major × 1000 + minor`, 리뷰 TP-3)이 정확히 담는 범위만 받도록 Apple
`minimum_os`에 major 9999·minor 999 상한을 추가했습니다. 범위 밖은 `ValueError::OutOfRange`입니다. 다른 `DottedVersion`
필드(Android `ui_system_version`)는 바꾸지 않았습니다. 자세한 내용과 검증은 IX.10.7 보고서의 후속 수정 절에 있습니다.
