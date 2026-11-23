# 작업 보고서: XVI.2.1 입력 재생 파일 형식

- WBS: XVI.2.1
- 작업명: 입력 재생 파일에 초기 project fixture hash, viewport, InputEvent sequence, ManualClock timeline 정의
- 상태: 완료 (`[x]`)
- 결정: ADR-0084 결정 9
- 선행: XII.3.1(`[x]`), XVI.1.1(이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 측정 장비는 개발 장비(Apple M3 Max, 36GB)이고 기준 장비(M1 16GB)가 아닙니다.

## 변경 파일 목록

- `tools/xtask/src/replay/file.rs`(신규).
- `tools/xtask/src/replay.rs`, `tools/xtask/src/lib.rs` — 모듈 등록.
- `tools/xtask/Cargo.toml`, `Cargo.lock` — core replay 내부 crate 의존 등록.
- `docs/task-reports/XVI.2.1.md`(신규).

## 구현 요약

- `stellar-replay v1` 줄 단위 텍스트입니다. 담는 값은 다음과 같습니다.
  - synthetic fixture 명세(이름·seed·규모)와 content hash(64 hex)
  - viewport, scenario
  - `event <ns> down|move|up|wheel <a> <b>`
- event는 공통 `InputEvent`의 pointer·wheel 부분과 같은 의미이고, 시각은 ManualClock timeline ns로 감소하지 않습니다.
- 같은 값은 같은 텍스트로 씁니다(f32 가장 짧은 왕복 표기). 파일은 프로젝트 밖에 두며 사용자 문서를 가리키지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p xtask --lib replay::file` | 2 passed |
| `cargo test -p xtask --lib replay` | 7 passed, 0 failed |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `replay_file_round_trips`: parse → to_text 바이트 동일.
- `rejects_malformed_files`: header·역순 시각·NaN·모르는 scenario·사용자 fixture 이름·중복 줄·hash 누락을 줄 번호와 함께 거부.

## 성능

- 해당 없음.

## 차이(작업 지시와 다른 점)

- `InputEvent`(stellar-ffi) 타입을 직접 쓰지 않고 같은 의미의 재생 event를 정의했습니다. xtask가 셸 경계 crate에 의존하지 않게 하려는 것입니다. key event는 아직 형식에 없습니다(재생 scenario에 필요 없음).

## 남은 위험 / 한계

- key·modifier 입력 재생은 텍스트 편집 scenario를 더할 때 형식을 넓혀야 합니다(version 올림).
