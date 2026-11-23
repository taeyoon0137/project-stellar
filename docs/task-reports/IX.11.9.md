# 작업 보고서: IX.11.9 Frame 위치와 transform 분리

- WBS: IX.11.9
- 작업명: Frame 위치와 transform 분리 migration
- 상태: 부분 완료 (`[~]`) — 레코드 변환, CSS 순서 합성, rotate(O13) 규칙은 끝났습니다. 다음이 남았습니다.
  - 제품 step 등록
  - 캔버스 배치 속성 저장 위치
  - `transform-origin` longhand
  - binding·variant 참조 보고
  - 인스펙터 경로
- 결정: ADR-0024 결정 8, ADR-0086 결정 3
- 선행: IX.11.6, IX.11.21, IX.11.22, X.1.2, V.8.6

## 필수 실행 환경

- `DEV-MAC` (순수 Rust).

## 변경 파일 목록

- `crates/project/src/migration/steps/{mod,frame_position}.rs`(신규), `crates/project/src/migration/mod.rs`(등록 한 줄).
- `crates/layout/src/transform.rs`: `css_node_transform`, `rotate_matrix`.

## 구현 요약

- 부모가 있는 Frame은 이동 성분을 layout record `position: absolute; left; top`으로 옮깁니다. 선형 부분은
  `matrix(a, b, c, d, 0, 0)` bit 그대로입니다.
- root Frame 이동 성분은 캔버스 배치로 보고서에 남깁니다. 이미 위치 longhand가 있으면 건너뜁니다(두 번 적용해도
  같음).
- O13 결정: 직각 배수(0·90·180·270도, ±360k)는 sin·cos를 정확한 0·±1로 둡니다. 그 밖은 f64 `sin_cos`를 f32로
  반올림합니다.
  - 근거: 직각 배수는 플랫폼 무관하게 bit가 같아야 하는 흔한 입력입니다.
  - 그 밖의 각은 libm 차이가 남을 수 있어 저장 형식은 `matrix()`로 둡니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-project frame_position` | 통과. 선형 부분 bit 동일, 이동 → left·top, root 보존, 멱등. 합성 결과가 옛 transform과 같습니다. |
| `cargo test -p stellar-layout transform` | 통과. 0·90·180·270·-90·450도 bit 결과, origin 고정점. |

## 차이(작업 지시와 다른 점)

- step을 `PRODUCT_STEPS`에 넣지 않았습니다. scene이 아직 layout 결과 위치로 그리지 않아, 켜면 옮긴 Frame이 원점에
  그려집니다.
  - (2026-11-23 style-defaults-finish 재확인) `crates/scene`은 여전히 `stellar-layout`에 의존하지 않습니다. 등록 조건과
    1→2 묶음 순서는 ADR-0115 결정 5에 적었습니다.

## 남은 위험 / 한계

- `cargo xtask test-migrations` 대상 step이 아직 없습니다.
- flow 부모 입력의 `translate` 분기, View·ComponentDefinition root의 캔버스 배치 longhand가 남았습니다.
