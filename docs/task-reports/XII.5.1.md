# 작업 보고서: XII.5.1 macOS 앱 UI 텍스트 네이티브 컨트롤

- WBS: XII.5.1
- 작업명: macOS 레이어명·검색·일반 필드에 NSTextField/NSTextView를 사용하고 core text engine을 호출하지 않음
- 상태: 완료 (`[x]`)
- 선행: II.5.3 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (로컬 Xcode, `scripts/macos/smoke.sh`의 headless `--text-input-check`). 사람의 수동 IME 입력은 수행하지 않았습니다.

## 변경 파일 목록

- `apps/macos/StellarMac/NativeTextControls.swift`(신규) — 레이어명 `NSTextField`, 검색 `NSSearchField`, 일반 `NSTextField`, 여러 줄 `NSTextView` 팩토리.
- `apps/macos/StellarMac/NativeFieldsPanel.swift`(신규) — 위 컨트롤을 배치한 최소 패널.
- `apps/macos/StellarMac/CanvasWindowController.swift` — 창 content를 `NSSplitView`(왼쪽 패널 + 오른쪽 `CanvasView`)로 구성, `fieldsPanel` 접근자.
- `apps/macos/StellarMac/TextInputCheck.swift`(신규), `apps/macos/StellarMac/main.swift` — `--text-input-check` headless 검사.
- `scripts/macos/smoke.sh` — 코어 FFI 비호출 구조 검사와 `--text-input-check` 추가.
- `docs/task-reports/XII.5.1.md`(신규), 계획서(XII.5.1 `[ ]`→`[x]`).

## 구현 요약

- **컨트롤**: 레이어명·일반 값은 한 줄 plain `NSTextField`, 검색은 `NSSearchField`, 여러 줄 입력은 `NSTextView`
  (`isRichText = false`, `allowsUndo = true`)를 씁니다. 입력·선택·undo·IME marked text는 AppKit 텍스트 시스템이 처리하고,
  두 파일(`NativeTextControls.swift`, `NativeFieldsPanel.swift`)은 `stellar_` FFI를 호출하지 않습니다.
- **창 배치**: 앱 창 왼쪽에 `NativeFieldsPanel`(검색·레이어명·값·메모), 오른쪽에 기존 `CanvasView`를 둡니다.
  필드의 목록·바인딩·검색 동작 같은 상세 UX는 연결하지 않았습니다.
- **headless smoke(`--text-input-check`)**: 앱이 여는 `CanvasWindowController` 창을 화면에 표시하지 않고 만든 뒤,
  여러 줄 `NSTextView`와 각 한 줄 필드의 AppKit field editor에서
  1) `NSTextInputClient.setMarkedText`로 한글 조합 중간(ㅎ→하→한, ㄱ→그→글)과 marked range를 확인하고 `insertText`로 확정,
  2) 빈 marked text로 조합 취소 후 확정 text 보존,
  3) 창 undo manager의 `undo`/`redo`로 확정 입력 되돌림·재적용,
  4) 편집 종료 시 필드 값 반영(`"한글"`)을 확인합니다.
- smoke.sh가 두 컨트롤 파일에 `stellar_` 호출이 없는지 grep으로 검사합니다(코어 텍스트 엔진 비호출).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `scripts/macos/smoke.sh` | `BUILD SUCCEEDED`(Swift warning/error 0), smoke·lifecycle·canvas·text-input OK, 종료 0 |
| `cargo test -p stellar-ffi` | 25 passed, 0 failed (Rust 변경 없음) |
| `cargo clippy -p stellar-ffi --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-ffi -- --check` | 통과 |
| `cargo run -p xtask -- generate-ffi-header` 후 `git diff --exit-code -- crates/ffi/include/stellar_ffi.h` | diff 없음 |
| `cargo xtask validate-crate-graph` | 종료 0 |
| `git diff --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **IME smoke**: NSTextView·레이어명 NSTextField·NSSearchField·일반 NSTextField에서 한글 marked text 조합·확정·취소 | 통과 |
| **native undo smoke**: 같은 컨트롤에서 확정 입력 undo(`'한글'`→`''`)·redo(`'한글'`) | 통과 |
| 컨트롤 파일의 코어 FFI(`stellar_`) 호출 없음(smoke.sh 구조 검사) | 통과 |
| 검사 민감도: `allowsUndo = false`로 임시 변경 시 `--text-input-check`가 실패(종료 1)함을 확인 후 원복 | 확인 |

## 차이(작업 지시와 다른 점)

- IME smoke는 시스템 한글 입력 소스를 사람이 전환해 타이핑하는 대신, IME가 text view에 호출하는 `NSTextInputClient`
  API(`setMarkedText`/`insertText`)를 직접 호출하는 headless 자동 검사로 수행했습니다.
- 레이어 목록·검색 결과·속성 패널이 아직 없어, 같은 컨트롤 팩토리를 쓰는 최소 패널로 대체했습니다.

## 남은 위험 / 한계

- 실제 입력 소스(한글·일본어 IME)의 후보 창 표시, dead key, 받아쓰기는 자동 검사하지 않았습니다. 수동 확인 또는
  UI automation이 별도로 필요합니다.
- 이번 검사는 편집 중 undo/redo만 확인했습니다. 편집 종료 후 field editor undo와 문서 수준 undo(창별 history)의
  관계는 XII.6.4에서 다룹니다.
- 패널의 접근성 label·focus 순서는 XII.8.1, native material/theme는 XII.9.1 소관입니다. 일반 실행 창의 시각 배치는
  사람이 확인하지 않았습니다.
