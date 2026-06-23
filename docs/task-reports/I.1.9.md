# 작업 보고서: I.1.9 cargo xtask help 진입점

- WBS: I.1.9
- 작업명: `.cargo/config.toml` alias + `tools/xtask` `help` 라우터
- 상태: 완료 (`[x]`)
- 선행: I.1.8 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `.cargo/config.toml` (신규, `xtask = "run --package xtask --"`)
- `tools/xtask/src/main.rs` (빈 main → `help` 라우터)
- `docs/task-reports/I.1.9.md` (신규, 본 보고서)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 갱신: I.1.9 `[~]` → `[x]`, 1.1 요약표·상단 상태 정리)

## 구현 요약

- 표준 라이브러리만 사용. 명령 `help`만 지원, 인자 없음·알 수 없는 명령은 usage를 stderr 출력 후 `64` 종료.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 | 종료 코드 |
|---|---|---|
| `cargo xtask help` | 사용법 stdout | 0 |
| `cargo xtask` | usage stderr | 64 |
| `cargo xtask bogus` | usage stderr | 64 |
| `cargo fmt -p xtask -- --check` | — | 0 |
| `cargo clippy -p xtask --all-targets -- -D warnings` | — | 0 |
| `cargo test -p xtask --locked` | — | 0 |

## 남은 경고 수

- 0

## 성능 수치

- 해당 없음

## 작업 지시와 다른 점

- 없음. 이 커밋으로 I.1.6~I.1.9 이력 정합화가 완료되어 1.1 현재 상태표를 갱신했습니다.
