# 작업 보고서: VI.3.3 음수 size 거부와 transform 반전 구분

- WBS: VI.3.3
- 작업명: width/height 비음수 강제, transform 반전과 음수 size 구분
- 상태: 완료 (`[x]`)
- 선행: VI.3.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/frame.rs` — `FramePayload::set_size`/`is_mirrored`/
  `mirror_horizontally`/`mirror_vertically`, `signed_extent_to_size_and_mirror`.
- `crates/core-types/src/value.rs` — `Transform2D::then`(합성) 추가.
- `docs/task-reports/VI.3.3.md`(신규), 계획서(VI.3.3 `[ ]`→`[x]`).

## 구현 요약

- **음수 size 거부**: `set_size`는 음수를 `Negative { field: "size" }`, 비유한을
  `NonFinite`로 거부하고 거부 시 기존 size를 유지합니다(`Size2` 타입이 저장
  표현에서도 음수를 막음, VI.3.1).
- **반전은 transform으로**: `mirror_horizontally/vertically`는 local 좌표에서
  `x → width − x`(또는 y) 반전을 먼저 적용한 뒤 기존 transform을 합성
  (`Transform2D::then`)합니다. size는 그대로 비음수이고, 부모 좌표계에서 차지하는
  영역은 변하지 않습니다. `is_mirrored`는 행렬식 < 0으로 판정합니다.
- **signed extent 정규화**: 반대 방향 드래그 등으로 생긴 음수 extent는
  `signed_extent_to_size_and_mirror`가 "양수 size + 축별 scale −1 transform"으로
  바꿉니다(두 축 모두 음수면 180° 회전과 같아 행렬식 양수).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document -p stellar-core-types` | 60 + 116 passed, 0 failed |
| `cargo fmt -- --check`(두 crate) | 통과 |
| `cargo clippy --all-targets -- -D warnings`(두 crate) | 경고 0 |

| 검사 | 결과 |
|---|---|
| 음수 size 거부 + mirror transform 허용 + 반전 frame round-trip(`negative_size_is_rejected_but_mirror_transform_is_allowed`) | 통과 |
| 반전 후 점유 영역 불변, 이중 반전 복원(`mirror_keeps_occupied_region`) | 통과 |
| signed extent 정규화(`signed_extent_normalizes_to_positive_size_and_mirror`), `Transform2D::then` 합성 순서·오버플로 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 반전 frame의 hit test·렌더 경로(Phase XI)는 행렬식 부호에 따른 winding 처리를
  확인해야 합니다.
