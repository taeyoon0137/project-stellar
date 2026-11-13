# 작업 보고서: XV.1.4 의존성 URL·local override 경로 민감값 분류

- WBS: XV.1.4
- 작업명: dependency URL과 local override path를 telemetry 민감값으로 분류
- 상태: 완료 (`[x]`)
- 선행: II.3.2(`[x]`), XV.1.1(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/dependencies/src/sensitivity.rs`(신규) — `DataClass`, `DeclarationField`, redaction
  `Debug`(`RepositoryUrl`·`LocalOverridePath`), `DeclarationDiagnostic`, `DeclarationTelemetry`,
  `DependencyDeclaration::diagnostic`/`telemetry`, compile_fail doctest.
- `crates/dependencies/src/declaration.rs` — 두 민감 타입의 `Debug` derive 제거, 원문 접근자를
  `as_str` → `reveal`로 변경.
- `crates/dependencies/src/selection.rs`, `crates/dependencies/src/validation.rs` — `reveal` 사용.
- `crates/dependencies/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XV.1.4.md`(신규), 계획서(XV.1.4 `[ ]`→`[x]`).

## 구현 요약

- **분류(아키텍처 39장)**: repository URL·local override 경로는 `LocalSourceOnly`(로컬 UI 표시만,
  telemetry 금지, 진단 export에서도 가림). logical name·library id·선택자 값(브랜치·커밋 포함)은
  `LocalDiagnostic`(telemetry 금지). source·선택자 종류·override 유무만 `TelemetrySafe`입니다.
- **타입 강제**: 두 민감 타입은 `Debug`가 `<redacted>`만 출력하고 `Display`가 없어 `format!`에 쓸 수
  없습니다(compile_fail doctest). 원문은 II.3.2 `Sensitive::reveal`과 같은 이름의 `reveal()`로만
  얻습니다. 선언 전체 `Debug`도 자동으로 가려집니다.
- **진단·telemetry 요약**: `DeclarationDiagnostic`(Display에 `repository=<redacted>`),
  `DeclarationTelemetry`(정적 문자열·불리언만 — II.3.2 `TelemetryEvent`와 같은 원칙).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 15 passed, 0 failed; doctest 1 passed(compile_fail) |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| URL·override 경로 = `LocalSourceOnly`, 선택자 값 = `LocalDiagnostic`, telemetry 허용 필드 3개뿐(`url_and_override_path_are_local_source_only`) | 통과 |
| 선언 `Debug`·진단 요약 Display/Debug에 URL·경로·host·사용자 이름 미포함, redaction 표식 존재, `reveal`로만 원문(`debug_and_diagnostic_output_are_redacted`) | 통과 |
| telemetry 요약에 URL·경로·이름·branch 미포함(`telemetry_carries_only_safe_fields`) | 통과 |
| `RepositoryUrl`을 `format!("{url}")`에 쓰면 컴파일 실패(doctest) | 통과 |

## 차이(작업 지시와 다른 점)

- 의존 방향상 `stellar-diagnostics`를 쓸 수 없어 같은 원칙(민감 래퍼 + 정적 telemetry 필드)을 이
  crate 타입으로 구현했습니다. 실제 telemetry 이벤트 연결은 project/셸 연결 작업에서
  `DeclarationTelemetry`만 넘기도록 합니다.

## 남은 위험 / 한계

- `DeclNode`(파일 내용 트리)의 `Debug`는 URL 스칼라를 그대로 보여 줍니다. 파일 형식 표현이므로
  가리지 않았고, 진단·로그에 노드를 기록하지 않는 규칙으로 관리해야 합니다.
- `RepositoryUrl::comparison_key` 결과도 민감값입니다(문서에 명시). 출력 경로에 쓰지 않습니다.

## 상태 정정(통합 리뷰, 2026-11-14)

- `[x]` → `[~]`: 엄격 선행 사슬에 `[~]` 행(XV.1.1 등)이 있어 함께 `[~]`로 둡니다. 이 행 자체의 검증은 dependencies crate 안에서 통과했습니다.

## 상태 복원(deps_wiring lane, 2026-11-14)

- `[~]` → `[x]`: 엄격 선행 II.3.2·XV.1.1 `[x]`.
- 실제 파서 경로 검증: 실제 파서가 만든 모델의 `Debug`·진단·telemetry와 스키마 오류 메시지에 URL·override 경로 원문이 없음(`manifest::tests::parsed_dependency_values_stay_redacted_in_debug_and_diagnostics`, `dependency_schema_violations_are_rejected_by_stellar_yaml_parser`).
- 공통 근거: `Fix: stellar.yaml 의존성 선언을 실제 매니페스트 파서에 연결`(리뷰 P2), `Fix: stellar.lock.yaml entry 형식을 IV.1.6 모델 하나로 통합`(리뷰 P3, ADR-0016), `Fix: resolver library 매니페스트 읽기를 실제 stellar.yaml 파서로 연결`.
- 실행(`DEV-MAC`): `cargo test -p stellar-project` lib 261 passed·통합 17 passed, `cargo test -p stellar-dependencies` 57 passed·doctest 1 passed, `cargo clippy -p stellar-project -p stellar-dependencies --all-targets -- -D warnings` 경고 0, `cargo fmt -p stellar-project -p stellar-dependencies -- --check` 통과.
