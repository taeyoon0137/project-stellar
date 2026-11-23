# 작업 보고서: XI.7.4 missing/decoding asset placeholder

- WBS: XI.7.4
- 작업명: missing/decoding asset placeholder를 결정론적 checker로 정의하고 양 backend가 같은 data 사용
- 상태: `[~]` 유지 — 판정 보류(2026-11-23 재판정, 아래 "재판정" 절). 이전 기록: 부분 완료 (`[~]`) — ADR-0006 4번 규칙에 따라 GPU 출력에 의존하는 "양 backend screenshot 동일"은 Metal backend(XI.8) screenshot fixture가 생길 때까지 확정하지 않습니다.
- 선행: VI.8.2 (`develop`에서 완료: `339f981`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증. GPU backend 실행 없음).

## 변경 파일 목록

- `crates/renderer-core/src/placeholder.rs`(신규) — `PlaceholderKind`, `PlaceholderColors`,
  `ImageAvailability`, `PlaceholderTexture`/`placeholder_texture`(컴파일 시점 생성 static tile),
  `SamplerDesc`·`TextureFilter`·`TextureWrap`·`PLACEHOLDER_SAMPLER`, `PlaceholderQuad`/
  `placeholder_quad`, CPU 기준 screenshot `rasterize_placeholder_reference`, `PlaceholderError`.
- `crates/renderer-core/src/lib.rs` — 모듈 등록.
- `crates/scene/src/lib.rs` — renderer-core용 `Size2` 재노출.
- `docs/task-reports/XI.7.4.md`(신규), 계획서(XI.7.4 `[ ]`→`[x]`).

## 구현 요약

- **placeholder 정의**: 16×16 px tile, 8 px cell 2×2 checker(`x/8 + y/8` 짝수 = light, 홀수 = dark),
  XI.7.1 decoded format(RGBA8, sRGB, 불투명). Missing은 `#CCCCCC`/`#999999`, Decoding은
  `#F0F0F0`/`#E0E0E0` neutral gray입니다. decode 실패(`DecodeFailed`)는 Missing으로, `Ready`는
  placeholder 없음으로 매핑합니다.
- **양 backend 동일 data**: tile pixel은 `const fn`으로 컴파일 시점에 생성한 `static`이며
  `placeholder_texture(kind)`가 같은 `&'static` 데이터를 반환합니다. sampler(Nearest + Repeat,
  mipmap 없음)도 공통 상수입니다. Metal·D3D12는 자체 placeholder를 만들지 않고 이것을 upload합니다.
- **배치**: checker는 node local 원점에 고정, cell 한 변 local 8 단위. quad는 layout bounds 크기를
  그대로 쓰고(VI.8.2 bounds 유지) uv = 크기 / 16입니다. fit·crop은 적용하지 않습니다.
- **기준 screenshot**: `rasterize_placeholder_reference`가 1:1 배율에서 pixel 중심 uv를
  `PlaceholderTexture::sample`(GPU와 같은 Nearest + Repeat 규칙)로 sampling한 RGBA8 이미지를
  만듭니다. 크기 0·상한 초과는 오류입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core` | 24 passed, 0 failed; doctest 1 passed |
| `cargo test -p stellar-scene` | 2 passed, 0 failed |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo clippy -p stellar-scene --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 통과 |
| `cargo fmt -p stellar-scene -- --check` | 통과 |
| `cargo xtask validate-crate-graph` | exit 0 |

| 검사 | 결과 |
|---|---|
| 20×12 pixel ASCII fixture·40×24 cell map fixture와 기준 screenshot byte 일치(색은 fixture literal, 구현과 독립), 4개 FNV-1a golden digest 일치(`reference_screenshot_matches_fixture`) | 통과 |
| "metal"·"d3d12" 이름의 별도 thread에서 요청한 texture가 같은 static 주소·tile bytes·sampler·기준 screenshot(`both_backends_share_identical_placeholder_data`) | 통과 |
| tile 16×16·RGBA8·sRGB·불투명·cell parity, Missing≠Decoding(`tile_texture_contract`) | 통과 |
| Repeat sampling(정수 tile 이동·음수 좌표), quad uv = bounds/16(`repeat_sampling_and_quad_uv`) | 통과 |
| 상태→placeholder 매핑(`availability_maps_to_placeholder`), 결정론·크기 0/상한 오류(`reference_rasterization_is_deterministic_and_bounded`) | 통과 |

## 차이(작업 지시와 다른 점)

- GPU backend(Metal·D3D12)로 실제 screenshot을 찍지 않았습니다. backend crate는 이 작업 범위 밖이며
  GPU 실행 환경이 필요합니다. 대신 양 backend가 쓰도록 강제되는 공유 data(같은 static tile·sampler)와
  CPU 기준 screenshot을 fixture로 고정했습니다.
- golden digest는 구현 출력에서 한 번 생성해 고정했습니다. 값 자체의 정확성은 구현과 독립인 ASCII
  fixture 비교가 보장합니다.
- 작업 worktree 기준 commit(`784fbe7`)에는 선행 VI.8.2 코드가 없고 `develop`에만 있습니다. renderer
  쪽 상태는 `ImageAvailability`로 독립 정의했습니다.

## 남은 위험 / 한계

- 실제 backend screenshot이 기준 screenshot과 일치하는지는 Metal(XI.8)·D3D12(XI.9) backend와 렌더
  적합성 fixture(XI.11.3)에서 확인해야 합니다. 비정수 배율·회전 transform에서는 rasterization
  규칙 차이로 경계 pixel 허용 오차 정책이 필요합니다(아키텍처 §10.2).
- 기준 screenshot은 sRGB 인코딩 값 그대로입니다. 선형 합성 경로(blend·opacity)와 결합된 결과는
  후속 적합성 테스트 범위입니다.
- 상태 정정(통합 리뷰): placeholder data와 CPU 기준 rasterization이 결정론적이라는 것만 증명했습니다. Metal screenshot fixture 비교는 XI.8 이후 추가하고, Direct3D 12 비교는 ADR-0006에 따라 보류합니다.

## 재판정 (2026-11-23, ledger-verification)

- 판정: `[~]` 유지 — 판정 보류.
- 근거: 판정 보류(GPU 출력). placeholder data와 CPU 기준 rasterization의 결정론은 코드로 확인했지만, 검증 열 "screenshot fixture 동일"은 GPU backend 출력이라 ADR-0006 결정 4에 따라 `[~]`입니다.
- 판정 기준: 행 지시·검증 열의 불변식을 구현 코드에서 확인했고, 테스트 통과는 보조 근거로만 썼습니다. 계획서 §1.2 규칙 4(선행이 모두 `[x]`여야 완료)를 적용했습니다. 기준 커밋은 `main` `ce60bb92`입니다.

### 확인한 코드 위치

- `crates/renderer-core/src/placeholder.rs`: 컴파일 시점 static tile, 공통 sampler, `rasterize_placeholder_reference`.
- `crates/renderer-metal/src/tests/`에서 placeholder texture를 씁니다. Metal offscreen screenshot을 기준 이미지와 비교하는 fixture는 없습니다.

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test --workspace`(DEV-MAC, 샌드박스 해제) | 테스트 binary 142개 모두 ok, 4136 passed, 0 failed |
| `cargo test -p stellar-renderer-core` | 156 passed |
| `cargo test -p stellar-renderer-metal` | 54 passed(실제 GPU) |

### 남은 조건과 finding

- metal-offscreen-render lane이 Metal offscreen readback과 `rasterize_placeholder_reference` 비교 fixture를 더하면 macOS 부분을 다시 판정합니다. Direct3D 12 비교는 ADR-0006으로 보류입니다.
