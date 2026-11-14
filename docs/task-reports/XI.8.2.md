# 작업 보고서: XI.8.2 resize와 drawable 획득, drawable 없음 frame skip

- WBS: XI.8.2
- 작업명: resize와 drawable acquisition 구현, drawable 없음은 frame skip으로 처리
- 상태: 부분 완료 (`[~]`)
  - 구현과 backend 처리 경로 테스트는 DEV-MAC 실제 GPU에서 통과했습니다.
  - 후속 실제 NSWindow harness에서 최소화·복귀·resize·숨김/재표시와 Metal frame 실행을 확인했습니다.
  - 실제 앱 활성화(`active=false`, `key=false`)와 다른 창에 의한 가림 상태(`covered=false`)는 이 실행 세션에서
    확인하지 못했습니다. harness는 종료 코드 1을 반환하며 `[~]`를 유지합니다. 최초 nil 주입 검증과 실제 창 증거는
    아래에서 구분합니다.
- 선행: XI.8.1 (이 lane에서 `df9a2d2`로 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (Apple M3 Max / Mac15,11, macOS 27.2 / 26B123, 실제 GPU).
- 최초 검증은 창 밖 CAMetalLayer readback, 이번 후속 검증은 화면에 표시한 NSWindow + CAMetalLayer입니다.
  CI-MAC에 실제 GPU·대화형 데스크톱을 필수로 추가하지 않습니다.

## 변경 파일 목록

이번 후속 변경:

- `crates/renderer-metal/examples/window_surface.rs` — 명시적으로 실행하는 DEV-MAC 실제 창 harness.
- `docs/task-reports/evidence/XI.8.2-window-surface.txt` — 이번 실행의 원시 프레임·창 상태 기록.
- `docs/task-reports/XI.8.2.md` — 실제 창 검증 결과와 미확인 범위 갱신.
- 제품 코드·FFI·생성 헤더·계획 상태·원장은 변경하지 않았습니다.

최초 구현 파일(이번에는 수정하지 않음):

- `crates/renderer-metal/src/surface.rs` — `SurfaceStats`(resize·skip 통계), `Drawable::size`, resize·skip
  규칙 모듈 문서.
- `crates/renderer-metal/src/backend.rs` — `resize`(같은 크기 no-op, 0 크기는 layer에 넣지 않음, 상한
  검사), `acquire_drawable`(nil → skip, 크기 불일치 → layer 재동기화 후 skip), `surface_stats`,
  `surface_size`, 테스트 주입 `drawable_failures`(`#[cfg(test)]`).
- `crates/renderer-metal/src/lib.rs` — `SurfaceStats` 공개.
- `crates/renderer-metal/src/tests/surface.rs`(신규), `tests/mod.rs` — XI.8.2 테스트.
- `docs/task-reports/XI.8.2.md`(신규), 계획서(XI.8.2 `[ ]`→`[x]`).

## 구현 요약

- **0 크기(최소화)**: `resize(0×N)`은 크기만 기억하고 layer `drawableSize`는 마지막 유효 값을 유지합니다.
  그 동안 surface frame은 `nextDrawable`을 부르지 않고 `Skipped(EmptySurface)`입니다(최소화된 창에서
  drawable 대기 없음). 복원 resize가 오면 새 크기를 layer에 넣고 다시 그립니다.
- **drawable 없음(가려짐·timeout)**: `nextDrawable`이 nil이면 오류가 아니라 `Skipped(NoDrawable)`이며
  frame을 열지 않습니다(XI.8.1의 임시 `Timeout` 오류를 대체). `allowsNextDrawableTimeout`은 항상
  켜져 있어 최대 1초 뒤 nil로 돌아옵니다.
- **크기 재동기화**: 얻은 drawable texture 크기가 backend 크기와 다르면(셸이 layer 크기를 직접 바꿈)
  layer를 backend 크기로 다시 맞추고 그 frame을 `Skipped(NoDrawable)`로 건너뜁니다. 크기가 맞지 않는
  attachment로 render pass를 만들지 않습니다.
- **resize 규칙**: 같은 크기는 no-op(layer·통계 불변), 한 변 16384 초과는 `InvalidUsage`(이전 크기 유지),
  열린 frame 중 resize는 `InvalidUsage`(XI.8.1).
- **통계**: `SurfaceStats { resizes, skipped_empty, skipped_no_drawable, drawable_size_resyncs }`를 device
  재생성에도 누적합니다(XI.11.5 dropped frame 보고에 쓸 수 있음).

## 실행한 명령 / 검증 (`DEV-MAC`)

후속 실제 창 검증:

| 명령 | 종료 코드 / 결과 |
|---|---|
| `cargo build -p stellar-renderer-metal --example window_surface` | 0 |
| `./target/debug/examples/window_surface` | 1 — 최소화·복귀·resize·숨김/재표시 통과, 활성화·가림 미확인 |
| `open -n -W --stdout /tmp/stellar-window-bundle-final.log --stderr /tmp/stellar-window-bundle-final-error.log target/window-surface.app` | launcher 0, 자식 출력 `covered=false activated=false` — 검증 성공 아님 |
| `cargo test -p stellar-renderer-metal` | 0 — 54 passed, 0 failed, 0 ignored |
| `cargo test -p stellar-renderer-metal --all-targets` | 0 — backend 54 + 기존 objc helper 2 passed, 창 harness main은 자동 실행되지 않음 |
| `cargo clippy -p stellar-renderer-metal --all-targets -- -D warnings` | 0 — 경고 0 |
| `cargo fmt -p stellar-renderer-metal -- --check` | 0 |
| `cargo xtask validate-task-report docs/task-reports/XI.8.2.md` | 0 |
| `git diff --check` | 0 |

최초 구현 당시 검증(기존 증거):

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-metal` | 15 passed, 0 failed |
| `cargo clippy -p stellar-renderer-metal --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-metal -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 0×0 resize 뒤 5 frame 모두 `Skipped(EmptySurface)`·validator `Idle`, layer 크기 16×8 유지, 32×12 복원 뒤 drawable readback 32×12 blue, 통계 일치(`minimized_surface_skips_without_requesting_drawables`) | 통과 |
| frame 하나를 GPU에 제출한 채 nil drawable 3회 → 3 frame `Skipped(NoDrawable)`·frame 미개방, 제출한 frame 정상 완료, 다음 frame 정상 렌더(`occluded_surface_without_drawable_skips_frames`) | 통과 |
| 64×32 → 0×0 → 64×32 → 1×1 → 0×17 → 17×0 → 128×96 → 3×5 연속 resize, 크기마다 readback 크기·내용·layer 크기 일치, 같은 크기 no-op, 16385 거부 후 크기 유지(`resize_sequence_renders_at_each_size`) | 통과 |
| layer 크기를 backend 모르게 7×7로 바꾸면 한 frame skip 후 16×8로 재동기화, 다음 frame 정상(`layer_size_changed_behind_backend_is_resynced`) | 통과 |
| present 예약 후 discard 8회 반복해도 GPU 제출 없음, 이후 frame 정상(`discarded_frame_releases_its_drawable`) | 통과 |

## 실제 창 후속 검증

- 재실행: `cargo run -p stellar-renderer-metal --example window_surface`. 기본 unit test·smoke·CI 실행 목록에는
  추가하지 않았습니다. Metal/WindowServer가 없거나 상태 전이가 확인되지 않으면 비0으로 종료합니다.
- 원시 결과: [XI.8.2-window-surface.txt](evidence/XI.8.2-window-surface.txt). 로그의 `Presented`는 backend의
  `poll().presented`가 해당 submission의 GPU 완료를 확인했다는 뜻이며, 화면 픽셀 readback·사용자 육안 확인은 아닙니다.
- 기존 제품 진입점 `MetalBackend::with_layer`와 `RendererBackend::{init, resize, encode, schedule_present,
  submit, poll}`을 그대로 씁니다. `drawable_failures`나 테스트 전용 backend API는 호출하지 않습니다.
- AppKit 생성·NSApplication.run·event 처리는 main thread, backend 생성·frame 실행·해제는 scoped render worker에서
  수행합니다. worker가 끝날 때까지 main의 layer 소유 참조를 유지하고, worker join 뒤 창을 닫습니다. 기본 frame은
  BGRA8 sRGB surface clear이며 각 GPU 제출 완료를 확인합니다. 이 harness가 제품의 frame scheduler는 아닙니다.

| 실제 관측 | frame 결과 |
|---|---|
| 최초 표시 `visible=true`, scale 2 | 960×640, submission 1 완료 |
| `miniaturize:` 뒤 `isMiniaturized=true`, `visible=false` | 0×0 resize 후 `Skipped(EmptySurface)` 3회 |
| `deminiaturize:` 뒤 `isMiniaturized=false`, `visible=true` | 960×640 복구, submission 2 완료 |
| 더 큰 불투명 floating NSWindow를 실제 대상 frame보다 20pt씩 넓게 덮음 | cover는 표시됐으나 대상 `visible=true` 유지. 가림 frame 검증 미실행 |
| cover 제거 | submission 3 완료(가림 성공 증거로 세지 않음) |
| `setContentSize:` 520×360pt | 1040×720, submission 4 완료 |
| `orderOut:` 뒤 `visible=false` | 0×0, `Skipped(EmptySurface)` 1회 |
| `orderFrontRegardless` 뒤 `visible=true` | 1040×720, submission 5 완료 |
| 종료 | worker join 후 창 닫기, 총 resize 5 / empty skip 4 / no drawable skip 0 |

최소화·가림 자체가 자동으로 bounds를 0으로 만든다고 가정하지 않습니다. harness가 실제 AppKit 상태를 확인한 뒤
0 크기를 backend에 전달하는 정책을 검증합니다. `orderOut:` 증거를 다른 창에 의한 가림 증거로 바꾸어 기록하지 않습니다.

활성화 실험은 직접 실행, transient `.app` 번들 LaunchServices 실행, 정상 `NSApplication.run` event loop를 모두
사용했습니다. 번들은 같은 실행 파일을 `target/window-surface.app/Contents/MacOS/window_surface`에 복사하고
`CFBundleIdentifier=dev.stellar.window-surface-check`, `CFBundleExecutable=window_surface`, `CFBundlePackageType=APPL`,
`NSHighResolutionCapable=true`인 Info.plist로 만들었습니다(저장소 추적 제외 빌드 디렉터리).
`makeKeyAndOrderFront:`·activate 요청 뒤에도 `active=false`, `key=false`였고, 4초 event 처리 안에 가림 상태도
관측되지 않았습니다. 이 결과만으로 다른 로그인 세션에서도 창 활성화가 불가능하다고 단정하지 않습니다.

## 차이(작업 지시와 다른 점)

- 실제 NSWindow harness로 선행 검증 경로를 독립시켰지만 활성화·가림 검증이 끝나지 않았습니다. 사용자 지시에 따라
  독립적으로 검증된 harness와 증거만 커밋하고, XI.8.2 `[~]`와 XII.4.1 `[ ]`를 유지합니다.
- XII.4.1 제품 CanvasView 연결, display scale 전환, release rollback 통합은 이번에 착수하지 않았습니다.
  지정된 `session-takeover-integration.md`와 `XII.4.1-blocked.md`도 수정하지 않았습니다.
- 원장 상태와 공개 ABI 변화가 없으므로 생성기 실행·생성물 변경은 없습니다.

## 남은 위험 / 한계

- 활성화 가능한 DEV-MAC 데스크톱에서 동일 harness의 실제 가림 전이·복귀를 재검증해야 합니다. 가림 시 실제 nil drawable이
  나오는지는 미확인입니다. 기존 nil 주입 테스트의 증거 범위를 확대하지 않습니다.
- 제품 CanvasView에서 실제 최소화·가림을 0 크기로 전달하는 lifecycle, detach·close·실패 rollback, scale 변경은
  XII.4.1 구현·검증 대상입니다. 현재 harness는 그 제품 경로가 아닙니다.
- 이번 2x scale 관측은 display scale change 검증이 아닙니다. GPU 완료 기록도 presented 화면의 픽셀 검증이나 T5 성능
  게이트를 대신하지 않습니다. 성능 측정은 수행하지 않았습니다.
- `nextDrawable`의 대기는 기존 backend와 같이 render worker에서 발생합니다. 완료 polling에는 4초, host 응답에는 8초
  제한을 두지만 OS/GPU 호출 자체가 반환하지 않는 경우까지 강제로 중단하는 harness는 아닙니다.

## 상태 이력

- 2026-11-14 첫 구현(`cfe5323`): `[x]`.
- 2026-11-14 리뷰 반영: `[~]`로 정정. 사유는 검증 열의 최소화·가림 창을 실제 창 없이 재현으로만 확인한
  것입니다. 최소화는 셸이 넘길 0 크기 resize로, 가림은 `#[cfg(test)]` `drawable_failures` 주입(nil
  drawable)으로 만들었고, 그 뒤 처리 경로만 실제 GPU로 확인했습니다. 실제 NSWindow 최소화·가림, 실제
  `nextDrawable` timeout은 실행하지 않았습니다.
- 같은 리뷰에서 Metal device가 없는 환경의 GPU 테스트가 검증 없이 통과하던 것을 고쳤습니다(`ded1435`,
  기본 실패, `STELLAR_ALLOW_MISSING_METAL=1`일 때만 건너뜀). 이 행의 테스트 결과는 그대로입니다.
- 2026-11-14 실제 창 후속: 최소화·복귀·resize·order-out/in의 실제 프레임 증거 추가. 앱 활성화와 다른 창에 의한 가림은
  미확인이라 `[~]` 유지. 독립 harness로 검증 경로를 만들었으므로 선행 순환을 풀기 위한 계획 변경은 필요하지 않습니다.
