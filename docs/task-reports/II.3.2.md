# 작업 보고서: II.3.2 구조화 로그 이벤트와 민감 타입

- WBS: II.3.2
- 작업명: diagnostics에 구조화 telemetry 이벤트 + 민감 타입 분리
- 상태: 완료 (`[x]`)
- 선행: II.3.1, II.2.7 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/diagnostics/src/lib.rs` (구조화 이벤트·민감 타입)
- `docs/task-reports/II.3.2.md`, 계획서(II.3.2 `[ ]`→`[x]`)

## 구현 요약

- `Sensitive`(프로젝트 내용·텍스트·경로 래퍼). `TelemetryEvent { name: &'static str, code: u32 }`(비민감, 런타임 문자열 불가). 민감 데이터는 `SensitiveDiagnostic`로 분리.
- 민감 타입을 비민감 telemetry 필드에 넣으면 컴파일 실패(`compile_fail` doctest로 검증).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 단위 테스트 2 | 통과 |
| doctest 2(정상 + `compile_fail`) | 통과 |
| fmt / clippy --locked | 통과 |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- trybuild 미승인이라 compile-fail은 rustdoc `compile_fail` doctest로 구현했습니다(`cargo test --doc`로 검증).
