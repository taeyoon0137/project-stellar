# 작업 보고서: XX.2.3 PNG encoding worker와 temp+atomic rename 저장

- WBS: XX.2.3
- 작업명: PNG encoding을 worker에서 실행하고 temp+atomic rename으로 저장
- 상태: 구현 완료 (`[x]` 후보, marker는 통합 때 정합니다) — worker 실행·취소·atomic 저장과 cancel/failure 검사(이전
  작업)에 더해, 남은 범위였던 CLI `stellar export png`와 MCP `document.export_png`를 연결했습니다. 선행 XX.2.1은 이
  lane에서 GPU target까지 구현했습니다(XX.2.1 보고서).
- 선행: V.2.5 (`[x]`), XX.2.1 (이 lane에서 구현, marker는 통합 때 판정).
- 결정: ADR-0103 결정 6·8, ADR-0116 결정 1·2·5.

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, CLI·MCP 실행 파일 통합 test).

## 변경 파일 목록

이전 작업(worker·저장):

- `crates/export/src/png/job.rs`, `crates/export/src/png/tests/job.rs`, `crates/ops/src/export_png.rs`,
  `crates/ops/src/export_png/tests.rs`.

이 lane(CLI·MCP 연결):

- `crates/scene/src/document.rs`, `crates/scene/src/document/{inputs,effects,tests}.rs`(runtime canvas에서 옮김 + 효과 평가),
  `crates/scene/Cargo.toml`, `crates/scene/src/lib.rs`.
- `crates/runtime/src/canvas.rs`, `crates/runtime/src/canvas/{shape,image}.rs`, `crates/runtime/Cargo.toml`(옮긴 함수를
  scene crate에서 씀).
- `crates/ops/src/export_png/view.rs`(신규, `prepare_view_png`·`export_view_png`·`render_view_png`),
  `crates/ops/src/export_png/view/tests.rs`(신규), `crates/ops/src/export_png/fixture.rs`(신규, test-support),
  `crates/ops/src/export_png.rs`, `crates/ops/src/test_support.rs`(fixture 재노출 한 줄), `crates/ops/Cargo.toml`.
- `tools/stellar-cli/src/export_png.rs`(신규), `tools/stellar-cli/src/export_native.rs`(`export png` 분기),
  `tools/stellar-cli/src/{cli,lib}.rs`, `tools/stellar-cli/tests/fixtures/help.txt`, `tools/stellar-cli/tests/export_png.rs`(신규).
- `crates/agent-protocol/src/catalog/tools.rs`(`EXPORT_PNG`), `crates/mcp-core/src/headless.rs`,
  `crates/mcp-core/src/headless/operations.rs`, `tools/stellar-mcp/tests/{headless_tools,headless_read}.rs`,
  `tools/stellar-mcp/tests/fixtures/{catalog.json,trust-boundary.jsonl}`(tool 목록 golden 갱신).
- `tools/xtask/src/crate_graph.rs`, `docs/architecture/crate-dependency-graph.md`(scene → document·commands(feature), ops → assets).
- `docs/adr/ADR-0116-render-effect-passes-and-png-export-paths.md`(신규).

## 구현 요약

- **scene 변환 이동**: 저장 문서 → scene source 변환(ADR-0084)을 `stellar_scene::document`(feature `document`)로 옮겨
  앱 runtime과 headless 도구가 같은 함수를 씁니다. renderer는 feature를 켜지 않습니다.
- **효과 modifier**: Shadow·Blur·Blend·Opacity modifier를 scene 합성 효과로 평가합니다(ADR-0116 결정 2). 내용 modifier 규칙은
  그대로입니다.
- **ops**: 저장 View → scene snapshot → `ExportCapture` → 사전 진단 → `ConfirmedExport` → CPU target. 빠진 node와 확인이
  필요한 진단은 기본 거부이고 호출자가 확인 인자로 진행합니다. 이미지는 generation의 asset metadata와 content store에서 PNG를
  읽고, PNG가 아닌 asset은 렌더 전에 거부합니다.
- **CLI** `stellar export png <경로> <문서 id> <View id> --output <파일> [--scale] [--profile] [--background] [--accept-issues]
  [--omit-skipped]`: worker 렌더 후 temp + atomic rename 저장, 결과 JSON(경로·byte 수·크기·profile·gamut·빠진 node·확인한
  진단). 실패 code는 `png-skipped-nodes`·`png-confirmation-required`·`png-unsupported`(그리지 못하는 효과, 종료 1)·`png-too-large`·
  `png-write-failed`(종료 73) 등입니다.
- **MCP** `document.export_png`(headless 읽기, 파일을 쓰지 않음): 같은 ops 함수로 렌더한 PNG를 base64로 돌려줍니다. 512 KiB를
  넘으면 `too-large`입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-scene --lib document` | 21 passed |
| `cargo test -p stellar-ops --lib` | 145 passed |
| `cargo test -p stellar-cli --no-fail-fast` | 실패 0(새 `tests/export_png.rs` 3개, help snapshot 포함) |
| `cargo test -p stellar-mcp --no-fail-fast` | 실패 0(tool 목록 golden 두 개를 `STELLAR_MCP_BLESS=1`로 갱신, 차이는 새 tool 항목뿐) |
| `cargo test -p stellar-runtime --lib canvas` | 107 passed |
| `cargo xtask validate-crate-graph` | 통과 |
| `cargo clippy -p stellar-ops -p stellar-cli -p stellar-mcp-core -p stellar-mcp -p stellar-agent-protocol -p stellar-scene --all-targets -- -D warnings` | 경고 0 |

| 검사(테스트) | 결과 |
|---|---|
| 저장 View를 2배로 내보내면 80×40, 불투명 빨강 pixel (255,0,0,255), 불투명도 0.5 파랑 Frame은 선형광 반 섞임 (188,0,188,255), 파일 없는 렌더 bytes가 저장 bytes와 같음(`saved_view_exports_to_png_with_scene_conversion`) | 통과 |
| 배경·display-p3 profile, 잘못된 배경 색·profile은 요청 오류(`background_and_profile_follow_request`) | 통과 |
| Text가 있는 View는 확인 없이 거부·파일 없음, `omit_skipped`면 저장하고 빠진 node 보고(`skipped_nodes_need_confirmation`) | 통과 |
| Shadow modifier는 CPU target 미지원 오류·파일 없음(`unsupported_cpu_effects_fail_without_writing`) | 통과 |
| 저장 generation 없음·없는 View·상대 출력 경로 오류(`missing_inputs_are_reported`) | 통과 |
| CLI: 결과 JSON 크기·profile·경로·byte 수, PNG signature, 같은 입력 같은 bytes, 프로젝트 파일 불변, display-p3·배경(`saved_view_exports_to_png_file`) | 통과 |
| CLI: 빠진 node 종료 1·`png-skipped-nodes`·파일 없음, `--omit-skipped` 저장, shadow 종료 73·파일 없음(`skipped_nodes_and_unsupported_effects_fail_without_writing`) | 통과 |
| MCP: 80×40 PNG base64, 실행 전후 파일 hash 같음, 빠진 node는 `rejected`(`export_png_renders_saved_view`), headless 목록 11개 읽기 전용(`every_listed_tool_runs_read_only_through_ops`) | 통과 |
| 효과 modifier 평가: Opacity 곱·Shadow·Blend·Blur가 scene paint로, Frame 효과만 허용, 반복 Blur·Frame Fill modifier skip(`effect_modifiers_become_scene_compositing_effects`, `repeated_single_value_effects_and_non_effect_frame_modifiers_are_skipped`) | 통과 |

## 차이(작업 지시와 다른 점)

- 저장(파일 시스템 정책)은 storage를 쓰는 ops에 두고, export crate는 bytes까지 만듭니다(같은 atomic 쓰기 구현을 따로 두지
  않음).
- MCP tool은 파일을 쓰지 않습니다(headless backend는 읽기 전용, ADR-0005 §3). 파일 저장은 CLI가 합니다.
- headless에는 Text layout hook이 없어 Text는 빠진 node입니다(확인하면 빼고 그림).

## 남은 위험 / 한계

- CPU target은 drop shadow·layer blur·Normal이 아닌 blend를 아직 그리지 않습니다(ADR-0103 결정 2). 이 효과가 있는 View는
  CLI·MCP에서 실패합니다. CPU target을 renderer CPU 기준(`effects::raster`) 공식으로 넓히는 일은 export raster 소유 작업입니다.
- headless Text layout(폰트 해석·shaping)이 없습니다. 저장 font asset으로 Text를 layout하는 hook이 생기면 같은 변환 option에
  넣습니다.
- 앱 export 명령·진행 표시(XX.5.1)는 UI 재설계 때 연결합니다.
- gamut 요약은 scene paint stack의 단색만 셉니다(이전과 같음).
