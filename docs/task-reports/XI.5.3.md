# 작업 보고서: XI.5.3 resource upload queue와 frame-local allocator interface

- WBS: XI.5.3
- 작업명: resource upload queue와 frame-local allocator interface 정의, 3 frames-in-flight에서 premature free 방지
- 상태: 완료 (`[x]`)
- 선행: XI.5.2 (이 lane에서 `b3c8d51`로 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/frame_resources.rs`(신규) — `FrameLocalAllocator` trait,
  `RingFrameAllocator`, `FrameAllocation`, `FrameAllocError`, `DeferredRelease<T>`, `UploadQueue`,
  `UploadRequest`, `UploadTarget`, `UploadFlush`, `UploadError`, `upload_bytes_per_pixel`.
- `crates/renderer-core/src/frame_resources/tests.rs`(신규) — 느린 GPU 모사 simulation과 단위 테스트.
- `crates/renderer-core/src/backend/mod.rs` — `destroy_*` 문서를 "공통 계층이 GPU 완료 뒤에만 호출"
  계약으로 고침(XI.5.1 문서의 "backend가 해제를 미룸"을 대체).
- `crates/renderer-core/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XI.5.3.md`(신규), 계획서(XI.5.3 `[ ]`→`[x]`).

## 구현 요약

- **frame-local allocator**: `FrameLocalAllocator` trait(`begin_frame` → `allocate`* → `end_frame(submission)`
  | `abandon_frame`, `retire(completed)`, `reset`)과 구현 `RingFrameAllocator`입니다. frames in flight
  수(1..=3)만큼 slot을 돌려 쓰는 bump allocator이며, backend는 slot마다 같은 크기 buffer를 둡니다.
  `end_frame`한 slot은 그 submission이 끝났다는 `retire` 전까지 다시 열리지 않고, 열려 하면
  `SlotBusy { slot, submission }`입니다(호출자는 GPU를 기다린 뒤 재시도 → frame pacing). 제출하지 않고
  버린 frame의 slot은 GPU가 보지 않았으므로 바로 다음 frame이 씁니다. alignment(1..=256, 2의 거듭제곱)·
  0 크기·용량 초과·offset 넘침을 panic 없이 거부합니다.
- **지연 해제(`DeferredRelease<T>`)**: 파괴 요청을 그 resource를 참조했을 수 있는 마지막 submission
  완료까지 보관합니다. 기록 중 frame에서 요청하면 그 frame의 submission까지, frame 사이에서 요청하면
  마지막 submission까지 기다리고, 버린 frame에서 요청한 항목은 마지막 submission 기준으로 돌아갑니다.
  `collect(completed)`가 해제 가능 항목을 돌려주고, device lost에는 `drain_all`로 모두 돌려받습니다.
  이에 맞춰 `RendererBackend::destroy_*`는 "공통 계층이 GPU 완료 뒤에만 호출하므로 backend는 즉시
  해제해도 된다"는 계약으로 정리했습니다.
- **upload queue**: FIFO `UploadQueue`가 buffer·texture upload를 모아 encode 전에 byte 예산만큼
  `update_buffer`/`update_texture`로 넘깁니다. 예산보다 큰 첫 요청도 한 번에 하나는 넘겨 진행을
  보장하고, backend 오류 시 그 요청을 앞에 남겨 재시도합니다. push 시 빈 데이터·범위 넘침·0 크기 영역·
  depth format·길이 불일치(영역 × pixel byte)를 거부하고, 파괴할 resource의 대기 upload를
  `cancel_texture`/`cancel_buffer`로 버려 stale handle upload를 막습니다.
- GPU는 단일 queue에서 제출 순서대로 끝낸다고 가정합니다(`completed = s`는 `s` 이하 완료).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core` | 63 passed, 0 failed; doctest 3 passed(compile_fail) |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | exit 0 |

| 검사 | 결과 |
|---|---|
| 3 frames in flight, 400 frame, frame당 GPU 완료 평균 0.5개인 느린 GPU 모사: GPU가 frame을 끝낼 때 그 frame의 frame-local bytes가 그대로이고(덮어씀 0) 참조 texture가 모두 살아 있음(premature free 0). slot 대기 발생, 최대 in-flight 3, 버린 frame 포함, 지연 해제 요청 전부 결국 해제(`three_frames_in_flight_never_free_early`) | 통과 |
| 위 simulation의 검출력: slot busy 검사를 끈 allocator는 덮어씀으로, 마지막 submission 대기를 끈 지연 해제는 premature free로 실패함을 임시 변경으로 확인(변경은 되돌림) | 확인 |
| slot은 retire 뒤에만 다시 열림, 버린 frame slot 즉시 재사용, 정렬 offset, 용량 초과(`ring_slots_reopen_only_after_retire`) | 통과 |
| 잘못된 frames in flight·용량·alignment·0 크기·offset 넘침 거부(`allocator_rejects_invalid_requests`) | 통과 |
| 기록 중·frame 사이·버린 frame·완료 뒤 요청의 해제 시점, 완료 번호 단조, device lost drain(`deferred_release_waits_for_the_last_possible_use`) | 통과 |
| upload FIFO·예산 분할·큰 요청 진행 보장, mock backend 호출 순서(`upload_queue_flushes_fifo_within_budget`) | 통과 |
| upload 검증 5종, 파괴 대상 upload 취소, backend 오류 후 재시도(`upload_queue_validates_cancels_and_retries`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 XI.5.2는 명시 row-ID이고 이 lane에서 먼저 `[x]`가 되었습니다. 완화 적용은 없습니다.
- XI.5.1 trait 문서의 `destroy_*` 계약(backend가 해제를 미룸)을 공통 계층 책임으로 바꿨습니다. 같은
  lane의 미배포 interface라 호환 영향은 없습니다.
- simulation의 GPU는 mock backend가 아니라 메모리·texture 생존을 직접 검사하는 테스트 전용 모사입니다
  (mock backend는 메모리 내용을 모델링하지 않음). upload queue는 mock backend로 검증했습니다.

## 남은 위험 / 한계

- frame slot 용량을 넘는 frame 데이터는 `OutOfSpace`이며, 전용 buffer로 대체하는 정책은 Metal
  backend(XI.8.3)에서 정합니다.
- upload 예산 기본값은 정하지 않았습니다. interaction frame 예산(아키텍처 §6.2)에 맞춘 값은 benchmark
  (XI.11.4) 뒤 정합니다.
- 다중 queue(copy queue 분리)는 가정하지 않습니다. 도입하면 완료 순서 가정을 queue별로 바꿔야 합니다.
