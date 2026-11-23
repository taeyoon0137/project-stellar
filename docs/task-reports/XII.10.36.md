# 작업 보고서: XII.10.36 확대 제스처 입력

- WBS: XII.10.36
- 작업명: 확대 제스처(Magnify, phase 포함)와 smart magnify 입력
- 상태: 부분 완료 (`[~]`) — runtime 입력 모델 완료, FFI `InputEventKind`·`StellarMacEvent` 변환과 version 올림 남음
- 결정: ADR-0117 결정 8.1
- 선행: 계획서 행의 선행 열

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, Apple M3 Max). 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 변경 파일 목록

- `crates/runtime/src/input_gesture.rs`(신규), `crates/runtime/src/lib.rs`(mod 등록)

## 구현 요약

- `NavigationInput`: `Magnify { phase, magnification, location }`(phase는 began·changed·ended·cancelled), `SmartMagnify { location }`, 스크롤(정밀 여부·phase·관성 phase·수식키), Hand 드래그, Zoom 도구 클릭·영역. 좌표는 캔버스 화면 pt입니다.
- `validate`가 비유한 값과 `magnification <= -1`을 거부합니다.
- camera 연결은 XII.10.37의 `canvas::navigate`가 이 타입을 받습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib input_gesture` | 통과(1): 확대 phase 네 종류와 smart magnify 값 검사, 비유한·-1 이하 배율 거부 |
| `cargo xtask generate-ffi-header --check` | drift 없음(FFI를 바꾸지 않음) |

## 차이(작업 지시와 다른 점)

- 계획서 행은 `crates/ffi`의 `InputEvent`·`StellarMacEvent`에 magnify 종류·필드를 더하고 `STELLAR_MAC_EVENT_VERSION`·`INPUT_EVENT_SCHEMA_VERSION`을 올리며 ADR-0014에 절을 더하라고 합니다. 사용자 방향(2026-11-23: 셸 FFI를 더하지 않음, UI 재설계 때 함)과 작업 소유 범위 때문에 FFI는 바꾸지 않았습니다. runtime 쪽 입력 모델을 먼저 두고, FFI 변환은 셸 재설계 때 이 타입으로 옮깁니다(ADR-0117 결정 8.1).
- 그래서 "새 kind round-trip, 이전 version 거부 fixture, ADR-0014 갱신"은 아직 없습니다.

## 남은 위험 / 한계

- FFI 변환 단계에서 `NSEvent.magnification`·phase 값을 이 타입으로 옮길 때 phase 대응표(began·changed·ended·cancelled, mayBegin 무시)를 정해야 합니다.
