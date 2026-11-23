# 작업 보고서: XX.1.3 내보내기 전 capability·missing resource 진단과 사용자 확인

- WBS: XX.1.3
- 작업명: unsupported target capability와 missing resource를 export 전 진단하고 user confirmation 없이는 진행하지 않음
- 상태: 완료 (`[x]`)
- 선행: IX.10.8 (`[x]`), XIII.6.7 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/export/src/preflight.rs`(신규) — `ExportPreflight`(`run`, `issues`, `notes`, `blockers`, `confirm`, `proceed`),
  `PreflightInput`, `PreflightIssue`, `FormatBlocker`, `ExportConfirmation`, `ConfirmedExport`, `PreflightRejection`,
  `PreflightError`, host 입력 값 타입(`TargetCapabilityReport`, `TargetCapabilityFinding`, `CapabilityCode`,
  `CapabilityClass`, `CapabilitySubjectRef`, `FontFallbackUse`, `FontSubstitution`, `LineBreakImpact`).
- `crates/export/src/preflight/tests.rs`(신규) — 진단 fixture와 확인·진행 테스트.
- `crates/export/src/svg/writer.rs` — `write_svg`가 `&ConfirmedExport`만 받고, 확인한 missing image·text layout·font만
  생략(확인하지 않은 resource가 없으면 오류).
- `crates/export/src/svg/writer/tests.rs`, `crates/export/src/svg/writer/tests/text.rs` — writer 동작 테스트가 crate 안
  테스트 전용 진행 허가(`ConfirmedExport::assume_confirmed`, `#[cfg(test)]`)를 쓰도록 변경.
- `crates/export/src/lib.rs` — `preflight` 모듈과 재노출.
- `docs/task-reports/XX.1.3.md`(신규). 계획서 marker는 통합 때 생성합니다.

## 구현 요약

- **진단 범위**: capture 범위(View 전체·selection subtree)의 보이는 scene node만 봅니다. 숨긴 node, selection 밖, scene
  node가 없는 document node의 진단은 결과에 영향이 없어 제외합니다. resource는 capture의 고정 revision으로만 확인합니다.
- **확인이 필요한 진단**(`issues`, 고정 순서):
  1. 요청 target profile을 검사하지 못함(pin한 registry release 미지, IX.10.8 `RegistryUnavailable`).
  2. Unsupported·NotRegistered capability(IX.10.8 Error 심각도, 입력 순서 유지).
  3. 고정 asset revision에 없는 이미지(asset 순, 사용 node 목록).
  4. 고정 font revision에 없는 text layout(key 순).
  5. 고정 font revision에 없는 폰트(face마다 glyph 하나로 확인, face 순).
  6. 명시 fallback chain으로 대체한 폰트(XIII.6.7, 줄바꿈 판정 `MayChange`·`Changed` 포함, Text id 순, 중복 제거).
- **참고 진단**(`notes`): Approximated·Deprecated capability. 확인 없이 진행합니다. Native는 기록하지 않습니다.
- **형식 차단**(`blockers`): SVG 요청에서 mapping table(XX.3.1)상 아직 기록할 수 없는 기능(shadow·blur·blend 등).
  확인해도 진행하지 않습니다.
- **확인과 진행**: `confirm()`은 사용자가 본 진단 목록과 capture 정체(scene semantic hash, resource pin, 요청)를 기록합니다.
  `proceed`는 차단이 있으면 `Blocked`, 확인이 필요한데 없으면 `ConfirmationRequired`, 다른 preflight의 확인이면
  `ConfirmationMismatch`이고, 통과하면 `ConfirmedExport`를 줍니다. SVG writer는 `ConfirmedExport`만 받으므로 진단과 확인을
  건너뛸 방법이 없습니다.
- **확인한 missing resource**: writer는 확인한 missing image의 pixel, missing layout Text의 glyph, missing font의 glyph만
  생략합니다. 진단 때 있던 resource가 쓰는 중에 없으면 생략하지 않고 `MissingImage`·`MissingTextLayout` 오류입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-export` | 91 passed, 0 failed (이 작업 신규 8) |
| `cargo fmt -p stellar-export -- --check` | 통과 |
| `cargo clippy -p stellar-export --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| **diagnostic fixture**: Frame 아래 Shape·Image 2(1개는 고정 revision에 없음)·Text 2(1개는 layout 없음, 1개는 없는 폰트 run 포함)·숨긴 Text·숨긴 Image, capability 진단 7개(Native, Unsupported, NotRegistered, Approximated, Deprecated, 숨긴 node, scene 밖 node), 폰트 대체 4개(숨긴 Text, 중복 포함) → 진단이 정확히 7개(capability 2, 이미지 1, layout 1, 폰트 1, 대체 2)·참고 2개·차단 0, 입력 순서를 바꿔도 같은 진단(`diagnostic_fixture_reports_expected_issues`) | 통과 |
| **확인 없이 진행하지 않음**: 확인 없이 `ConfirmationRequired`(진단 7건), 확인하면 진행하고 생략 대상이 확인한 resource와 같음(`export_does_not_proceed_without_confirmation`) | 통과 |
| 다른 진단 목록의 확인·scene 내용이 다른 capture의 확인은 `ConfirmationMismatch`, 같은 내용으로 다시 진단한 확인은 허용(`confirmation_of_another_preflight_is_rejected`) | 통과 |
| 문제 없는 selection은 확인 없이 진행, Deprecated 참고만 있어도 같음(`clean_or_note_only_exports_proceed_directly`) | 통과 |
| 검사하지 못한 profile은 확인 필요, 다른 profile의 검사 결과는 `ProfileMismatch`(`unvalidated_profile_requires_confirmation_and_profile_must_match`) | 통과 |
| SVG로 기록할 수 없는 drop shadow는 확인해도 `Blocked`, PNG 요청은 차단 없음(`unsupported_output_features_block_even_with_confirmation`) | 통과 |
| 확인한 missing image는 pixel 없이 그룹만, 있는 이미지는 그대로, missing layout Text는 glyph 없이, missing font glyph만 생략(`confirmed_missing_resources_are_omitted_and_others_are_kept`) | 통과 |
| 진단 뒤 사라진 이미지·layout은 생략하지 않고 오류(`resources_lost_after_the_preflight_are_errors`) | 통과 |
| 기존 writer·text·PNG 테스트 | 통과 |

## 차이(작업 지시와 다른 점)

- export crate는 crate 의존 정책(ADR-0019)상 target-profile·project crate에 의존할 수 없어, IX.10.8 capability 진단과
  XIII.6.7 폰트 대체 결과를 같은 의미의 값(번호·식별자만)으로 host가 옮겨 넘기게 했습니다. 실제 변환과 확인 UI는 export
  command(XX.5.1)의 범위입니다.
- "user confirmation"은 코드가 사람을 확인할 수 없으므로, 확인 UI가 승인 뒤 부르는 `confirm()` 기록과 그 기록이 같은
  진단·같은 capture에만 유효하다는 검사로 구현했습니다.
- 진단 범위에 SVG 형식 차단(XX.3.1 mapping)을 함께 넣었습니다. 쓰는 도중이 아니라 내보내기 전에 알리기 위해서입니다.
- `write_svg`의 첫 인자를 `&ExportCapture`에서 `&ConfirmedExport`로 바꿨습니다(XX.3.3 API 변경).

## 남은 위험 / 한계

- missing font는 face마다 glyph 하나로 확인합니다. 손상된 폰트의 일부 glyph(`Malformed`)나 outline 없는 color glyph는
  진단하지 않고 writer가 오류로 멈춥니다.
- PNG 경로(XX.2.1·XX.2.3)는 아직 없어 `ConfirmedExport`를 받는 PNG writer 연결은 그 작업에서 해야 합니다.
- 확인 기록은 메모리 값이며 저장하지 않습니다. 앱이 확인 화면과 진행 사이에 다시 capture하면 다시 확인해야 합니다(scene hash
  비교).
- 확인 UI 문구·표시 방식은 정하지 않았습니다(진단 `Display`는 한국어 기본 문장만 제공).

## 재적용과 수정 (2026-11-23, `lane/svg-export`)

- 앞 lane 브랜치에 남아 통합되지 않은 이 행과, 그 lane이 커밋하지 않고 남긴 회귀 test를 `edfea9db` 위에 다시 적용했습니다.
- 결함 수정: 진단이 run마다 첫 glyph 하나로 폰트 유무만 봐서, 뒤 glyph가 outline 없는 color glyph이거나 손상 glyph이면
  진단을 통과한 내보내기가 writer에서 멈췄습니다. SVG 요청은 vector로 쓸 Text의 glyph key를 모두 조회(같은 key는 한 번)해
  `GlyphBlocker`(이유 NoOutline·Malformed)로 알리고, 확인해도 진행하지 않습니다(`PreflightRejection::GlyphsBlocked`).
  PNG 요청은 renderer가 bitmap glyph도 그리므로 폰트 유무만 봅니다.
- 부분 래스터화 연결(XX.4.2): `PreflightInput::rasterizer`가 렌더를 제공하면 래스터 기능을 형식 차단에서 빼고, 래스터
  경계 안 Text는 glyph outline 진단을 하지 않습니다.
- 결정 기록: ADR-0091 결정 2·3.

| 검사(테스트) | 결과 |
|---|---|
| 첫 glyph가 정상이어도 뒤 color glyph·손상 glyph를 진단하고, 진단을 통과한 내보내기는 writer가 멈추지 않음(`exports_that_pass_the_preflight_do_not_stop_on_glyphs`, `glyphs_without_outlines_block_svg_but_not_png`) | 통과 |
| rasterizer가 있으면 shadow 차단 해제, 경계 안 color glyph 차단 없음(`rasterizers_lift_raster_blockers_and_skip_rasterized_glyphs`) | 통과 |
