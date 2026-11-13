# 작업 보고서: X.6.1 SizingMode 정의와 잘못된 조합 거부

- WBS: X.6.1
- 작업명: SizingMode에 Fixed, Fill, HugContent, Percentage 정의
- 상태: 완료 (`[x]`)
- 선행: X.1.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/sizing.rs`(신규) — `SizingMode { Fixed, Fill, HugContent, Percentage }`, canonical tag, 축별 값·조합 검증.
- `crates/layout/src/input.rs` — `SizingMode`를 sizing 모듈에서 재노출, 검증에 root 여부 전달, sizing tag 인코딩·디코딩.
- `crates/layout/src/error.rs` — `LayoutError::InvalidSizing { node, axis, reason }`.
- `crates/layout/src/lib.rs` — `pub mod sizing;`.
- `docs/task-reports/X.6.1.md`(신규), 계획서(X.6.1 `[ ]`→`[x]`).

## 구현 요약

- **mode 의미**: `Fixed(len)` 고정, `Fill` 부모가 정한 영역 채움, `HugContent` 자식 extent 또는 intrinsic
  콘텐츠 + padding, `Percentage(p)` containing block content 크기의 p%(50 = 50%). `Fill`·`Percentage`는
  containing block에 의존합니다(`depends_on_containing_block`).
- **거부 조합**: root node의 `Fill`·`Percentage`(root에는 containing block이 없음)는
  `InvalidSizing { axis }`, 음수 `Fixed`·`Percentage`는 `NegativeValue`, 범위 밖 `Fixed`는 `OutOfRange`.
- **허용 조합(명시)**: 부모 `HugContent` 축의 자식 `Fill`·`Percentage`는 허용합니다. 자식은 부모 측정에
  자신의 콘텐츠 크기로 기여하고, 부모 크기 확정 후 다시 해석되므로 순환이 생기지 않습니다(엔진 X.2.1에서 구현).
- **canonical tag**: Fixed 0, Fill 1, HugContent 2, Percentage 3(뒤에 finite f32 값).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 41 unit passed + 2 doctest passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| root (가로, 세로) 16개 mode 조합 전부: Fill·Percentage가 있는 축은 `InvalidSizing`(가로 우선), 나머지 통과(`invalid_mode_combinations_are_rejected`) | 통과 |
| 자식 16개 조합은 부모 Hug에서도 전부 허용(같은 테스트) | 통과 |
| 음수 Percentage·Fixed 거부, 0%·250% 허용(`negative_values_are_rejected`) | 통과 |
| 16개 조합 canonical round-trip과 tag golden `[0, 1, 2, 3]`(`sizing_modes_round_trip_with_fixed_tags`) | 통과 |

## 차이(작업 지시와 다른 점)

- 계획 순서상 X.2보다 뒤지만 같은 선행(X.1.1)만 필요하고 X.2.x 배치 규칙이 sizing mode를 사용하므로 먼저
  수행했습니다(섹션 내부 순서 유지).
- 이 작업은 타입과 검증만 정의합니다. mode별 크기 계산은 레이아웃 엔진(X.2.1 이후)이 구현합니다.

## 남은 위험 / 한계

- 다른 속성과의 조합 규칙(anchor 양끝 + 고정 크기의 과다 제약 X.2.2, flex basis와 main축 Fill 충돌 X.3.3,
  aspect ratio X.6.3, Constraint container의 Hug 등)은 해당 작업에서 추가합니다.
- HugContent cycle 진단(X.6.5)은 intrinsic 측정 trait(X.6.4) 선행이 미완료라 이 lane에서 다루지 않습니다.
