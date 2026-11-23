# 작업 보고서: XXIX.2.12 원문 보기 버퍼

- WBS: XXIX.2.12
- 작업명: CLI 출력을 수신한 그대로 담는 세션별 메모리 상한 버퍼
- 상태: `[x]` 완료
- 선행: XXIX.2.4
- 설계 기준: ADR-0005 §4·§6·§8, ADR-0023 결정 9, ADR-0092

## 필수 실행 환경

- `DEV-MAC` (macOS 프로세스·파이프·process group·Code Signing Services, 로컬 검증). 네트워크와 실제 공급자 계정은 쓰지 않습니다.

## 변경 파일 목록

- `crates/agent-host/src/raw.rs`(신규): `RawBuffer`, `RawStream`, `RawSegment`, `neutralize_controls`, `DEFAULT_RAW_CAP_BYTES`
- `crates/agent-host/tests/raw_buffer.rs`

## 구현 요약

- stdout·stderr를 받은 순서대로 조각(stream 표시 포함)으로 담습니다. 같은 stream의 연속 조각은 64 KiB까지 합칩니다.
- 상한을 넘으면 오래된 byte부터 버립니다. 버린 byte 수와 잘림 사실을 남기고, 표시 텍스트 첫 줄에 `[앞부분 N byte 잘림]`을 넣습니다.
- 제어 문자는 해석하지 않습니다(PTY·terminal emulation 없음). 저장은 받은 byte 그대로입니다. 읽기 전용 표시 텍스트는 다음과 같이 바꿉니다.
  - C0(줄바꿈·탭 제외): Unicode Control Pictures(U+2400\~)
  - DEL: U+2421
  - C1: U+FFFD
  - 그래서 escape sequence가 표시 단계에서 실행되지 않습니다.
- 디코더 성공·실패와 관계없이 항상 먼저 원문에 넣습니다(`SessionOutput`).
- 세션 메모리에만 둡니다. 직렬화 수단이 없고 `Debug`는 크기만 씁니다.
- **상한 값 8 MiB(실측)**:
  - 8 MiB 상한에 32 MiB를 64 KiB 조각으로 넣는 비용은 debug 0.96 ms, release 1.8 ms입니다. 수신 경로 비용은 무시할 수준입니다.
  - 8 MiB 전체 표시 텍스트 준비는 release 19 ms, debug 267 ms입니다.
  - 표시 준비는 Agent Host·worker 스레드에서 하고(ADR-0005 §6) UI는 준비된 텍스트만 받습니다. 그래도 한 번에 준비하는 크기가 UI 갱신 주기에서 감당할 수 있는 수십 ms 안에 들도록 8 MiB로 정했습니다.

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
| 제어 문자 주입 fixture(OSC title, 화면 지우기, alt screen, NUL, DEL, C1 CSI): 원문 byte 그대로 보존, 표시 텍스트 제어 문자 0 | 통과 |
| 상한 초과(1 MiB 상한에 4 MiB+): 담은 byte ≤ 상한, 버린 + 담은 = 받은, 잘림 표시 | 통과 |
| 대역 32 MiB 출력: 8 MiB 상한 유지(XXIX.2.4 test) | 통과 |
| 진단(세션 종료 진단 기록)·`Debug`(출력 상태, 통계)에 내용 0 | 통과 |
| 디코드 실패 뒤에도 원문 보존 | 통과 |

- 성능: 위 실측 값(`cargo test -p stellar-agent-host --test raw_buffer -- --nocapture`, `--release`도 실행). 성능 예산 파일 변경 없음.

## 차이(작업 지시와 다른 점)

- recovery snapshot 경로는 이 crate에 없습니다. 원문 버퍼는 직렬화 수단이 없어 snapshot에 들어갈 수 없습니다(구조 확인).

## 남은 위험 / 한계

- 실측은 개발 Mac에서 다른 작업과 동시에 돌렸습니다. 표시 텍스트 증분 준비(전체 재생성 대신)는 원문 보기 UI(XXIX.4.4) 때 필요하면 더합니다.
