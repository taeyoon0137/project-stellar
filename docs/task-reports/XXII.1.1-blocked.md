# 차단 보고서: XXII.1.1 resolved View → CodegenIR 변환

## WBS

- WBS: XXII.1.1
- 작업명: resolved View를 layout, semantics, state, tokens, target capability가 포함된 `CodegenIR`로 변환
- 상태: `[!]`
- 선행: G8(gate), IX.10.8 `[x]`, IX.11.6 `[ ]`, IX.11.11 `[ ]`, IX.11.19 `[ ]`

## 실패 선행조건

- CSS 기반 레이아웃·타이포그래피 영속 record와 타깃 capability matrix 행(IX.11.6·IX.11.11·IX.11.19)이 `[ ]`입니다.
- 첫 구현(`7c152dc6`)은 ADR-0025가 이 행을 CSS 기반 속성을 소비하도록 고치기 전의 정의로 만들었습니다. 그래서 IR 입력이
  CSS 기반 속성 record가 아니라 하위 crate 지역 모델입니다.
- 2026-11-14 통합에서 `[~]`로 올렸고, 2026-11-15 검토에서 `[!]`로 고쳤습니다.

## 재현 명령

- `rg -n '^\| (IX\.11\.6|IX\.11\.11|IX\.11\.19|XXII\.1\.1) ' docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md`
- `cargo test -p stellar-codegen` — 이미 있는 IR의 검사입니다. 2026-11-15 DEV-MAC의 `cargo test --workspace`(종료 0)에
  포함되어 통과했습니다.

## 필요한 결정과 환경

- `DEV-MAC`. [ADR-0049](../adr/ADR-0049-codegen-crate.md) 결정 6(2026-11-15 보완)은 지금 IR 입력 경계를 IX.11 행이 끝날
  때까지의 임시 경계로 정했습니다. ADR-0025 결정 4가 우선합니다.

## 변경되지 않은 파일

- 선행 완료 전에 들어온 구현은 유지합니다: `crates/codegen`의 `build_ir`·IR 모델·semantic snapshot과 golden(`7c152dc6`),
  2026-11-15 검토의 Fragment source 접근성 entry 거부와 자식 목록 한 번 계산(`8fab565c`).
- 문서 계층에서 IR 입력을 채우는 연결은 없습니다. 실제 프로젝트 View로는 아직 IR을 만들 수 없습니다.
- 첫 구현은 작업 규칙 13(한 작업의 프로덕션 파일 5개 이하)을 넘었고, 리뷰 전 커밋이 통합되었습니다.

## 재개 조건

- IX.11.6·IX.11.11·IX.11.19가 `[x]`가 된 뒤 IR 입력을 CSS 기반 속성 record로 바꾸고, 검증 기준(profile별 IR semantic snapshot
  golden)을 다시 실행해 보고서를 완료 형식으로 씁니다.
