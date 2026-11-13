# 작업 보고서: VI.4.2 Fragment 금지 속성 validation

- WBS: VI.4.2
- 작업명: Fragment에 transform, opacity, blend, mask, filter, fill, border, padding 설정 시 validation 오류
- 상태: 완료 (`[x]`)
- 선행: VI.4.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/fragment.rs` — `VisualPropertyClass`(8종),
  `validate_visual_property`, `FragmentError::ForbiddenProperty`.
- `docs/task-reports/VI.4.2.md`(신규), 계획서(VI.4.2 `[ ]`→`[x]`).

## 구현 요약

- **분류**: 렌더·layout에 영향을 주는 속성을 `VisualPropertyClass`(transform,
  opacity, blend, mask, filter, fill, border, padding)로 고정했습니다.
- **검증**: `validate_visual_property(kind, class)`는 Fragment에 대해 모든 분류를
  `ForbiddenProperty { class }`로 거부하고, 오류 메시지는 Frame 변환(VI.4.4)을
  안내합니다. 다른 종류의 세부 허용 규칙은 각 종류의 property registry가
  담당하므로 이 규칙에서는 통과시킵니다. command 계층(Phase VII)이 속성 설정 전
  이 검증을 호출합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 72 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 금지 속성 8종 fixture 전부 실패(`every_forbidden_property_fails_on_fragment`) | 통과 |
| Fragment 외 종류는 이 규칙으로 막지 않음(`other_kinds_pass_through_fragment_rule`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- Fragment modifier 삽입 거부(VI.9.6)는 modifier stack 구현 시 이 분류와
  연결합니다.
