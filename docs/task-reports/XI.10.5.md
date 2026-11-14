# 작업 보고서: XI.10.5 Display P3 surface capability 감지와 sRGB fallback·gamut 경고

- WBS: XI.10.5
- 작업명: Display P3 surface 지원 여부를 platform capability로 감지하고, 미지원 display에서는 sRGB 변환과 gamut 경고를 생성
- 상태: 완료 (`[x]`)
- 선행: III.4.2 (`[x]`), XI.5.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/output_color.rs`(신규) — `OutputColorPlan`(`negotiate`, `surface_config`, `convert`, `scan`,
  `warnings`), `OutputColor`, `SurfaceColor`, `GamutStatus`, `OutputColorWarning`(정적 code), `GamutReport`,
  `GAMUT_EPSILON`, `MAX_REPORTED_NODES`.
- `crates/renderer-core/src/output_color/tests.rs`(신규) — mock capability fixture와 변환·경고 테스트.
- `crates/renderer-core/src/test_scene.rs` — 테스트 fixture에 Display P3 색 기록(`in_display_p3`)과 shadow 색 도우미 추가
  (`#[cfg(test)]` 전용).
- `crates/renderer-core/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XI.10.5.md`(신규), 계획서(XI.10.5 `[ ]`→`[x]`).

## 구현 요약

- **capability 감지**: backend `init`이 돌려준 `DeviceCapabilities::display_p3_surface`(XI.5.1)를 씁니다.
  `negotiate(requested, caps)`는 P3 요청 + 지원이면 P3 surface, P3 요청 + 미지원이면 sRGB surface와
  `DisplayP3Unsupported` 경고, sRGB 요청이면 항상 sRGB입니다. `surface_config`가 결정된 색공간을 surface 설정에 넣습니다.
- **변환**: transfer decode(ADR-0002 공통 곡선) → D65 primaries 행렬(P3↔sRGB, VIII.4.4와 같은 상수) → surface 색공간
  encode. surface 범위 `[−ε, 1+ε]`(ε = 1e-5) 밖이면 잘라 표시하되 `GamutStatus::Clipped`로 표시합니다(조용히 자르지
  않음). 같은 색공간은 transfer round-trip만 합니다.
- **gamut 경고**: `scan(snapshot)`이 모든 node의 fill·stroke·shadow 단색을 변환해 잘린 색 수·node 수와 node 안정 ID(앞에서
  최대 64개, pre-order)를 모읍니다. `warnings`는 협상 경고 + `GamutClipped`를 돌려줍니다. 경고에는 정적 code와 개수·ID만
  있습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-renderer-core` | 종료 코드 0 |
| `cargo test -p stellar-renderer-core` | 142 unit passed(신규 5) + 3 doctest passed, 0 failed |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| mock backend capability `display_p3_surface` true/false로 init한 caps: P3 요청 → P3/sRGB fallback + `DisplayP3Unsupported`(code 고정), sRGB 요청은 경고 없음, surface 설정 반영(`mock_capability_selects_surface_color_space`) | 통과 |
| sRGB surface의 P3 순빨강 `Clipped`(잘린 값 1,0,0), 무채색 경계 반올림 비경고, 저채도 P3 색 배정밀도 reference 1e-3 이내, 알파 보존(`p3_on_srgb_surface_converts_and_flags_out_of_gamut`) | 통과 |
| P3 surface의 sRGB 순빨강 = (0.9175, 0.2003, 0.1386) 1e-3 이내, 무작위 sRGB 500색 모두 `InGamut`, 같은 색공간 1e-5 이내(`srgb_on_p3_surface_is_always_in_gamut`) | 통과 |
| scene scan: sRGB node 비경고, P3 fill·stroke·shadow 잘린 색 3개·node 2개·ID 순서, 지원 display에서는 경고 없음(`snapshot_scan_reports_clipped_nodes`) | 통과 |
| 잘린 node 100개 중 ID는 64개만 기록하고 개수는 모두 셈(`reported_node_ids_are_bounded`) | 통과 |

## 차이(작업 지시와 다른 점)

- 실제 Metal capability(연결된 display의 EDR/P3 지원, `CAMetalLayer.colorspace`)는 Metal backend 작업(XI.8.1)에서
  `display_p3_surface`를 채웁니다. 이 작업은 공통 계층 판정과 mock fixture까지입니다.
- 이미지 pixel의 gamut 검사는 하지 않습니다(텍스처는 GPU 변환 경로, XI.10.1).

## 남은 위험 / 한계

- 잘라내기(clip)만 하고 perceptual gamut mapping은 하지 않습니다. 디자인 도구 관례(표시만 근사, 저장 값 보존)와 같지만 향후
  Target Profile 정책에서 재검토할 수 있습니다.
- transfer decode·encode는 core-types `powf`(libm)를 씁니다. 판정은 허용 오차 기반이라 macOS 결과로 완료 처리했지만,
  Windows 재개 시 경계 색 판정 fixture를 다시 확인해야 합니다(ADR-0006 규칙 4).
- 행렬 상수가 expressions crate와 중복입니다(VIII.4.4 보고서의 공용화 후속과 같은 문제).
