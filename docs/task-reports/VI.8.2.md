# 작업 보고서: VI.8.2 누락 asset 진단

- WBS: VI.8.2
- 작업명: missing AssetRef를 invalid node가 아닌 missing resource diagnostic으로 표현, placeholder bounds 유지
- 상태: 완료 (`[x]`)
- 선행: VI.8.1, III.2.4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/image.rs` — `ImageResolution`, `ImagePayload::resolve_asset`,
  `ImagePayload::layout_bounds`, `IMAGE_PROP_ASSET`.
- `docs/task-reports/VI.8.2.md`(신규), 계획서(VI.8.2 `[ ]`→`[x]`).

## 구현 요약

- **진단, not invalid**: `resolve_asset(image_id, asset_exists)`는 asset이 없으면
  `MissingResource(UnresolvedRef)`(기대 타입 Asset, 위치 `PropertyPath(image, ASSET)`)를
  반환합니다. node는 그대로 유효하고 저장·복원됩니다.
- **placeholder bounds**: `layout_bounds()`는 payload의 배치 상자 크기에서만 계산해
  asset 존재 여부와 무관합니다 — asset이 사라져도 layout이 바뀌지 않고, 렌더러는 같은
  bounds에 placeholder를 그립니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 127 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 에셋 누락 전후 layout bounds 동일 + 진단 id·타입·위치 보존 + node 유효(`missing_asset_keeps_layout_bounds`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- asset store 조회(Phase XIII 에셋 관리)와 진단 패널 표시는 후속입니다.
