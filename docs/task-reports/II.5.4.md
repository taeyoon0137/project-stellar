# 작업 보고서: II.5.4 CanvasHost drawable/scale 전달 구조

- WBS: II.5.4
- 작업명: 최소 `NSView` CanvasHost가 backing scale·drawable 크기 변경을 코어에 전달(렌더링 없음)
- 상태: 완료 (`[x]`)
- 선행: II.5.3 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (로컬 Xcode 27.0). CI-MAC 연결은 II.7.1.

## 변경 파일 목록

- `crates/ffi/src/lib.rs` (`Drawable` 보관 + `stellar_runtime_set_drawable_size`/`stellar_runtime_drawable_size`, 단위테스트)
- `tools/xtask/src/ffi_header.rs` + `crates/ffi/include/stellar_ffi.h` (새 함수 2개 헤더 반영·재생성)
- `apps/macos/StellarMac/CanvasMetrics.swift` (pixel = point × scale 계산)
- `apps/macos/StellarMac/CanvasView.swift` (resize·backing 변경 시 코어로 전달)
- `apps/macos/StellarMac/CanvasWindowController.swift`, `AppDelegate.swift` (handle 연결)
- `apps/macos/StellarMac/CanvasCheck.swift`, `main.swift` (`--canvas-check`)
- `scripts/macos/smoke.sh` (canvas 검사 추가)
- `docs/task-reports/II.5.4.md`, 계획서(II.5.4 `[ ]`→`[x]`)

## 구현 요약

- 코어: `Runtime`에 `Drawable { pixel_width, pixel_height, scale }`을 보관. `stellar_runtime_set_drawable_size`로 받고 `stellar_runtime_drawable_size`로 읽음(렌더링 없음 — 전달/보관 구조만). 미설정 시 `NoError`.
- 새 FFI 2개를 `ffi_header.rs` 고정 템플릿에 추가하고 헤더를 재생성(II.4.6 drift 검사 통과).
- 셸: `CanvasMetrics.pixelSize(point, scale) = round(point × scale)`. `CanvasView`가 `viewDidMoveToWindow`/`viewDidChangeBackingProperties`/`setFrameSize`에서 drawable 픽셀 크기·scale을 계산해 코어로 전달.
- `CanvasWindowController`가 runtime handle을 `CanvasView`에 주입.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| Rust `drawable_size_round_trip`(set→get, 미설정 NoError, null 무시, InvalidHandle) | 통과(11 tests) |
| `generate-ffi-header --check`(drift) | drift 없음 |
| `StellarMac --canvas-check`(scale 1/2/3 포함 4 fixture, pixel==point×scale + 코어 round-trip) | `canvas ok`, 종료 0 |
| `scripts/macos/smoke.sh`(smoke+lifecycle+canvas) | 종료 0 |
| `cargo fmt --all --check` / clippy(ffi,xtask) `-D warnings` | 통과 |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- "resize fixture"는 실제 화면 의존 `convertToBacking` 대신 결정적 `CanvasMetrics`(point × scale)로 검증했습니다. `CanvasView`도 동일 함수를 사용하므로 런타임 경로와 검증 경로가 일치합니다.
- drawable 전달을 위해 코어에 setter/getter FFI를 추가했습니다(plan의 "코어에 전달하는 구조").
