# 작업 보고서: XII.10.21 정렬·분배·행열 정리

- WBS: XII.10.21
- 작업명: 정렬·분배·행열 정리
- 상태: 부분 완료 (`[~]`) — 코어 완료, 메뉴·`ShortcutPolicy` 남음
- 결정: ADR-0105 결정 1·6
- 선행: 계획서 행의 선행 열

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, Apple M3 Max). 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 변경 파일 목록

- `crates/commands/src/arrange/{align,plan}.rs`(신규)
- `crates/runtime/src/canvas/transform/arrange.rs`(신규)

## 구현 요약

- 정렬 6종(단일은 부모, 다중은 선택 경계 또는 primary 기준 객체), 분배 2종(축 중심 순서 첫·끝 고정), 행열 정리(읽는 순서, `ceil(√n)`열, 흐름 컨테이너를 만들지 않음).
- 회전 node는 world 경계 기준입니다. 계획은 `arrange::plan` 하나이고 상자 원천만 runtime(scene)·CLI·MCP(문서 값 `ModelGeometry`)로 다릅니다.
- 흐름 자식: flex 주축 정렬 비활성(`justify-content` 안내), 교차축 `align-self`(wrap-reverse 반영), grid `justify-self`·`align-self`, 분배·행열 정리 비활성.
- 명령 하나가 CommandTransaction 하나입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib -- canvas::transform::arrange` | 통과(9): `canvas/align-distribute-matrix`(6종 단일·다중·기준 객체, 분배, 행열, undo 원복), 회전 node world 경계, flex 교차축·grid에서 바뀐 longhand가 self 정렬뿐, 비활성 조합 revision 불변 |
| `cargo test -p stellar-commands --lib -- arrange` | 통과: 정렬·분배·행열 순수 계산, 흐름 판정(css·react-native 세트, absolute 자식), `ModelGeometry` |

## 차이(작업 지시와 다른 점)

- ADR-0074가 Overlay 같은 비CSS 배치를 없앴으므로 "Overlay 자식" 규칙은 대응할 것이 없습니다. 옛 `layout_mode` Frame의 자식은 모두 비활성입니다.
- 행열 정리 기본 간격 10px은 XII.10.2 값이 다르면 바꿉니다.
- 위치·선형부는 지금 저장 위치(Frame `transform`, 그 밖은 위치 Transform modifier)에 씁니다. IX.11.9 step이 제품 목록에 없고 scene이 layout record 위치를 그리지 않기 때문입니다(ADR-0086 결정 3, ADR-0105 결정 1). step이 등록되면 `stellar_commands::arrange::place` 한 곳만 바꿉니다.
- 셸 UI·FFI·메뉴·`ShortcutPolicy` 부분은 사용자 방향(2026-11-23, ADR-0098)에 따라 하지 않았습니다. UI 재설계 때 합니다.

## 남은 위험 / 한계

- CLI·MCP 명령 연결은 새 ID 원천(ops)과 CLI module이 소유 밖이라 하지 않았습니다. 계획은 `arrange::plan`·`ModelGeometry`로 바로 쓸 수 있습니다.
