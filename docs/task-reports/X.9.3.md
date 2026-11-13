# 작업 보고서: X.9.3 Unicode BiDi 줄 재배치와 logical cluster mapping

- WBS: X.9.3
- 작업명: 승인 dependency(`unicode-bidi`) wrapper로 visual order와 logical cluster mapping 유지
- 상태: 완료 (`[x]`)
- 선행: X.9.2(이 lane `[x]`)

## 필수 실행 환경

- `DEV-MAC` (Apple Silicon arm64, 순수 Rust 로컬 검증).

## 변경 파일 목록

- `crates/text/src/bidi.rs` — `BidiParagraph::line_levels`(L1), `BidiParagraph::line_order`(L2 + cluster mapping),
  `LineOrder`, `VisualRun`, `BidiError`, 고정 corpus·참조 일치 테스트.
- `crates/text/src/lib.rs` — 모듈 설명.
- `crates/text/Cargo.toml` — `unicode-segmentation = "=1.13.3"`.
- `config/dependencies.yaml` — `unicode-segmentation` 승인 항목.
- `docs/adr/ADR-0016-text-shaping-dependencies.md` — Decision 4에 정확한 버전(1.13.3, Unicode 17.0) 기록.
- `Cargo.lock` — `unicode-segmentation` 1.13.3(자동).
- `docs/task-reports/X.9.3.md`(신규), 계획서(X.9.3 `[ ]`→`[x]`).

## 구현 요약

- **L1**: 줄마다 S·B 구분자와 그 앞 공백·isolate 서식 문자열, 줄 끝 공백열을 해당 BiDi paragraph level로
  되돌립니다. 보존된 explicit 서식 문자·BN은 앞 문자 level을 따릅니다(`unicode-bidi` `reordered_levels`와
  같은 규칙). 줄이 BiDi paragraph 경계를 넘으면 paragraph별로 따로 적용합니다.
- **L2 + logical cluster mapping**: 재배치 최소 단위는 UAX #29 extended grapheme cluster입니다. 같은
  level의 연속 cluster를 논리 run으로 묶고 최고 level부터 가장 낮은 홀수 level까지 run 구간을 뒤집습니다
  (level ≤ 126이라 유한 반복, 재귀 없음). RTL run 안 cluster는 역순입니다. `LineOrder`는 visual→logical,
  logical→visual mapping, visual 순서 run(연속 byte 범위 + level), offset→cluster 조회를 제공합니다.
- 결합 부호는 base와 같은 cluster라 재배치 후에도 붙어 있습니다(L3 효과). L4(거울 glyph)는 shaping 엔진
  (RTL run)이 처리합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 / 결과 |
|---|---|
| `cargo fmt -p stellar-text` | 0 |
| `cargo test -p stellar-text` | 0 (58 unit passed, 0 failed) |
| `cargo clippy -p stellar-text --all-targets -- -D warnings` | 0 (경고 0) |
| `cargo fmt -p stellar-text -- --check` | 0 |
| `cargo run -q -p xtask -- validate-dependencies` | **65 (`unused-approval` ×3)** — X.9.1 보고서의 validator 범위 문제 |
| workspace 직접 외부 의존성 대조(scratch 스크립트) | 직접 의존 = 승인 목록(harfrust, unicode-bidi, unicode-segmentation, yaml-rust2) |

| 검사 | 결과 |
|---|---|
| **고정 BiDi corpus 19항목**(`fixed_bidi_corpus_passes`): L·R·AL·EN·AN·CS·WS·S, Auto 방향, W2·W4·W7·N0·N1·N2·I1·I2·L1·L2, RLI/PDI isolate, 결합 부호 cluster, 빈 줄. 기대 level과 visual 순서는 UAX #9 규칙을 손으로 적용해 고정 | 통과 |
| 양방향 mapping 역함수, visual run이 줄을 정확히 덮음 | 통과 |
| **참조 일치**(`corpus_agrees_with_reference_reordering`): 단일 문자 cluster 항목 18개가 `unicode-bidi` `reordered_levels_per_char`·`reorder_visual`과 level·순서 일치 | 통과 |
| 부분 줄 재배치, visual run 범위, offset→cluster 조회 | 통과 |
| 텍스트 불일치·문자 경계 위반·범위 밖 줄 거부 | 통과 |
| explicit embedding 400개 중첩도 level ≤ 126, 유한 처리 | 통과 |
| Unicode 데이터 버전 고정(BiDi 16.0, grapheme 17.0) | 통과 |

## 차이(작업 지시와 다른 점)

- "공식 corpus"(Unicode `BidiCharacterTest.txt`, 수 MB)는 저장소에 넣지 않고, 규칙별 고정 corpus 19항목과
  참조 구현 일치 검사로 대신했습니다(작업 지시의 "공식 또는 고정 BiDi corpus" 중 고정 corpus).

## 남은 위험 / 한계

- 공식 `BidiCharacterTest.txt` 전체 적합성은 `unicode-bidi` crate 자체 테스트에 의존합니다. 이 crate가 L1·L2를
  다시 구현했으므로 참조 일치 검사 범위(corpus 항목)를 넘는 입력의 차이는 아직 검증하지 않았습니다.
- `cargo xtask validate-dependencies` 범위 문제(xtask 직접 의존성만 수집)는 X.9.1 보고서와 같습니다.
