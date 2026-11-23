# 작업 보고서: XII.10.11 모서리 반경 핸들

- WBS: XII.10.11
- 작업명: 모서리 반경 핸들
- 상태: 부분 완료 (`[~]`) — 코어 완료, 셸 핸들 그리기 남음
- 결정: ADR-0105 결정 2, ADR-0071 UX.2.10
- 선행: 계획서 행의 선행 열

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, Apple M3 Max). 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 변경 파일 목록

- `crates/runtime/src/canvas/transform/radius.rs`(신규)

## 구현 요약

- 쓰기는 기존 `CanvasEdit::Radius` 경로 하나입니다(Frame·Rectangle 바뀐 `border-*-radius` longhand, Polygon stellar-only key 하나).
- `RadiusHandles`: 핸들 수치는 저장값, 표시 반경은 VI.5.2 평가(`effective_corner_radius`) 결과입니다.
- `RadiusHandles::edit`: pointer·수식키를 `gesture_math::preview`로 해석합니다. Option은 Frame·Rectangle에서 잡은 모서리만, Polygon은 같은 결과입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib -- canvas::transform::radius` | 통과(2): Frame·Rectangle 동시·개별·oversize(저장값 유지·표시 축소), Polygon 단일 key·Option 동일 |
| `cargo test -p stellar-runtime --lib -- app::canvas_gesture` | 통과(기존 반경 test 포함) |

## 차이(작업 지시와 다른 점)

- 묶음 key 0x0106 미사용은 기존 경로가 longhand key만 쓰는 것과 history 항목 문자열로 확인했습니다.
- 셸 UI·FFI·메뉴·`ShortcutPolicy` 부분은 사용자 방향(2026-11-23, ADR-0098)에 따라 하지 않았습니다. UI 재설계 때 합니다.

## 남은 위험 / 한계

- Polygon 표시 반경은 path 평가(VI.5.6)에 맡기고 핸들은 저장값만 보입니다.
