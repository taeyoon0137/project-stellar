# 작업 보고서: IX.10.6 선택 profile 기준 생성 가능 여부·비활성화 사유 조회

- WBS: IX.10.6
- 작업명: selected TargetProfile에 따라 element·property 생성 가능 여부와 비활성화 사유를 조회하는 API
- 상태: 완료 (`[x]`)
- 선행: IX.10.5(이 lane `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/target-profile/src/query.rs`(신규) — `CapabilityQuery`, `CreationAvailability`, `DisabledReason`.
- `crates/target-profile/src/lib.rs` — 모듈 등록.
- `docs/task-reports/IX.10.6.md`(신규), 계획서(IX.10.6 `[ ]`→`[x]`).

## 구현 요약

- **입력**: `RegistryCatalog::resolve`로 해석한 `ResolvedTargetProfile`(profile + pin한 release). 조회는 pin한 release의
  capability 표만 쓰므로 앱 업데이트로 결과가 바뀌지 않습니다.
- **생성 정책**: Native·Approximated는 생성 가능(근사 여부는 `status`로 전달), Unsupported·Deprecated·미등록은 불가입니다.
  Deprecated는 새 생성만 막고 기존 사용은 IX.10.8 진단으로 다룹니다.
- **비활성화 사유**: `Unsupported { kind, available_from }`(같은 종류의 더 높은 수준에서 Native·Approximated가 되면 그 최소
  수준, 예: API 31·Baseline 2023), `Deprecated { replacement }`, `NotRegistered`. `describe()`가 UI용 한국어 설명을 만듭니다.
- **API**: `subject`·`element`·`property`·`contract`, palette용 `filter`(입력 순서 보존)와 `creatable`.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-target-profile` | 27 passed, 0 failed |
| `cargo clippy -p stellar-target-profile --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-target-profile -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| Button/TextInput/List 6개 profile 전부 Native 생성 가능, Modal Web 2021 근사(생성 가능)·2022 이후 Native; 대표 CSS(`subgrid` Web 2022 불가·Baseline 2023부터, `backdrop_filter` Web 근사·Android 불가, `clip_rect` Deprecated·대체 `clip_path`), Apple(`sf_symbol` iOS Native·Android 근사, `material_vibrancy` Android 불가), Android(`dynamic_color` API 30 불가·API 31부터, API 35 Native, `status_bar_color` API 35 Deprecated·API 30 Native, `ripple` macOS 근사), element, 미등록 subject (`controls_and_representative_properties_per_profile`) | 통과 |
| `filter` 입력 순서 보존, `creatable`이 비활성 subject 제외, 사유 설명 문구 (`filter_keeps_input_order_and_creatable_drops_disabled`) | 통과 |

## 차이(작업 지시와 다른 점)

- 실제 UI 연결은 지시대로 Phase XII로 미룹니다. 이 crate는 조회 API만 제공합니다.
- 생성 정책(Deprecated 생성 불가)은 허용 경로 밖인 `docs/adr/` 대신 모듈 문서와 이 보고서에 기록했습니다.

## 남은 위험 / 한계

- 사유 설명의 subject 표기는 numeric(`property:0x00000005`)입니다. 사람이 읽는 이름은 UI 계층이 `builtin::property_name`
  등으로 바꿔 표시해야 합니다.
