# 작업 보고서: XI.8.2 resize와 drawable 획득, drawable 없음 frame skip

- WBS: XI.8.2
- 작업명: resize와 drawable acquisition 구현, drawable 없음은 frame skip으로 처리
- 상태: 부분 완료 (`[~]`)
  - 구현과 backend 처리 경로 테스트는 DEV-MAC 실제 GPU에서 통과했습니다.
  - 검증 열 "minimized/occluded window crash 없음"을 실제 창이 아니라 재현(0 크기 resize, `#[cfg(test)]`
    nil drawable 주입)으로만 확인했습니다. 계획 §1.3 규칙 5에 따라 모방한 증거로 `[x]`를 두지 않습니다
    (아래 "상태 이력").
- 선행: XI.8.1 (이 lane에서 `df9a2d2`로 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (Apple M3 Max, 실제 GPU, 창 밖 CAMetalLayer drawable readback).

## 변경 파일 목록

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

## 차이(작업 지시와 다른 점)

- "occluded window"는 실제 창 없이 재현했습니다. 창 밖 CAMetalLayer는 drawable 수 제한이 걸리지 않아
  (probe: 연속 8개 획득) nil drawable이 자연적으로 나오지 않으므로, nil 반환은 `#[cfg(test)]` 주입으로
  만들고 그 뒤 처리 경로는 실제 GPU로 확인했습니다. "minimized window"는 셸이 넘기는 0 크기 resize로
  재현했습니다.

## 남은 위험 / 한계

- 실제 NSWindow 최소화·가림에서 CanvasHost가 0 크기를 넘기는지, `nextDrawable`이 실제로 1초 timeout 후
  nil을 주는지는 창이 필요한 XII.4.1에서 확인해야 합니다.
- 창 밖 layer는 drawable을 제한 없이 주므로 discard 테스트는 drawable 반환을 간접적으로만
  확인합니다(GPU 미제출·후속 frame 정상).
- `nextDrawable` 대기(최대 1초)는 render thread를 막습니다. drawable timeout event화는 XI.8.7 범위입니다.
- 남은 증거(실제 NSWindow를 최소화·가렸을 때 CanvasHost에 붙은 layer로 crash 없이 frame skip)는 CanvasHost
  layer 연결(XII.4.1)이 있어야 얻을 수 있습니다. 그런데 XII.4.1은 이 행을 선행으로 두므로, 남은 증거를
  XII.4.1 검증에 포함할지 별도 검증 행을 둘지는 계획 변경 결정이 필요합니다(이 수정에서는 계획 행 문구를
  바꾸지 않았습니다).

## 상태 이력

- 2026-11-14 첫 구현(`cfe5323`): `[x]`.
- 2026-11-14 리뷰 반영: `[~]`로 정정. 사유는 검증 열의 최소화·가림 창을 실제 창 없이 재현으로만 확인한
  것입니다. 최소화는 셸이 넘길 0 크기 resize로, 가림은 `#[cfg(test)]` `drawable_failures` 주입(nil
  drawable)으로 만들었고, 그 뒤 처리 경로만 실제 GPU로 확인했습니다. 실제 NSWindow 최소화·가림, 실제
  `nextDrawable` timeout은 실행하지 않았습니다.
- 같은 리뷰에서 Metal device가 없는 환경의 GPU 테스트가 검증 없이 통과하던 것을 고쳤습니다(`ded1435`,
  기본 실패, `STELLAR_ALLOW_MISSING_METAL=1`일 때만 건너뜀). 이 행의 테스트 결과는 그대로입니다.
