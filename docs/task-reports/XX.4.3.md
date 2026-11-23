# 작업 보고서: XX.4.3 rasterized subtree 목록과 이유를 export report에 기록

- WBS: XX.4.3
- 작업명: rasterized subtree 목록과 이유를 export report에 기록
- 상태: 완료 (`[x]`)
- 선행: XX.4.2 (이 작업 묶음에서 완료).
- 결정: ADR-0091 결정 4.

## 필수 실행 환경

- `DEV-MAC` (순수 Rust).

## 변경 파일 목록

- `crates/export/src/svg/report.rs`(신규): `SvgExportReport`, `RasterizedRegion`, `RasterizedKind`, `RasterReason`,
  `BackdropReport`, `BackdropSource`, 고정 표기 `to_text`.
- `crates/export/src/svg/writer.rs`: 경계마다 report 영역을 paint 순서로 기록, `SvgDocument::report`.
- `crates/export/src/svg/mapping.rs`: BlendMode row 설명을 래스터 이유(sRGB와 선형 합성 차이)로 고침(출력 bytes 무관).
- `crates/export/src/svg/mod.rs`: 모듈 등록과 재노출.
- `crates/export/src/svg/writer/tests/report.rs`(신규), `crates/export/tests/fixtures/raster_report_v1.txt`(신규).

## 구현 요약

- report는 문서 전체의 보이는·vector·래스터 node 수와 래스터 영역 목록입니다. 영역은 종류(Content·Subtrees·Canvas),
  영역 최상위 node 목록, 렌더한 device pixel 상자(그리지 않는 영역은 없음), 래스터 node 수, 이유 목록입니다.
- 이유는 node, 래스터 기능, 값의 원천 registry CSS longhand(`SvgFeature::css_longhands`, IX.11.20), SVG로 정확히
  표현할 수 없는 까닭(mapping table 설명), blend면 isolation group과 영역을 넓힌 앞선 내용(XX.4.1)입니다.
- `to_text`는 고정 표기라 같은 snapshot이면 같은 문자열입니다. CLI·MCP가 이 값을 그대로 보여 줄 수 있습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-export --locked` | 통과 |
| `cargo clippy -p stellar-export --all-targets --locked -- -D warnings` | 경고 0 |

| 검사(테스트) | 결과 |
|---|---|
| shadow(Content)·blur 가지(Subtrees, node 2개)·blend 범위(앞 형제까지)가 있는 View의 report 고정 표기가 golden과 같고, 종류·root·pixel·node 수·이유·backdrop이 계획과 SVG image 수와 맞음(`report_lists_rasterized_subtrees_and_reasons`) | 통과 |
| 모두 vector인 View는 영역 없이 `vector = visible`(`vector_exports_report_no_rasterized_regions`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- report는 SVG 문서 결과에 붙습니다. 진단 단계(사전 진단)에서 미리 보여 주려면 shell·CLI가 `plan_raster` 결과를 같은
  타입으로 바꿔 쓰면 됩니다. 지금 CLI·MCP export 명령은 없습니다(UI 재설계와 export 명령 작업에서 연결).
