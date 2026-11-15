# 작업 보고서: XII.10.26 자유 배치 Frame 안에 자식 Frame 드래그 생성

## WBS

- WBS: XII.10.26
- 상태: 부분 완료(`[~]`, `reported_done`)

구현 커밋 `7702b089a8d94cb0470d1e645781dad9984c7eac`에서 macOS 기본 창의 F 도구가 자유 배치 Frame 안에 자식 Frame을 생성합니다. 선행 행은 XII.10.1, VII.2.1, XII.10.9입니다.

## 변경 파일

- `apps/macos/Stellar/UI/Shell/ShellCanvasFrameInsert.swift`와 `apps/macos/Stellar/CanvasFrameScene.swift`에서 누른 부모를 식별하고 화면 영역을 부모 좌표로 변환합니다.
- `crates/ffi/src/window_session.rs`와 `crates/runtime/src/app/frame_insert.rs`에서 창의 신뢰·표시 중인 View·문서 revision·부모를 확인한 뒤 기존 `InsertNode` 명령을 실행합니다.
- `crates/history/src/window.rs`에서 이전 선택과 생성 후 선택을 같은 문서 이력에 기록합니다. `apps/macos/Stellar/UI/Shell/ShellFrameInsertCheck.swift`와 `scripts/macos/smoke.sh`가 실제 앱 경로를 검사합니다.
- 이 보고서와 함께 구현 계획의 XII.10.26 상태를 변경하고 `planning/task-status.yaml`에 생성된 해당 행을 반영했습니다.

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `./scripts/macos/smoke.sh` | 0, `frame-insert ok`와 `smoke OK` |
| `cargo test --locked --offline -p stellar-runtime -p stellar-history -p stellar-document -p stellar-ffi` | 0, 합계 626개 통과 |
| `cargo clippy --locked --offline -p stellar-runtime -p stellar-history -p stellar-document -p stellar-ffi --all-targets -- -D warnings` | 0 |
| `cargo xtask generate-ledger`, `cargo xtask sync-plan --check` | 각각 0 |
| `cargo xtask validate-task-status`, `cargo xtask validate-task-report docs/task-reports/XII.10.26.md` | 각각 0 |
| `git diff --cached --check` | 0 |

## 환경과 증거

DEV-MAC, macOS 27.2 arm64. smoke는 로컬 Xcode로 빌드한 `Stellar.app`에서 임시 폴더에 프로젝트를 만들고 실제 창 responder에 합성 AppKit 키·마우스 이벤트를 전달했습니다.

## 테스트

- 새 프로젝트의 800×600 루트에서 F를 누르고, 현재 배율을 100%에서 Zoom Out 한 단계(50%)로 낮췄습니다. 문서 좌표 (100, 90)에서 (40, 30)까지 역방향 드래그해 자식 하나가 생기고 Move 도구로 돌아갔습니다. 마우스를 움직인 두 번의 갱신과 놓기까지 문서 revision과 Undo 깊이는 각각 한 번 증가했습니다.
- Undo가 추가한 자식과 선택을 되돌리고 Redo가 같은 ID를 선택했습니다. Save Now로 저장한 뒤 창을 닫고 같은 폴더를 다시 열어 자식 안 두 점과 경계 바로 밖의 부모를 확인했습니다. 이어서 위치가 (40, 30)인 자식을 부모로 삼아 다시 그린 Frame의 안팎을 검사했습니다.
- 제한 모드에서 생성을 시도하거나 Escape로 미리보기를 취소하면 revision이 유지됩니다. 화면 양축이 3pt에 미달하는 드래그는 이력을 추가하지 않습니다.
- Rust 테스트 `insert_and_selection_share_an_entry_and_save_real_geometry`가 다른 View의 부모와 오래된 revision의 거부, 기존 선택 복원, 마지막 형제 뒤 순서, 저장 후 다시 읽은 크기·변환을 검사합니다.

## 성능

입력 지연이나 대규모 문서에서의 Present 시간은 측정하지 않았습니다.

## 차이

계획에 있는 다섯 생성 도구 중 자유 배치 Frame의 일반 드래그 경로를 구현했습니다. 후속 도구에서도 `InsertNode`와 생성 전후 선택을 한 entry에 담는 기존 경로를 재사용할 수 있습니다.

## 최초 Frame 드래그 검증 당시의 범위

- Rectangle·Ellipse·Line·Polygon 생성과 클릭 기본 크기 생성이 남았습니다.
- Shift 정비례·45도, Option 중심 기준, 드래그 중 Space로 시작점 이동, 흐름 컨테이너의 pointer 위치에 맞춘 삽입 순서가 남았습니다.
- 합성 이벤트를 사용했습니다. 물리 마우스와 트랙패드의 드래그 감각, 회전된 부모에서의 실기 조작은 미검증입니다.

## 후속 구현 — Ellipse·Line·Polygon

- 자유 배치 부모의 두 local 점으로 실제 Shape를 생성합니다. Ellipse·Polygon은 Transform과 Fill, Line은 시작점 Transform과 Stroke를 기록합니다. 선의 로컬 끝점에는 방향 부호를 보존합니다.
- 클릭 기본값은 Ellipse·Polygon 100×100, Line은 오른쪽 100입니다. Polygon은 기존 도구의 `pentagon` 아이콘에 맞춘 5변, 위쪽 첫 꼭짓점, 모서리 반경 0입니다.
- `drawn_shapes_preserve_local_geometry_paint_selection_and_history_on_reopen`은 중첩 Frame 속 다섯 입력(역방향 Ellipse·Polygon, 수평·수직·대각 Line)의 한 번 Undo·이전 선택, Redo·ID, 재열기 후 실제 payload·modifier stack을 검사합니다. 같은 revision으로 재시도한 요청과 길이 0인 Line도 검사합니다.
- 세 도구는 Shift 정비례·45도와 Option 중심 기준을 지원합니다. 드래그 중 오각형 미리보기는 위의 첫 꼭짓점에서 시작해 실제 경로와 같은 다섯 꼭짓점을 연결합니다.

## 남은 위험

- Frame·Rectangle의 클릭 기본 크기와 수식키, 생성 드래그 중 Space로 시작점 이동, 흐름 컨테이너 삽입은 미구현입니다.
- 물리 마우스·트랙패드 입력과 화면 지연은 별도 실기 측정이 필요합니다.
