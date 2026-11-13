# 작업 보고서: VI.3.5 Frame property metadata registry

- WBS: VI.3.5
- 작업명: Frame 각 PropertyKey, 타입, 기본값, layout/paint invalidation 범위 고정
- 상태: 완료 (`[x]`)
- 선행: VI.3.1, III.2.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/frame_metadata.rs`(신규) — `FRAME_PROPERTIES`(7항목),
  `FramePropertyMetadata`, `FramePropertyType`, `Invalidation`, `frame_property`,
  `frame_field_keys`, `register_frame_property_names`.
- `crates/document/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VI.3.5.md`(신규), 계획서(VI.3.5 `[ ]`→`[x]`).

## 구현 요약

- **registry**: size·transform·clipping·background·border·corner-radius·
  layout-mode 7개 속성의 key(`frame::keys`, VI.3.4), 값 타입, 기본값 설명, 무효화
  범위를 key 오름차순 상수 표로 고정했습니다.
- **무효화 범위**: layout+paint = size, border(두께가 content box에 영향을 줄 수
  있어 보수적), layout-mode. paint만 = transform(시각 변환은 배치를 바꾸지 않음),
  clipping, background, corner-radius.
- **누락 검사**: `frame_field_keys`가 `FramePayload`를 `..` 없이 구조 분해하므로
  payload 필드 추가·제거 시 컴파일이 실패하고, 테스트가 registry↔필드 1:1과
  기본값 일치를 검사합니다. 이름은 III.2.1 `PropertyNameRegistry`에 등록할 수
  있습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 68 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| registry와 payload 필드 1:1 누락 검사(`registry_and_payload_fields_match_one_to_one`) | 통과 |
| key·이름 유일·정렬, 기본값 일치, 무효화 분류 고정, 이름 registry 등록 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- border의 layout 영향 여부는 layout 엔진(Phase X) box model 확정 후 재검토합니다
  (현재는 보수적으로 layout까지 무효화).
