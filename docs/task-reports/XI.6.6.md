# 작업 보고서: XI.6.6 glyph cache miss 정책(이전 valid glyph 또는 빈 glyph)

- WBS: XI.6.6
- 작업명: cache miss 시 placeholder 대신 rasterization 완료 전 이전 valid glyph 또는 빈 glyph를 그리는 정책을 명시·테스트
- 상태: 완료 (`[x]`)
- 선행: XI.6.5 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/glyph/resolve.rs`(신규) — `GlyphResolver`, `GlyphDraw`·`GlyphSource`·`EmptyReason`,
  `CompleteOutcome`, `ResolverStats`, 상한 상수(정책은 모듈 문서에 명시).
- `crates/renderer-core/src/glyph/resolve/tests.rs`(신규) — 정책 fixture와 무작위 workload 검사.
- `crates/renderer-core/src/glyph/mod.rs` — `resolve` 모듈·재노출.
- `docs/task-reports/XI.6.6.md`(신규), 계획서(XI.6.6 `[ ]`→`[x]`).

## 구현 요약

- **정책**(placeholder·tofu·`.notdef` 미사용): `resolve`는 (1) 요청 key의 atlas 배치·outline이 있으면 `Exact`,
  (2) 없으면 같은 glyph 모양(폰트 hash·glyph id·variation·mode 모두 같고 크기만 다름) 중 지금 유효한 크기를
  `Substitute{from, scale}`로 — 요청 이상 중 가장 작은 크기, 없으면 요청 미만 중 가장 큰 크기, (3) 그 밖에는
  `Empty(Pending | Failed | NoInk)`입니다. 다른 폰트·face·glyph id·variation·mode는 절대 대체하지 않습니다.
- **요청·반영**: miss key는 대기 집합으로 한 번만 `take_requests`에 나옵니다. `complete`는 mode 불일치·page 초과
  bitmap을 실패로 기록하고(이후 `Empty(Failed)`, `forget_failures`로 재시도), 잉크 없는 결과는 `NoInk`로,
  atlas `Full`은 버리고 다음 resolve에서 다시 요청합니다. Outline mode는 outline path를 저장해 같은 정책을 씁니다.
- **유효성**: 대체 후보는 atlas `lookup`으로만 얻으므로 eviction된 크기는 쓰지 않고(기록에서 제거) 고른 대체
  glyph는 현재 frame 사용으로 표시되어 그 frame 동안 eviction되지 않습니다(XI.6.5).
- **상한**: 대기 4096, 실패·무잉크·outline·크기 기록 각 4096(오래된 것부터 버림), glyph 모양당 크기 16개.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-renderer-core` | 종료 코드 0 |
| `cargo test -p stellar-renderer-core` | 131 unit passed(신규 6) + 3 doctest passed, 0 failed |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| 처음 보는 glyph는 `Empty(Pending)`·요청 1회, zoom 18 px는 16 px를 1.125배 대체, 완료 후 `Exact`, 대체 우선순위 4경우(`zoom_draws_previous_valid_size_until_exact_is_ready`) | 통과 |
| 다른 glyph id·폰트·face·variation·mode는 cache에 같은 크기 glyph가 있어도 대체하지 않음(`never_substitutes_a_different_glyph`) | 통과 |
| 공백 `NoInk`·재요청 없음, 실패 `Failed`·재요청 없음·forget 후 재요청, mode 불일치·page 초과 결과는 실패(`failures_no_ink_and_bad_results_draw_nothing`) | 통과 |
| Outline mode 이전 크기 outline 대체(scale 1.2)(`outline_mode_uses_previous_outline_size`) | 통과 |
| 대체 glyph page는 현재 frame 보호(삽입 `Full`), eviction된 크기는 대체하지 않음(`evicted_sizes_are_not_substituted_and_substitutes_are_protected`) | 통과 |
| 무작위 400 frame(2 폰트 × 6 glyph × 4 크기 × 2 variation × 3 mode, 늦은 도착·5% 실패·작은 atlas eviction): `Exact`는 정확히 그 key·유효 배치, `Substitute`는 같은 모양·다른 크기·유효 배치, 잉크 있는 glyph의 `NoInk` 0, 대체·eviction 실제 발생(`random_workload_never_shows_a_wrong_or_stale_glyph`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 실제 font rasterizer와 worker 연결은 text·worker 통합 작업에서 이 interface(`take_requests`/`complete`)로
  연결합니다.

## 남은 위험 / 한계

- 대체 glyph를 크기 비율로 늘이면 확대 시 흐려집니다(의도된 일시 표시). 대체 표시 시간은 rasterize 지연에 달려
  있으므로 XVI 성능 측정에서 확인해야 합니다.
- 실패 기록은 `forget_failures` 전까지 유지되어 일시적 실패도 빈 glyph로 남습니다. 폰트 재등록 이벤트에서 호출해야
  합니다.
