# 작업 보고서: XXIX.2.7 세션 실행 전 확인

- WBS: XXIX.2.7
- 작업명: ADR-0005 §4.3 실행 대상 확인과 XXIX.2.5 연결 상태를 확인하고, 확인되지 않으면 실행하지 않음
- 상태: `[x]` 완료
- 선행: XXIX.2.6
- 설계 기준: ADR-0005 §4·§6·§8, ADR-0023 결정 9, ADR-0092

## 필수 실행 환경

- `DEV-MAC` (macOS 프로세스·파이프·process group·Code Signing Services, 로컬 검증). 네트워크와 실제 공급자 계정은 쓰지 않습니다.

## 변경 파일 목록

- `crates/agent-host/src/preflight.rs`(신규): `preflight`, `PreflightInput`, `PreflightPass`, `PreflightRefusal`
- `crates/agent-host/tests/preflight.rs`

## 구현 요약

- 순서는 profile 활성 → 실행 대상 신원(XXIX.2.3) → 버전 범위 → 연결 상태(XXIX.2.5)입니다. 앞 단계가 실패하면 뒤 단계와 세션 실행을 하지 않습니다(fail closed).
- 버전 출력은 4 KiB까지만 보고 첫 `숫자.숫자.숫자`를 씁니다. 종료 코드 0이 아니거나 찾지 못하면 판정 불가로 거부합니다.
- 통과하면 `PreflightPass`(외부에서 만들 수 없는 증표)를 돌려줍니다. 세션 프로세스는 `PreflightPass::launch`로만 시작할 수 있습니다. launch는 실행 직전 실행 파일 지문을 다시 확인합니다.
- 거부 이유는 고정 코드(`profile-inactive`, `target-identity`, `version-undeterminable`, `version-out-of-range`, `not-connected`, `scratch`)이며 내용을 담지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-agent-host` | 통과(아래 표의 대상별 수, 실패 0) |
| `cargo clippy -p stellar-agent-host --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-agent-host -- --check` | 통과 |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |
| `cargo xtask validate-adr --base edfea9db` | 종료 코드 0 |
| `STELLAR_CHECK_BASE=edfea9db cargo xtask check-core` | lane 최종 실행 결과는 lane 보고(통합 기록)에 적음 |

| 검사 | 결과 |
|---|---|
| 확인 실패 fixture 5종(신원 실패, 버전 판독 불가, 버전 명령 실패, 연결 안 됨, 상태 판정 불가): 세션 실행 0, scratch 정리 | 통과 |
| 정상 fixture: 확인만으로는 실행 0, launch 뒤 실행 1, 서명 확인 호출 1 | 통과 |
| 확인 뒤 실행 파일이 바뀌면 launch 거부(`TargetChanged`), 실행 0 | 통과 |

- 성능 예산 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 세션 시작 확인 데이터(대상 프로젝트·CLI·모드 확인 기록, 약관 고지)는 XXIX.3.5 범위입니다.
