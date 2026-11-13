# 작업 보고서: X.10.3 caret 위치·선택 사각형·hit-test cluster mapping

- WBS: X.10.3
- 작업명: caret positions, selection rects, hit-test cluster mapping 생성
- 상태: 완료 (`[x]`)
- 선행: X.10.1(이 lane `[x]`), X.9.3(이 lane `[x]`)

## 필수 실행 환경

- `DEV-MAC` (Apple Silicon arm64, 순수 Rust 로컬 검증).

## 변경 파일 목록

- `crates/text/src/caret.rs`(신규) — `caret_position`, `caret_stops`, `selection_rects`, `hit_test`, `line_geometry`,
  `TextPosition`, `Affinity`, `CaretPosition`, `SelectionRect`, `HitTestResult`, `LineGeometry`, `CaretError`.
- `crates/text/src/text_layout.rs` — `PositionedLine::order`(줄 BiDi mapping을 최종 줄에 보존).
- `crates/text/src/lib.rs` — 모듈 선언.
- `docs/task-reports/X.10.3.md`(신규), 계획서(X.10.3 `[ ]`→`[x]`).

## 구현 요약

- **cluster 기하**: 줄의 glyph를 shaping cluster로 묶어 x 범위를 구하고, 한 shaping cluster에 grapheme이 여럿이면
  (ligature) 폭을 정수 등분합니다(RTL은 오른쪽부터). glyph가 없는 grapheme은 visual 앞 cluster 오른쪽에 폭 0.
- **caret**: (paragraph, byte offset, affinity). `Downstream` = offset 뒤 grapheme의 leading edge(LTR 왼쪽·RTL
  오른쪽), `Upstream` = offset 앞 grapheme의 trailing edge. 줄 경계 offset은 affinity로 앞·뒤 줄을 고르고, 빈 줄은
  줄 x입니다. caret 정지 위치는 grapheme 경계입니다.
- **선택**: 범위 안 grapheme을 visual 순서로 모아 인접 구간을 합친 사각형(줄 순서). BiDi 경계를 넘는 논리 범위는
  화면에서 떨어진 여러 사각형이 됩니다. 경계가 grapheme 경계가 아니거나 범위가 거꾸로면 오류입니다.
- **hit test**: y로 줄(위·아래·줄 사이 밖은 가까운 줄), x로 visual 순서 grapheme을 찾고, leading 절반 →
  (시작, Downstream), trailing 절반 → (끝, Upstream). 줄 밖 좌우는 visual 가장자리 grapheme의 해당 edge입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 / 결과 |
|---|---|
| `cargo fmt -p stellar-text` | 0 |
| `cargo test -p stellar-text` | 0 (97 unit passed, 0 failed) |
| `cargo clippy -p stellar-text --all-targets -- -D warnings` | 0 (경고 0) |
| `cargo fmt -p stellar-text -- --check` | 0 |

| 검사 | 결과 |
|---|---|
| **BiDi caret fixture**("abc אבג def", LTR): offset 4 Downstream 3175(א 오른쪽)/Upstream 1792(공백 오른쪽), offset 10 Downstream 3175/Upstream 1792, RTL 안쪽 offset 6은 둘 다 2714, 줄 시작 0·끝 4762, 방향 RTL, 정지 위치 12개 | 통과 |
| BiDi 선택: `[3,6)` → 떨어진 두 사각형 (1536,256)·(2714,461), 히브리어 전체 `[4,10)` → 한 사각형, 빈 선택·경계 위반·역순 오류 | 통과 |
| hit test: ג 왼쪽 절반 → (10, Upstream), א 오른쪽 절반 → (4, Downstream), LTR 절반 규칙, 9개 점 왕복(caret x = 맞은 cluster 가장자리), 줄 밖 좌우 | 통과 |
| 여러 줄: 줄 경계 offset의 affinity별 줄, y로 줄 선택(위·아래 밖 포함), 없는 paragraph 오류 | 통과 |
| ligature "ffi"(819) 3등분 caret 512/785/1058/1331, 빈 줄 caret·hit, 줄 없는 layout은 `None` | 통과 |
| RTL 문단: 논리 시작 caret 오른쪽 끝, 논리 끝 caret 왼쪽 끝, 왼쪽 밖 hit → 논리 끝 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 세로 caret 이동(위·아래 줄로 같은 x 찾기)과 단어 단위 이동은 범위 밖입니다(IME·편집 세션 XII.5.3에서 사용).
- ligature 내부 caret은 등분 근사입니다(OpenType `GDEF` ligature caret 값은 아직 읽지 않음).
