# 작업 보고서: XI.7.2 mipmap 생성과 texture upload task

- WBS: XI.7.2
- 작업명: decode된 이미지의 mipmap chain 생성과 GPU texture upload task 구현
- 상태: 완료 (`[x]`)
- 선행: XI.7.1 (`[x]`), XI.5.3 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/mipmap.rs`(신규) — `mip_level_count`, `mip_level_size`, `MipLevel`, `MipChain`,
  `generate_mip_chain`(worker scope 인자), `generate_mip_chain_on_worker`, `TextureUploadTask`, `UploadSummary`,
  `MipmapError`, `TextureUploadError`, 고정 전달 함수 표.
- `crates/renderer-core/src/mipmap/tests.rs`(신규) — 크기별 mip 수·filter·결정론·upload 테스트.
- `crates/renderer-core/src/lib.rs` — `mipmap` 모듈 등록.
- `docs/task-reports/XI.7.2.md`(신규), 계획서(XI.7.2 `[ ]`→`[x]`).

## 구현 요약

- **level 수**: `MipmapMode::Full`은 `floor(log2(max(w, h))) + 1`(정수 `leading_zeros`), level `i` 크기는 각 변
  `max(1, 변 >> i)`. `MipmapMode::None`은 base level 하나입니다(XI.7.3 texture key의 decode setting과 일치).
- **pixel 규약**: 출력은 `Rgba8UnormSrgb` texture용으로, RGB는 선형광에서 premultiply한 값을 원본 색공간의 transfer
  곡선(ADR-0002)으로 인코딩한 8-bit, 알파는 선형 8-bit입니다. GPU sampling이 선형 premultiplied 값을 내므로 filtering이
  선형 공간에서 일어납니다. gamut 변환은 하지 않고 chain이 원본 `ColorSpace`를 담습니다(XI.7.1 decode 문서의 "premultiply는
  XI.7.2" 규약 이행).
- **filter**: 2×2 box 평균, 홀수 변 마지막 pixel은 3개 평균(원본 pixel 버리지 않음). 선형 premultiplied 16-bit working
  buffer에서 평균하고 level마다 재양자화하지 않습니다. 투명 pixel의 색은 premultiply로 번지지 않습니다.
- **결정론**: 전달 함수는 IEC 61966-2-1 곡선을 배정밀도로 계산해 반올림한 256-entry 정수 표(decode)와 이웃 중간값 이진
  탐색(encode)입니다. 실행 중 `powf`가 없어 출력 bytes가 정수 연산만으로 정해집니다.
- **upload task**: `TextureUploadTask::new(&chain)`이 descriptor(level 수, sampled)와 level별 영역을 만들고,
  `check_device`가 device 최대 크기를 검사하며, `enqueue`는 모든 level 요청을 검증한 뒤에만 `UploadQueue`에 base level부터
  넣고(부분 upload 없음), `start`는 texture 생성 + enqueue이며 요청이 거부되면 만든 texture를 바로 파괴합니다. 실제
  전송은 XI.5.3 `UploadQueue::flush` 예산으로 여러 frame에 나뉩니다.
- **worker 전용**: 생성은 `DecodeWorkerScope`를 요구하며, `generate_mip_chain_on_worker`는 worker가 아니면
  `NotOnWorkerThread`입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-renderer-core` | 종료 코드 0 |
| `cargo test -p stellar-renderer-core` | 137 unit passed(신규 6) + 3 doctest passed, 0 failed |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| 크기별 expected mip count 12경우(1×1=1, 2×2=2, 3×5=3, 17×9=5, 256×1=9, 255²=8, 1000×1=10, 1024×768=11, 1025×3=11, 16384²=15 등), 마지막 level 1×1, 실제 생성 chain level 수·크기·길이 일치, `None`은 1 level(`expected_mip_count_by_size`) | 통과 |
| 표가 core-types `decode_channel`과 1/65535 + 1e-5 이내, 256 code encode∘decode round-trip, 엄격 증가(`transfer_table_matches_core_curve_and_round_trips`) | 통과 |
| 검정+흰색 → 188(선형 평균), 빨강+투명 초록 → `[188,0,0,128]`(번짐 없음), 홀수 3-tap, 반투명 base premultiply(`filtering_is_premultiplied_linear_box`) | 통과 |
| 단색 37×21 모든 level·pixel 보존, 잡음 33×17 chain golden digest `0x7f06_c56d_80ad_42aa`(`uniform_color_is_exact_at_every_level_and_output_is_deterministic`) | 통과 |
| 비 worker thread 거부, worker에서 생성(`generation_requires_worker_thread`) | 통과 |
| descriptor·영역(mip 0/1/2 = 5×3, 2×1, 1×1)·bytes, mock backend texture 생성 + 예산 flush로 2 frame에 3개 `UpdateTexture`, device 최대 크기 초과 오류(`upload_task_creates_texture_and_uploads_every_level`) | 통과 |

## 차이(작업 지시와 다른 점)

- 색공간(gamut) 변환은 mip 생성에서 하지 않습니다. texture cache key의 `ColorTransform`(XI.7.3) 적용은 공통 색 변환 계층
  (XI.10.1)에서 정의합니다.

## 남은 위험 / 한계

- 2×2 box filter는 고주파 이미지에서 aliasing이 남을 수 있습니다(Lanczos 등은 품질 benchmark 후 판단).
- 큰 이미지(16384²)의 chain 메모리는 base의 약 4/3배입니다. 메모리 예산·화면 밖 mip eviction은 XVI.5 범위입니다.
- 결과는 정수 연산만으로 정해져 ADR-0006 판정 규칙상 플랫폼 간 동일 구성입니다(golden digest 고정).
