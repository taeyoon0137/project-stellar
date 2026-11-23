# 작업 보고서: XXII.5.1 생성 파일 순서·식별자 escape·formatting 고정

## WBS

- WBS: XXII.5.1
- 작업명: 생성 파일 순서, 식별자 escape, formatting 고정
- 상태: 부분 완료 (`[~]`) — Apple·Android 출력 파일 집합 연결이 남았습니다.
- 결정: ADR-0106 결정 8
- 선행: XXII.2\~XXII.4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust 로컬 검증).

## 변경 파일

- `crates/codegen/src/output.rs`(신규): `GeneratedFiles`(경로 byte 오름차순), `FileKind`별 생성 marker 주석,
  `normalize`(LF, 끝 줄바꿈 하나), 상대 경로 검사, `manifest_text`.
- `crates/codegen/src/lib.rs`: module 등록. JavaScript 식별자·문자열 escape는 XXII.2.3에서 만든 `web::js`를 씁니다.
- `crates/ops/src/export_web.rs`(신규): IR → Web 파일 집합(`context.js`, `index.html`, `state.js`, `styles.css`,
  `tokens.css`, `tokens.js`). 규칙이 가리키는 element와 export root에만 id를 붙입니다.
- `crates/ops/src/export_web/tests.rs`(신규), `crates/ops/src/export_web/fixture/web_export.txt`(신규 golden).
- `crates/ops/src/lib.rs`: module 등록.

## 구현 요약

- 파일 순서: 경로 byte 오름차순. 빈 파일은 만들지 않고, `index.html`은 있는 CSS·script만 연결합니다.
- formatting: UTF-8, LF, 끝 줄바꿈 하나, 두 칸 들여쓰기. 파일 첫 줄(HTML은 doctype 다음 줄)에 생성 marker 주석
  (`stellar:generated …다시 가져오지 않습니다`)을 씁니다.
- 식별자: HTML id `s-<StableId 32자리 hex>`, CSS custom property 이름은 문서 IX.11.16 escape, JavaScript 문자열은 JSON
  escape(`</`·U+2028·U+2029 포함), JavaScript 식별자 규칙은 `web::js::Identifiers`(ASCII, 예약어 회피, 충돌 시 hex 접미사).
- 경로: 상대 경로만, `..`·`.`·빈 조각·`\`·NUL 거부.

## 실행한 명령 / 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-codegen` | 통과(92 + 2) |
| `cargo test -p stellar-ops --lib export_web` | 통과(10) |
| `cargo clippy -p stellar-codegen -p stellar-ops --all-targets -- -D warnings` | 경고 0 |

검증 기준 대응:

- same input byte-identical: `ops::export_web::tests::same_input_is_byte_identical_and_files_are_in_fixed_order`(두 번
  출력 동일, 파일 순서 고정, golden `web_export.txt`).
- 식별자·formatting: `identifiers_are_escaped_and_selectors_point_at_ids`(모든 selector id가 HTML에 있음, HTML 텍스트
  escape, JavaScript에 `</script` 없음, 토큰 이름 CSS escape, 모든 파일 LF·끝 줄바꿈 하나·marker),
  `codegen::output::tests::paths_formatting_and_order_are_fixed`, `web::js::tests::*`.

## 차이(작업 지시와 다른 점)

- Web 출력만 연결했습니다.

## 남은 위험

- 접근성 entry가 없는 IR(fixture)에서는 접근성 mapper가 subtree를 숨김 처리합니다. 실제 View는 XXII.1.1 재개 뒤
  접근성 snapshot과 함께 들어옵니다.
- Apple·Android 출력 파일 집합 연결은 XXII.3·XXII.4 몫입니다.
