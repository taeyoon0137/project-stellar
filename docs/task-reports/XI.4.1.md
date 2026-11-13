# 작업 보고서: XI.4.1 backend 독립 RenderGraph·pass·attachment·resource lifetime 타입

- WBS: XI.4.1
- 작업명: `renderer-core`에 backend 독립 `RenderGraph`, pass, attachment, resource lifetime 타입 정의와 cycle·missing resource validator
- 상태: 완료 (`[x]`)
- 선행: XI.1.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/render_graph.rs`(신규) — `RenderGraph`, `RenderPass`, `PassKind`,
  `ColorAttachment`, `DepthStencilAttachment`, `LoadOp`, `StoreOp`, `ClearColor`, `GraphResource`,
  `TextureDesc`, `TextureFormat`, `AttachmentSize`, `SampleCount`, `ResourceLifetime`, `ImportKind`,
  `ResourceId`, `PassId`, `ResourceVersion`, `RenderGraphError`, `CompiledRenderGraph`, `LiveRange`,
  `ResourceLiveness`, `RenderGraphBuilder`, `GraphBuildError`.
- `crates/renderer-core/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XI.4.1.md`(신규), 계획서(XI.4.1 `[ ]`→`[x]`).

## 구현 요약

- **resource version 모델**: pass가 resource에 쓸 때마다 새 version(1 이상)을 생산하고, 다른 pass는
  특정 version을 읽습니다. version 0은 graph 시작 내용(imported는 외부 내용, transient는 미정의)입니다.
  pass 순서는 선언 순서가 아니라 resource 흐름의 RAW(생산 → 읽기), WAW(`v-1` 생산 → `v` 생산),
  WAR(`v-1` 읽기 → `v` 생산) 제약에서 정해집니다. `LoadOp::Load` attachment는 이전 version을 읽는
  것으로 봅니다.
- **attachment**: color(최대 8개)·depth-stencil attachment에 load(Load/Clear/DontCare)·store(Store/
  Discard)와 finite clear 값을 둡니다. Raster pass만 attachment를 갖고, Compute·Transfer는 `writes`로
  생산합니다. 한 pass의 attachment는 크기·sample 수가 같아야 하고 color/depth format이 attachment
  종류와 맞아야 합니다.
- **resource lifetime**: `Imported(Surface|External)`는 graph 밖 소유, `Transient`는 graph 안에서만
  삽니다. compile 결과는 transient마다 실행 순서상 사용 구간(`LiveRange`)과 alias slot을 줍니다.
  descriptor가 같고 구간이 겹치지 않는 transient는 같은 slot을 받습니다(처음 사용 순 greedy).
- **validator(`RenderGraph::compile`)**: 다음을 typed error로 거부합니다.
  - missing resource: 없는 resource id(`UnknownResource`), 생산자 없는 version(`MissingProducer`),
    미정의 transient 읽기(`UndefinedTransientRead`, Load로 시작하는 경우 포함).
  - cycle: Kahn 정렬이 끝나지 않으면 남은 pass에서 선행 pass를 거슬러 cycle 하나를 찾아
    의존 방향·최소 id 우선으로 회전한 경로를 돌려줍니다(`Cycle { passes }`).
  - 그 밖: version 0 쓰기, 중복 생산자, version gap, feedback loop(같은 resource 읽기+쓰기·이중 쓰기),
    Discard한 version 읽기, attachment format·크기 불일치, pass 종류와 attachment 구성 불일치,
    고정 크기 범위, output이 imported가 아니거나 최신 version이 아님.
- **결정론**: 순서 제약을 만족하는 pass 중 선언 index가 작은 pass를 먼저 둡니다. edge는 정렬된
  집합으로 모읍니다. 재귀를 쓰지 않습니다.
- **builder**: `RenderGraphBuilder`가 쓰기마다 새 version handle을 돌려주고, 최신이 아닌 handle에
  쓰면(분기 쓰기) `StaleVersion`으로 즉시 거부합니다. 최종 검증은 compile이 합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core` | 45 passed, 0 failed; doctest 1 passed(compile_fail) |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | exit 0 |

| 검사 | 결과 |
|---|---|
| 2-pass·3-pass(하류 pass 포함)·Load 경유 cycle을 `Cycle`로 거부하고 경로가 결정론적(`cycles_are_rejected_with_deterministic_path`) | 통과 |
| 없는 resource id(read·attachment), 생산자 없는 version, 미정의 transient 읽기·Load, output 생산자 없음을 거부하고 imported v0 Load는 허용(`missing_resources_are_rejected`) | 통과 |
| 중복 생산자, version gap, v0 쓰기, feedback loop, Discard 읽기, format·크기 불일치, attachment 구성, 0 크기, output 규칙(`structural_rules_are_enforced`) | 통과 |
| 선언 순서와 무관한 resource 흐름 순서(offscreen → opaque → transparent)와 수명(`dependencies_follow_resource_flow_not_declaration_order`) | 통과 |
| WAR: 뒤에 선언된 reader가 덮어쓰기 pass보다 먼저 실행(`write_after_read_orders_readers_before_overwrite`) | 통과 |
| 겹치지 않는 같은 descriptor transient의 slot 공유, format이 다르면 새 slot, 미사용 transient `Unused`, builder stale handle 거부(`non_overlapping_transients_share_alias_slots`, `overlapping_transients_get_distinct_slots`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 XI.1.1은 명시 row-ID이고 `[x]`라 완화 적용은 없습니다.
- graph resource는 texture만 둡니다. frame-local buffer(uniform·vertex)는 XI.5.3 frame allocator가
  담당하고 graph에는 넣지 않았습니다.
- 출력에 기여하지 않는 pass 제거(culling)는 하지 않습니다. external texture 쓰기 같은 부수 효과를
  잘못 지울 수 있어 compiler(XI.4.2 이후)가 필요한 pass만 만들도록 했습니다.

## 남은 위험 / 한계

- alias slot은 descriptor가 완전히 같은 transient끼리만 공유합니다. 크기가 다른 texture의 memory
  heap aliasing은 backend(XI.8)가 판단합니다.
- pass 안의 draw 내용은 graph에 없습니다. pass별 draw 목록은 compiler(XI.4.2)가 `PassId` index로
  따로 둡니다.
