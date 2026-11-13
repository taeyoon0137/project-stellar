# 작업 보고서: X.11.1 레이아웃 fixture schema와 runner

- WBS: X.11.1
- 작업명: Free, Stack, Grid, Overlay, Constraint 각각 최소 50개 fixture를 생성하는 schema와 runner 작성
- 상태: 완료 (`[x]`)
- 선행: X.2~X.6(섹션 단위 — 완화, 아래 차이 참고)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/corpus.rs`(신규) — fixture schema(`LayoutFixture`, `CorpusMode`, `validate_fixture`, `validate_corpus`, `FixtureSchemaError`), 결정적 생성기(`generate_corpus`, SplitMix64), runner(`run_fixture`, `run_corpus`, `ModeSummary`, `corpus_input_digest`), schema 위반 테스트.
- `crates/layout/tests/layout_corpus.rs`(신규) — corpus count·schema·결정성·golden 비교, 모든 fixture의 box 수·왕복·반복 동일성 검사.
- `crates/layout/tests/fixtures/layout_corpus_v1.golden`(신규, 12줄) — 입력 hash와 배치 방식별 요약.
- `crates/layout/src/lib.rs` — `pub mod corpus;`.
- `docs/task-reports/X.11.1.md`(신규), 계획서(X.11.1 `[ ]`→`[x]`).

## 구현 요약

- **schema v1**: 이름 `"<mode>-<3자리>"`·유일, root container = mode, `LayoutInput::validate` 통과, canonical bytes 왕복
  동일, node ≤ 64, 배치 방식별 50개 이상.
- **생성기**: 고정 seed SplitMix64로 배치 방식별 입력을 만듭니다. 길이는 1/4 px 배수, StableId·OrderKey도 seed에서
  결정, 자식 삽입 순서와 node 목록 순서를 섞어 visual order·입력 순서 무관성을 함께 검사합니다.
  - Free: anchor(시작·끝·양끝+Fill, 길이·백분율 offset), Fixed·Hug(intrinsic)·Fill·Percentage, 일부 중첩 container.
  - Stack: axis·direction·gap·main/cross·wrap·cross gap·line 정렬, grow·shrink·basis.
  - Grid: Fixed·fr·MinContent·MaxContent·MinMax track, 명시·자동·열 고정 item, 암시 행.
  - Overlay: 축별 정렬과 다양한 sizing.
  - Constraint: parent·이전 sibling(Right·CenterY·Baseline) 참조, Fill 양끝 등식, 일부 underdetermined·미해결 sibling(진단).
  - 공통: hidden 자식, transform, min/max, aspect ratio 일부 포함(임시 측정: hidden 86, wrap Stack 20, 자동 grid item 72,
    min/max 지정 316, flex 지정 94, aspect ratio 63 node).
- **runner**: fixture별 결과 bytes hash와 배치 방식별 fixture·box·진단 수, digest 묶음을 golden과 비교합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 118 unit + 2 integration(layout_corpus) + 2 doctest passed, 0 failed |
| `cargo test -p stellar-layout --release --test layout_corpus` | 2 passed(최적화 빌드에서도 같은 golden) |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| fixture count: Free·Stack·Grid·Overlay·Constraint 각 50개(총 250), schema 검증 통과, 재생성 결과 동일, golden(입력 hash + 5개 배치 방식 요약) 일치(`corpus_count_schema_and_golden`) | 통과 |
| 250개 fixture 모두 오류 없이 계산, box 수 = 표시 node 수, 결과 bytes 왕복·반복 계산 동일, Constraint 진단 35건 발생(`every_fixture_lays_out_all_visible_nodes`) | 통과 |
| schema 위반 거부: 이름 형식, mode 불일치, 이름 중복, 50개 미만, 잘못된 입력(`corpus::tests::schema_rejects_malformed_fixtures`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 "X.2~X.6"은 섹션 단위라 사용자 지시(2026-11-14)에 따라 완화했습니다. corpus가 쓰는 Free·Stack·Grid·Overlay·
  Constraint·sizing 코드(X.2.1~X.6.3)는 이 lane에서 모두 구현되었고, 미완료인 X.6.4·X.6.5(intrinsic 측정 trait·Hug
  cycle, 선행 IX.2.2 미완료)는 corpus에 필요하지 않습니다(intrinsic 콘텐츠는 입력의 고정 값).
- golden은 생성 결과 요약(hash) 형태의 작은 파일입니다. 개별 rect 기대값은 각 작업의 손계산 fixture 테스트가 고정합니다.

## 남은 위험 / 한계

- golden은 이 엔진의 현재 결과를 고정한 회귀 기준입니다(독립 reference 구현과의 비교가 아님).
- 양 플랫폼 비교(X.11.3)는 선행 X.10.7 미완료이며 CI 변경 권한 밖이라 하지 않았습니다. 값은 정수 subpixel 연산과
  IEEE 기본 연산만 쓰므로 같은 golden이 Windows에서도 성립해야 하지만 실행 증거는 없습니다.
