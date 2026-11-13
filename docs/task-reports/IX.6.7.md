# 작업 보고서: IX.6.7 contract registry와 target export hint

- WBS: IX.6.7
- 작업명: contract별 유효성 registry와 target export hint 정의(코드 생성 없음)
- 상태: 완료 (`[x]`)
- 선행: IX.6.2~IX.6.6(이 lane에서 모두 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/control/registry.rs`(신규) — `REGISTRY`, `ContractDescriptor`, `FieldDescriptor`,
  `ExportHints`/`ExportTarget`, `KNOWN_KINDS`, `descriptor`, `check_registry`/`check_registry_entries`,
  `validate_contracts`, `RegistryError`.
- `crates/document/src/control/mod.rs` — 모듈 등록.
- `docs/task-reports/IX.6.7.md`(신규), 계획서(IX.6.7 `[ ]`→`[x]`).

## 구현 요약

- **descriptor**: 종류마다 안정 이름, field 표(id·이름·필수 여부), 검사기, target export hint(Web·Apple·Android).
  알려진 종류는 Button·TextInput·Modal·List·SingleSelectionGroup 5종입니다.
- **export hint**: target에서 대응하는 구성 요소 **이름**만 담습니다(예: Button → `button`/`Button`/`Button`,
  SingleSelectionGroup → `div[role=radiogroup]`/`Picker(.radioGroup)`/`Modifier.selectableGroup`). 코드 생성은
  하지 않습니다(기술 아키텍처 17.3 로드맵).
- **누락 검사**(`check_registry_entries`): 알려진 종류가 빠짐없이·중복 없이 있는지, field 표가 종류 모듈의 field
  목록(`fields::ALL`)과 정확히 같은지, 모든 target hint가 비어 있지 않은지 검사합니다.
- **표 검사**(`validate_contracts`): contract 표의 모든 계약을 종류별 검사기로 검사합니다(frame 순). registry에 없는
  종류는 데이터를 보존한 채 `UnknownKind` 진단을 냅니다(12.5: 계약 목록은 닫힌 enum이 아님).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 304 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 내장 registry 누락 검사 통과, 종류마다 필수 field 1개, 항목 제거·중복·field 표 불일치·빈 hint는 실패(`registry_has_no_missing_entries`) | 통과 |
| hint 이름 조회, 모르는 종류 descriptor 없음(`export_hints_are_names_only`) | 통과 |
| 표 검사가 종류별 검사기 결과와 같고 모르는 종류는 `UnknownKind`(`table_validation_dispatches_by_kind_and_flags_unknown_kinds`) | 통과 |

## 차이(작업 지시와 다른 점)

- target 종류는 Target Profile(IX.10.1, 미구현)이 정의할 예정이라, 이 registry는 hint 조회용 `ExportTarget`
  (Web·Apple·Android)을 따로 두었습니다. IX.10.1이 생기면 그 종류로 바꿉니다.

## 남은 위험 / 한계

- capability 상태(Native·Approximated 등, IX.10.5)는 이 registry를 입력으로 삼는 후속 작업입니다.
