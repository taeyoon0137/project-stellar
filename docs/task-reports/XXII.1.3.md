# 작업 보고서: XXII.1.3 CodegenIR 단방향 API inventory

## WBS

- WBS: XXII.1.3
- 작업명: CodegenIR은 one-way이며 generated source parser/import API를 만들지 않음
- 상태: 완료 (`[x]`) — 선행 XXII.1.1이 차단 상태라 원장 표기는 통합 때 정합니다.
- 결정: ADR-0106 결정 9, ADR-0025 결정 4.5, 기술 아키텍처 §17.3
- 선행: XXII.1.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust 로컬 검증).

## 변경 파일

- `crates/codegen/tests/one_way_api.rs`(신규): 공개 API inventory 검사.
- `crates/codegen/src/lib.rs`: 단방향 원칙과 검사 위치를 문서에 적음.

## 구현 요약

- `stellar-codegen`의 모든 source와 ops 코드 생성 module(`crates/ops/src/codegen*`, `crates/ops/src/export_web*`)에서
  test module 밖의 공개 item(`fn`·`trait`·`struct`·`enum`·`mod`·`type`) 이름을 모읍니다.
- 이름에 `parse`·`import`·`decode`·`deserialize`·`from_html`·`from_css`·`from_js`·`from_swift`·`from_kotlin`·
  `from_source`·`from_generated`·`read_generated`·`round_trip`·`reverse`·`ingest`가 있으면 실패합니다.
- `FromStr`·`TryFrom<&str>`·`TryFrom<String>`·`Deserialize` 구현이 있어도 실패합니다.
- 검사기가 실제로 잡는지 합성 source로 확인합니다.

## 실행한 명령 / 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-codegen --test one_way_api` | 통과(2) |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험

- 이름 기반 inventory입니다. 이름을 바꿔 되읽기 API를 만드는 경우는 리뷰로 막아야 합니다.
