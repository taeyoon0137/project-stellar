# 작업 보고서: XII.10.37 줌·팬 입력의 camera 연결

- WBS: XII.10.37
- 작업명: 핀치·스크롤·⌘+스크롤·Shift+휠·Space 드래그·Zoom 도구·smart magnify를 camera에 연결
- 상태: 부분 완료 (`[~]`) — 코어 완료, 셸 입력 연결과 기준 장비 Present 측정 남음
- 결정: ADR-0117 결정 8.2
- 선행: 계획서 행의 선행 열

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, Apple M3 Max). 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 변경 파일 목록

- `crates/runtime/src/canvas/navigate.rs`(신규), `crates/runtime/src/canvas.rs`(mod 등록)

## 구현 요약

- `Navigator::apply(viewport, 화면 크기, 입력, 선택 경계)`가 `CanvasViewport`만 바꿉니다. `apply_to`는 창 active 탭 viewport(`set_viewport`)에 씁니다. 문서 geometry·history는 바뀌지 않습니다.
- 핀치: 커서 기준 `× (1 + magnification)`. 정밀 스크롤·관성: 팬, 관성 중 새 제스처가 시작되면 다음 관성 시작 전까지 남은 관성 event를 버립니다. ⌘+스크롤: 커서 기준 `× 2^(dy/100)`. 마우스 휠 Shift: 세로 값을 가로 팬. Hand·Space 드래그: pointer 화면 이동만큼 팬. Zoom 도구: 클릭은 다음 줌 단계, Option-클릭은 이전 단계(ADR-0104 표, 커서 고정), 영역 드래그(3pt 이상)는 world 영역 맞춤. smart magnify: 선택 경계 맞춤(여백 5%)과 이전 viewport를 번갈아, 선택이 없으면 100%와 이전 viewport.
- 배율 상하한은 viewport(`view_tabs`)가 맞추고, 상한에서 더 키우면 바뀌지 않음(`None`)입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib canvas::navigate` | 통과(5): 핀치·⌘+스크롤 커서 고정 줌과 상한, 스크롤 팬·Shift 휠 가로 팬·트랙패드 Shift, 관성 유지·새 제스처 뒤 관성 버림, Hand 드래그·Zoom 도구 클릭·Option-클릭·영역·3pt 미만 클릭·smart magnify 번갈기, 입력 연쇄 뒤 save bytes·history·revision 불변 |

## 차이(작업 지시와 다른 점)

- "Pan·Zoom replay Present 예약 10ms 이하"는 셸·render thread가 없어 Present 예약을 잴 수 없습니다. 코어 처리(입력 → viewport → scene camera) 시간을 XII.10.43 캔버스 편집 replay의 `pan`·`zoom` scenario로 쟀습니다(그 보고서 참조). 기준 장비 Present 판정은 XVI.2.3 full replay 몫입니다.
- 입력 변환(FFI)은 XII.10.36의 이유로 셸 재설계 때 합니다.

## 남은 위험 / 한계

- 관성 event 차단 규칙은 macOS 관성 event 순서(제스처 Ended 뒤 momentum Began)를 가정합니다. 셸 연결 때 실제 event 순서로 다시 확인해야 합니다.
