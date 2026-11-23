# 작업 보고서: XVI.4.5 결과 보존 schema와 privacy

- WBS: XVI.4.5
- 작업명: 결과를 commit, hardware profile, OS build, app build와 함께 보존하되 사용자 project 정보 미포함
- 상태: 부분 완료 (`[~]`) — 2026-11-23 ledger-refresh 판정: 구현 판정 통과, 선행 미충족(계획 1.2 규칙 4): 선행 XVI.4.4(`[~]`)가 `[x]`가 아닙니다(XVI.1.2 사슬). 이전 기록: 완료 (`[x]`)
- 결정: ADR-0084 결정 11
- 선행: XVI.4.4(이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 측정 장비는 개발 장비(Apple M3 Max, 36GB)이고 기준 장비(M1 16GB)가 아닙니다.

## 변경 파일 목록

- `crates/diagnostics/src/result.rs`(신규).
- `docs/task-reports/XVI.4.5.md`(신규).

## 구현 요약

- `stellar.performance-result.v1` JSON의 key는 고정(`RESULT_KEYS`)입니다. 담는 값은 다음과 같습니다.
  - commit(소문자 hex 40·64), hardware model·CPU·메모리, OS build, app build
  - synthetic fixture(닫힌 이름·seed·규모·hash), build profile
  - scenario 통계·구간 p99·scene hash, gate
- 환경 문자열은 생성 때 형식을 검사합니다. `/`·`\`·`:`·`@`를 막아 경로·URL·이메일을 넣을 수 없습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-diagnostics` | lib 25 passed, doc 3 passed(compile_fail 포함), 0 failed |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `result_json_uses_only_allowed_keys`: 모든 key가 허용 목록, 경로·URL 문자 없음.
- `rejects_project_like_strings`: branch 이름·대문자 hex·경로·git URL·빈 값·`~/` 경로·URL 거부.

## 성능

- 해당 없음.

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- CI 보존·추이 비교(아키텍처 §38.3)는 이 행 밖입니다.
