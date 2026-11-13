# 작업 보고서: XI.7.1 worker 전용 이미지 decoder interface와 decoded pixel format

- WBS: XI.7.1
- 작업명: 이미지 decoder interface를 worker 전용으로 정의하고 decoded pixel format(RGBA8) 명시
- 상태: 완료 (`[x]`)
- 선행: VI.8.1 (`develop`에서 완료: `ec94b15`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/image_decode.rs`(신규) — `ImageDecoder` trait, `DecodeWorkerScope`,
  `decode_on_worker`, `DecodedPixelFormat`, `DecodedImage`, `ImageHeader`, `DecodeLimits`,
  `ImageDecodeError`. 테스트 전용 참조 decoder(binary PPM `P6`)와 손상 corpus.
- `crates/renderer-core/src/thread_role.rs`(신규) — `ThreadRole`(Ui/Document/Render/Worker),
  `bind_thread_role`, `ThreadRoleGuard`, `current_thread_role`, `ThreadRoleError`.
- `crates/renderer-core/src/lib.rs` — 모듈 등록.
- `crates/scene/src/lib.rs` — renderer-core용 `ColorSpace` 재노출.
- `docs/task-reports/XI.7.1.md`(신규), 계획서(XI.7.1 `[ ]`→`[x]`).

## 구현 요약

- **worker 전용**: thread 시작 시 역할을 thread-local로 한 번 bind합니다(재 bind 거부, guard drop 시
  해제, guard `!Send`). `ImageDecoder`의 `probe`/`decode`는 `&DecodeWorkerScope`를 요구하고,
  scope는 `ThreadRole::Worker` thread에서만 `DecodeWorkerScope::current()`로 얻을 수 있으며
  `!Send`·`!Sync`(compile_fail doctest로 고정)입니다. 따라서 UI·Document·Render thread에서는
  decoder를 호출할 수 없고 `decode_on_worker`는 `NotOnWorkerThread`를 반환합니다.
- **decoded pixel format**: `Rgba8Unorm` 하나로 고정 — R,G,B,A 순서, 채널당 8-bit unorm, RGB는
  이미지 색공간(sRGB/Display P3)의 비선형 인코딩 값, 알파는 선형·straight(core-types `Color`와
  같은 규약), row-major·좌상단 원점·padding 없음(stride = width×4). 기타 색공간은 decoder가
  sRGB/P3로 변환하고, 색공간 정보가 없으면 sRGB로 간주합니다. premultiply는 XI.7.2 소관입니다.
- **손상 입력 방어**: `decode_on_worker`가 입력 크기 → probe → header 상한(0 크기, 변 16384,
  64 Mi pixel, buffer 크기 넘침) → decode → header와 출력(크기·색공간) 일치 순으로 검사합니다.
  거대한 header는 pixel buffer 할당 전에 거부됩니다. `DecodedImage::new`가 buffer 길이를 검증하므로
  계약을 어긴 출력은 만들 수 없습니다. release가 `panic = "abort"`이므로 `catch_unwind`에 의존하지
  않고 무 panic을 구현 계약으로 명시했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core` | 18 passed, 0 failed; doctest 1 passed(compile_fail) |
| `cargo test -p stellar-scene` | 2 passed, 0 failed |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo clippy -p stellar-scene --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 통과 |
| `cargo fmt -p stellar-scene -- --check` | 통과 |
| `cargo xtask validate-crate-graph` | exit 0 |

| 검사 | 결과 |
|---|---|
| 손상 corpus 905건(수작업 24 + 유효 파일 절단 41 + byte 변형 328 + 무작위 512) panic 없이 처리, 228건 Ok·677건 Err, Ok 출력은 모두 format 계약 충족(`malformed_corpus_does_not_panic`) | 통과 |
| 대표 손상 입력의 오류 분류(형식 불명, 0 크기, 숫자 넘침, 상한 초과, data 길이 불일치, 미지원 maxval)(`specific_malformed_inputs_map_to_errors`) | 통과 |
| 계약 위반 decoder 출력(크기·색공간 불일치) 거부, 0 크기·상한 초과 header에서 decode 미호출(`contract_violating_decoder_output_is_rejected`) | 통과 |
| 미 bind·UI·Document·Render thread 거부, worker thread 성공(`decode_is_worker_only`), 역할 재 bind 거부·해제(`bind_once_and_release_on_drop`, `roles_are_per_thread`) | 통과 |
| RGBA8 순서·stride·좌상단 원점·straight 알파(`decoded_pixel_format_contract`), buffer 불일치·크기 넘침 거부(`decoded_image_rejects_inconsistent_buffers`), 입력 크기 상한(`input_size_limit_is_checked_before_probe`) | 통과 |

## 차이(작업 지시와 다른 점)

- 제품용 실제 codec은 포함하지 않았습니다(새 외부 의존성 금지, 지원 형식은 제품 결정). 손상
  corpus 검증은 테스트 전용 binary PPM 참조 decoder와 계약 위반 모사 decoder로 수행했습니다.
- thread 역할 표시(`thread_role`)는 worker 전용 강제를 위해 renderer-core 안에 정의했습니다.
  worker pool 구현(XII.1.4)이 생기면 그쪽에서 bind합니다.
- 작업 worktree 기준 commit(`784fbe7`)에는 선행 VI.8.1 코드가 없고 `develop`에만 있습니다. renderer
  계층은 document를 의존하지 않으므로 decode 입력은 asset bytes로만 정의했습니다.

## 남은 위험 / 한계

- 무 panic은 구현 계약이며, 실제 platform codec(ImageIO·WIC 등) 연동 시 같은 corpus와 fuzzing
  (아키텍처 §37.2)으로 각 구현을 다시 검증해야 합니다. process 격리는 XVI.7.4 범위입니다.
- `DecodeLimits` 기본값은 초기값입니다. 메모리 벤치마크(XVI) 후 조정이 필요합니다.
- 역할 guard를 `mem::forget`하면 해당 thread에 역할이 남습니다(안전성 문제는 아니며 정책 위반).
