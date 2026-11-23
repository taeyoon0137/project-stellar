# 작업 보고서: XXII.1.2 IR node별 mapping 진단 보존

## WBS

- WBS: XXII.1.2
- 작업명: unsupported/approximated/native mapping diagnostic을 IR node별로 보존
- 상태: 완료 보고(`reported_done`). 선행 XXII.1.1이 `[!]`이라 원장 판정은 통합 때 정합니다.
- 결정: ADR-0106 결정 2·3, ADR-0025 결정 1.3
- 선행: XXII.1.1

## 변경 파일

- `crates/codegen/src/ir/mapping.rs`(신규): `CssPropertyUse`, `IrMapping`, `MappingSource`, `MappingSubject`, validator 진단과
  CSS 기반 속성 사용을 mapping으로 바꾸는 함수, 정렬·중복 제거.
- `crates/codegen/src/matrix/mod.rs`(신규): 코드 생성 matrix 진입점 `css_entry`. Web은 IX.11.19 `web_capability` 결과를
  규칙·진단으로 옮깁니다. Apple·Android는 XXII.1.4가 채웁니다(이 커밋에서는 미등록).
- `crates/codegen/src/ir/{mod,build,snapshot,tests}.rs`, `crates/codegen/src/fixture.rs`, `crates/codegen/src/lib.rs`.
- `crates/codegen/tests/fixtures/{ir_site,ir_ios,ir_android}.snapshot`(v3), `mappings_{site,ios,android}.txt`(신규).

## 구현 요약

- `CodegenSources::css_properties`(node → 그 node가 쓰는 registry 항목)를 새 입력으로 받습니다.
- `IrNode::mappings`에 validator 진단(element·contract·capability 속성)과 matrix 판정(CSS 기반 속성)을 Native까지 모두
  담습니다. 순서는 (출처, subject, path) 오름차순이고 같은 항목은 하나만 남깁니다.
- profile에서 제외한 node의 사용은 버리고, View에 없는 node를 가리키면 `CssPropertyOutsideView`로 거부합니다.
- semantic snapshot 머리를 `codegen-ir v3`로 올리고 `capability` 줄을 `mapping` 줄(출처, subject, 상태, 규칙, 진단,
  대체, path)로 바꿨습니다.

## 명령과 결과

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-codegen` | 통과(57) |
| `cargo clippy -p stellar-codegen --all-targets -- -D warnings` | 경고 0 |

시험:

- `ir::tests::mapping_diagnostics_are_kept_per_node_including_native`: Web·iOS·Android 진단 fixture golden, Native·
  Approximated·Unsupported 판정, 정렬, 제외 node 진단 없음.
- `ir::tests::mapping_diagnostics_are_deterministic_and_reject_unknown_nodes`: 입력 순서·중복과 무관한 결과, View 밖 node 거부.

## 남은 위험

- 문서에서 `css_properties`를 채우는 연결은 XXII.1.1 재개 범위입니다. 지금은 fixture 입력으로만 검증했습니다.
