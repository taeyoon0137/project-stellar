# 작업 보고서: XIII.6.7 missing font 명시 fallback과 줄바꿈 변경 진단

- WBS: XIII.6.7
- 작업명: missing font에서 명시 fallback chain만 적용하고 줄바꿈 변경 진단을 생성
- 상태: 완료 (`[x]`)
- 선행: X.8.3(`[x]`), XIII.6.2(`[x]`), XIII.6.3(`[!]` Windows 전용 — ADR-0006에 따라 충족으로 봄)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/fonts/missing.rs`(신규): `resolve_text_fonts`, `TextFaceResolution`, `TextFontResolution`,
  `MissingFontReport`, `LineBreakSignature`, `LineBreakChange`, `LineBreakChangeDiagnostic`, `LineBreakEvidence`,
  `NoLineBreakEvidence`, 테스트.
- `crates/project/src/fonts/mod.rs`: `pub mod missing;`과 모듈 목록 한 줄.
- `docs/task-reports/XIII.6.7.md`(신규), 계획서(XIII.6.7 `[ ]`→`[x]`).

## 구현 요약

- **명시 fallback만 적용**: Text마다 기록된 `FontFallbackChain`(X.8.3)을 registry에 대해 해석합니다. primary가
  등록되어 있으면 `Primary`, 없으면 chain 기록 순서상 다음으로 등록된 항목 `Fallback { chain_index, handle }`,
  chain 안에 등록 face가 없으면 `NoFace`(missing glyph 경로). chain 밖 face는 registry에 있어도(같은 family
  이름의 시스템 font 포함) 쓰지 않습니다. face 정체성은 file hash + face index(X.8.2)입니다.
- **누락 진단**: 누락 face는 X.8.4 `collect_missing_fonts`로 요청 face·출처·영향 Text를 함께 보고합니다.
- **줄바꿈 변경 진단**: 대체(`Fallback`·`NoFace`)가 일어난 Text마다 `LineBreakChangeDiagnostic`
  (코드 `font-fallback-line-break`)을 만듭니다. 호출자가 `LineBreakEvidence`로 의도 face 기준 줄바꿈과 대체
  결과로 다시 계산한 줄바꿈을 주면 비교해 같으면 진단 없음, 다르면 `Changed { before_lines, after_lines }`,
  어느 한쪽이라도 없으면 `MayChange`입니다.
- **내용 미포함·결정론**: 줄바꿈 요약(`LineBreakSignature`)은 줄 수와 줄 경계(byte 범위·강제 줄바꿈)의 SHA-256
  digest만 담고 Text 내용·줄 폭은 넣지 않습니다. 결과는 Text id 오름차순이며 입력 순서와 무관합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 358 passed, 1 ignored(수동 system font scan); 통합 테스트 전부 통과) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **missing face fixture expected fallback**: 시스템 discovery(XIII.6.2)로 실제 파일 B·C·Z를 등록(Z는 체인 밖, B와 같은 family 이름), Project A·External D 미등록. `[A,B,C]` → B(1번), `[A,D,C]` → C(2번), `[B,A]` → Primary B, `[A,D]` → NoFace, Z 사용 0, 누락 진단 A{1,2,3,4}·D{2,4}, 줄바꿈 진단 Text 1·2·4 `MayChange`, 입력 순서 반대여도 같은 결과(`missing_primary_uses_only_the_explicit_chain`) | 통과 |
| 비교 입력: 2줄→3줄 `Changed`, 같은 줄 경계는 진단 없음, 한쪽만 있으면 `MayChange`(`line_break_evidence_confirms_or_clears_the_change`) | 통과 |
| 모두 등록된 chain은 진단 없음(`fully_available_chains_produce_no_diagnostics`) | 통과 |
| 줄바꿈 요약이 줄 경계·강제 줄바꿈만 반영(`signature_depends_on_line_boundaries_only`) | 통과 |

## 차이(작업 지시와 다른 점)

- 줄바꿈을 실제로 다시 계산하는 텍스트 엔진 호출(X.10)은 이 모듈 밖(`LineBreakEvidence` 구현자)에 둡니다.
  project crate에는 font bytes·shaping 입력이 없어서이며, 비교 입력이 없을 때는 대체 사실만으로 `MayChange`를
  냅니다.
- XIII.6.3(Windows discovery)은 `[!]`라 macOS discovery로만 fixture를 만들었습니다(ADR-0006).

## 남은 위험 / 한계

- 기준 줄바꿈(의도 face로 마지막에 계산한 결과)을 어디에 보관할지(문서 cache·layout cache)는 아직 정하지
  않았습니다. 연결 전에는 대체가 일어난 Text가 모두 `MayChange`입니다.
- glyph coverage 기준 run 단위 fallback(X.9.5)은 shaping 단계의 몫이며, 이 모듈은 Text 단위 face 선택과
  진단만 다룹니다.
