# 작업 보고서: XVI.4.3 사전 정의 코드로만 sample 제외

- WBS: XVI.4.3
- 작업명: OS preemption, device lost, sleep transition 등 excluded sample은 사전 정의 오류 코드가 있을 때만 제외
- 상태: 부분 완료 (`[~]`) — 2026-11-23 ledger-refresh 판정: 구현 판정 통과, 선행 미충족(계획 1.2 규칙 4): 선행 XVI.4.2(`[~]`)가 `[x]`가 아닙니다(XVI.1.2 사슬). 이전 기록: 완료 (`[x]`)
- 결정: ADR-0084 결정 11
- 선행: XVI.4.2(이 lane), XI.5.4(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 측정 장비는 개발 장비(Apple M3 Max, 36GB)이고 기준 장비(M1 16GB)가 아닙니다.

## 변경 파일 목록

- `crates/diagnostics/src/analyzer.rs` — `ExclusionCode`(OS 선점·장치 상실·절전 전환·드라이버 정지).
- `docs/task-reports/XVI.4.3.md`(신규).

## 구현 요약

- `LatencySample::exclusion`에 닫힌 코드가 있을 때만 판정에서 빼고 코드별로 셉니다. 코드가 없는 고지연 sample은 그대로 셉니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-diagnostics` | lib 25 passed, doc 3 passed(compile_fail 포함), 0 failed |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `only_predefined_codes_exclude_samples`: 코드 없는 500ms sample은 max·초과 수에 남습니다. 같은 크기의 코드 있는 sample은 제외되고 코드별로 셉니다.

## 성능

- 해당 없음.

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 코드를 sample에 붙이는 감지(운영체제 선점·장치 상실 이벤트 연결)는 앱·renderer 경로 몫입니다. core replay sample은 모두 통제 가능으로 봅니다.
