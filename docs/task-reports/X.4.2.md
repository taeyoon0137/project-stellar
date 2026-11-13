# 작업 보고서: X.4.2 explicit row/column track sizing

- WBS: X.4.2
- 작업명: explicit row/column track sizing 구현
- 상태: 완료 (`[x]`)
- 선행: X.4.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/grid_tracks.rs`(신규, 내부) — 한 축 track sizing `size_tracks`, `track_offsets`, 단위 fixture 테스트.
- `crates/layout/src/lib.rs` — `mod grid_tracks;`(엔진 연결 X.4.3 전까지 비테스트 빌드 dead code 허용).
- `docs/task-reports/X.4.2.md`(신규), 계획서(X.4.2 `[ ]`→`[x]`).

## 구현 요약

- track을 (최솟값, 최댓값)으로 봅니다: Fixed(v) = (v, v), Fraction(f) = (0, f fr), MinContent·MaxContent =
  (콘텐츠, 콘텐츠), MinMax(a, b) = (a, b).
- **초기화**: base = 최솟값(고정 값 또는 span 1 item 기여 최댓값), limit = 최댓값(fr은 무한, limit ≥ base).
- **span item**: fr track을 걸치지 않는 span ≥ 2 item의 부족분을 걸친 콘텐츠 최솟값 track(없으면 콘텐츠
  최댓값 track)에 균등 분배(span 오름차순, 잔여 subpixel은 track 순서).
- **maximize**: 가용 크기가 definite이고 남으면 비fr track을 limit까지 균등하게 늘립니다.
- **fr**: definite면 남은 공간을 fr 비례 정수 분배(잔여는 track 순서), 몫 < base인 fr track은 base로 고정 후
  재분배. indefinite면 `fr 단위 = max(기여/fr, base/fr)`(유리수 비교)로 각 fr track = max(base, 단위 × fr).
- item 기여 크기는 호출자 입력입니다. 엔진에서 item 측정 크기를 기여로 연결하는 것은 X.4.3, 텍스트·이미지
  intrinsic hook 연결은 X.4.5 범위입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 83 unit passed + 2 doctest passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| fixed/fraction fixture: [100, 1fr, 2fr] 가용 400 gap 10 → 6400/5974/11946 subpixel(잔여 1은 첫 fr), offset·extent 400, 고정 넘침 유지, fr만 100×3, 고정이 다 쓰면 fr 0(`fixed_and_fraction_fixture`) | 통과 |
| minmax(150, 1fr) 고정 후 재분배 150/50, maximize minmax(20, 50) → 50(`minmax_fraction_freezes_at_base`) | 통과 |
| content track + span 2 부족분 분배 35/35, fr 걸친 span 무시(`content_tracks_and_spanning_items`) | 통과 |
| minmax(min-content, 100) maximize 80 / indefinite 30(`minmax_content_minimum_grows_to_limit`) | 통과 |
| indefinite fr 단위 max(30/1, 40/2) → 30/60, item 없음 0(`indefinite_fraction_uses_largest_fr_unit`) | 통과 |

## 차이(작업 지시와 다른 점)

- `Fraction(f)`의 최솟값은 0으로 둡니다(CSS의 `auto` 최솟값 대신). 콘텐츠 최솟값이 필요하면 `MinMax(MinContent, fr)`를 씁니다.
- 이 작업은 track sizing 함수와 그 fixture까지입니다. Grid container의 엔진 배치는 item 배치(X.4.3)와 함께 연결합니다.

## 남은 위험 / 한계

- min-content와 max-content 기여는 현재 같은 측정 값입니다(텍스트 줄바꿈 기반 차이는 X.4.5·X.10.4 이후).
