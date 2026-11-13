# 작업 보고서: XVI.3.3 text·image·effect·token·constraint-heavy fixture 분리

- WBS: XVI.3.3
- 작업명: text-heavy, image-heavy, effect-heavy, token-heavy, constraint-heavy fixture를 분리하고 목표 병목 특성을 계측값으로 확인
- 상태: 부분 완료 (`[~]`) — text·image·effect·constraint-heavy는 구현·계측 완료, token-heavy는 토큰 모델(VIII.5)이 없어 만들 수 없음
- 선행: Phase X, Phase XI (Phase 단위 선행은 완화 적용: 필요한 레이아웃 엔진(X)과 문서 modifier·이미지·텍스트 payload가 존재해 진행. Phase XI 렌더러 계측은 이 작업에 쓰지 않음)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `benchmarks/fixtures/src/workload.rs`(신규) — `DocumentWorkload`, `workload_document`, `DocumentProfile`,
  `profile_document`, `constraint_heavy`, `ConstraintProfile`, `profile_constraints`, `UNAVAILABLE_WORKLOADS`.
- `benchmarks/fixtures/src/error.rs`(수정) — `Layout` 오류 추가.
- `benchmarks/fixtures/src/cli.rs`(수정) — `text-heavy`, `image-heavy`, `effect-heavy`, `constraint-heavy` 추가(기준 문서 계측값 동봉).
- `benchmarks/fixtures/src/lib.rs`(수정) — `workload` 모듈 등록.
- `benchmarks/fixtures/Cargo.toml`(수정) — 내부 crate `stellar-layout` path 의존 추가.
- `Cargo.lock`(수정, 생성 결과) — `stellar-bench-fixtures` 의존 목록에 `stellar-layout` 한 줄. 외부 crate 변경 없음.
- `benchmarks/fixtures/tests/workload_fixtures.rs`(신규) — 계측값 검증.
- `docs/task-reports/XVI.3.3.md`(신규), 계획서(XVI.3.3 `[ ]`→`[~]`).

## 구현 요약

- **분리**: workload마다 별도 생성기·별도 content hash입니다. 문서 workload는 페이지 → View(약 250
  node) → root Frame → section Frame(leaf 24개) → leaf 모양이고, 실제 문서 모델·전체 validator·
  serializer를 거칩니다(미해결 참조 0).
  - text-heavy: leaf 7/8이 40..=200 단어 다문단(12 단어마다 줄바꿈) Text, style run 4..=16개.
  - image-heavy: leaf 7/8이 Image, 이미지마다 서로 다른 asset, 배치 상자 512..=4096 px, fit·crop·sampling 다양.
  - effect-heavy: 모든 Frame·leaf가 Shadow·Blur·Blend·Opacity·StrokeExpand·Stroke 3..=6개 stack.
  - constraint-heavy: 문서 레코드에 constraint 필드가 없어 레이아웃 엔진 입력(`stellar_layout::LayoutInput`)으로
    만듭니다. root(Constraint) ← panel(Constraint, 부모 기준 위치) ← item 32개 chain(첫 item은 부모 top,
    다음 item은 앞 item bottom에 sibling constraint, 너비 `Fill` + Left·Right 두 등식).
- **계측값**(생성 규칙이 아니라 결과에서 셈): `profile_document`는 영속 레코드와 canonical 인코딩에서 text
  scalar·문단·style run, Image 수·서로 다른 asset·배치 면적, modifier·effect modifier·geometry 무효화
  modifier(VI.9.7 registry), 종류별 payload bytes를 셉니다. `profile_constraints`는 등식 수, sibling 참조
  union-find 연결 요소 크기를 세고 `compute_layout`으로 실제 배치해 box 수·진단 수를 얻습니다.
- **token-heavy**: 토큰 모델·registry·dependency graph(VIII.5.1~VIII.5.3)가 `[ ]`라 토큰을 담을 원본
  표현이 없습니다. 임의 표현을 만들지 않고 `UNAVAILABLE_WORKLOADS`에 이유와 함께 명시했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-bench-fixtures` | 완료 |
| `cargo test -p stellar-bench-fixtures` | unit 10, `limit_fixtures` 3, `synthetic_scale` 2(+2 ignored), `workload_fixtures` 5 passed, 0 failed |
| `cargo clippy -p stellar-bench-fixtures --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-bench-fixtures -- --check` | 통과 |
| `cargo check -p stellar-bench-fixtures --locked` | 통과(lockfile 일치) |
| `cargo run -q -p xtask -- validate-dependencies` / `validate-crate-graph` | 둘 다 exit 0 |
| `git diff --check` | 통과 |

10,000 node 규모 계측값(`cargo run -q -p stellar-bench-fixtures --release -- <workload>`, seed 1, 기준 = 같은
규모·seed의 synthetic 문서):

| workload | 계측값 | 기준 문서 |
|---|---|---|
| text-heavy | Text 8,381개, text scalar 5,238,513(node당 523.9), style run 83,689, Text payload 9,878,681 / 9,931,825 bytes(99.5%) | Text 1,876개, scalar 62,106(node당 6.2), Text payload 157,594 / 807,118 bytes(19.5%) |
| image-heavy | Image 8,339개(83.4%), 서로 다른 asset 8,339, 배치 면적 44,175,046,384 px² | Image 758개(7.6%), asset 32, 42,312,089 px² |
| effect-heavy | modifier 45,017, effect modifier 37,853(node당 3.79), geometry 무효화 31,169, modifier bytes 2,027,132 | modifier 3,838, effect 1,421(node당 0.14), geometry 2,171, 204,549 bytes |
| constraint-heavy | constraint 노드 9,999, 등식 29,694(node당 2.97), sibling 등식 18,786, 최대 연결 요소 32, 배치 box 10,000, 진단 0 | (문서에 constraint 없음) |

| 검사 | 결과 |
|---|---|
| text-heavy: node당 문자 수 ≥ 기준 10배, Text bytes 비중 ≥ 80%, Text당 style run ≥ 4·문단 ≥ 3, seed 재현(`text_heavy_dominates_text_volume`, 5,000 node) | 통과 |
| image-heavy: Image 비중 ≥ 75%·기준 5배 이상, asset 수 = Image 수·기준 10배 이상, 이미지당 512² px 이상, seed 재현(`image_heavy_dominates_image_count_assets_and_area`) | 통과 |
| effect-heavy: node당 effect ≥ 2·기준 10배 이상, geometry 무효화 modifier ≥ node 수, modifier bytes > node payload bytes, seed 재현(`effect_heavy_dominates_effect_modifiers`) | 통과 |
| constraint-heavy: root 외 전부 constraint 배치, node당 등식 ≥ 2.5, 연결 요소 ≥ 32, 엔진 배치 box = node 수, 진단 0, seed 재현(`constraint_heavy_is_solved_by_layout_engine_with_long_chains`) | 통과 |
| token-heavy 불가 사유 명시(`token_heavy_is_reported_as_unavailable`) | 통과 |

## 차이(작업 지시와 다른 점)

- token-heavy fixture는 만들지 않았습니다(위 사유). 그래서 상태는 `[~]`입니다.
- constraint-heavy는 문서 chunk model이 아니라 레이아웃 엔진 입력입니다. 문서 레코드(VI.10)에 layout·
  constraint 필드가 아직 없기 때문입니다.
- 계측값은 결정론적 구조 계측(개수·bytes·연결 요소)과 레이아웃 엔진 실제 실행 결과입니다. 시간 측정은
  benchmark 단계(XVI.2·XVI.4)에서 이 fixture를 입력으로 합니다.
- 이 crate가 레이아웃 엔진을 쓰도록 내부 path 의존 `stellar-layout`을 추가해 `Cargo.lock`의 이 crate 항목이
  한 줄 바뀌었습니다(생성 결과, 외부 의존성 변화 없음). ADR-0019는 외부 crate만 별도 ADR을 요구합니다.

## 남은 위험 / 한계

- token-heavy: VIII.5(토큰 모델)와 VIII.6(binding graph)이 구현되면 토큰 수·의존 edge·propagation fan-out을
  계측하는 fixture를 추가해야 합니다(VIII.9.1의 100만 token/binding edge 생성기와 범위를 맞춰야 함).
- effect·image 병목의 실제 GPU 비용(blur pass 수, 텍스처 업로드 bytes)은 렌더러(Phase XI) 계측에서 확인해야
  합니다. 이 fixture는 문서 수준 특성만 계측합니다.
- text-heavy View 하나의 subtree payload는 약 250 KB라 split 임계치(256 KiB) 근처입니다. 저장 경로 분할
  benchmark에서는 의도된 부하입니다.
