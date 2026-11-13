# 작업 보고서: XVI.3.1 1만·20만·프로젝트 100만 node synthetic fixture generator

- WBS: XVI.3.1
- 작업명: 1만, 20만, 프로젝트 전체 100만 node synthetic fixture generator — 파일을 Git에 두지 않고 seed+generator version으로 재현
- 상태: 완료 (`[x]`)
- 선행: VI.10.5 (`[x]`), V.6.4 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증. 대형 tier는 같은 장비에서 `--release`로 실행).

## 변경 파일 목록

- `benchmarks/fixtures/src/lib.rs`(수정) — crate 문서, `GENERATOR_VERSION`, 모듈 등록.
- `benchmarks/fixtures/src/rng.rs`(신규) — SplitMix64 PRNG.
- `benchmarks/fixtures/src/ids.rs`(신규) — 결정론적 StableId 발급기(`IdAllocator`).
- `benchmarks/fixtures/src/error.rs`(신규) — `FixtureError`.
- `benchmarks/fixtures/src/digest.rs`(신규) — fixture content hash(`FixtureDigest`).
- `benchmarks/fixtures/src/build.rs`(신규) — payload factory, `finish_document`(로드·검증·직렬화).
- `benchmarks/fixtures/src/synthetic.rs`(신규) — `ScaleTier`, `SyntheticSpec`, `TreeShape`, `generate_synthetic`.
- `benchmarks/fixtures/src/split.rs`(신규) — View subtree split 계획 요약(`summarize_split`).
- `benchmarks/fixtures/src/cli.rs`, `benchmarks/fixtures/src/main.rs`(신규) — on-demand 생성 CLI.
- `benchmarks/fixtures/tests/synthetic_scale.rs`(신규) — tier 검증.
- `docs/task-reports/XVI.3.1.md`(신규), 계획서(XVI.3.1 `[ ]`→`[x]`).

## 구현 요약

- **tier**(아키텍처 §5.3): `nodes-10k`(문서 1, 10,000), `nodes-200k`(활성 문서 상한 200,000),
  `project-1m`(문서 9개 = 200,000 1개 + 100,000 8개, 동시 열린 문서 상한 10 이내, 합 1,000,000).
  node 수는 정확히 맞춥니다(View별 예산 분배 + container 몫 분배의 합이 항상 예산과 같음).
- **실제 경로**: 영속 레코드를 조립한 뒤 `DocumentModel::from_records`(전체 validator VI.10.1)로
  로드하고 `stellar_project::document_io::serialize_document`로 chunk model을 만듭니다. 미해결
  참조가 하나라도 있으면 fixture를 거부합니다.
- **문서 모양**: 문서 → 페이지(20 View) → View root Frame(1440×1024) → Frame·Fragment container
  (fan-out 4..=12, depth ≤ 10) → Shape·Text·Path·Image·빈 Frame·빈 Fragment leaf. 일부 node는
  modifier stack(Fill·Stroke·Shadow·Opacity·Blur·Corner·Transform·Blend, 드물게 앞서 만든 node를
  가리키는 Mask)을 가집니다. 참조는 먼저 만든 node만 가리켜 DAG입니다. 좌표·크기는 1/4 px 정수
  격자라 f32로 정확히 표현되고 플랫폼 무관합니다.
- **재현**: fixture 정체는 `(GENERATOR_VERSION, fixture 이름, seed, 규모 매개변수)`입니다. content
  hash는 각 문서 chunk(논리 ChunkId·kind·schema version·payload 길이·payload SHA-256)와 프로젝트
  문서 index canonical bytes를 길이 프리픽스 record로 누적한 SHA-256입니다(ADR-0004 hash).
  생성 규칙이 바뀌면 `GENERATOR_VERSION`을 올리도록 문서화했습니다.
- **메모리**: 프로젝트 tier는 문서를 하나씩 만들어 hash에 넣고 `visit` 콜백에 넘긴 뒤 버리므로
  최대 메모리는 가장 큰 문서(20만 node) 수준입니다. 생성은 반복(비재귀)입니다.
- **V.6.4 연결**: `summarize_split`이 각 View subtree를 storage split planner에
  `config/performance-budgets.yaml` 임계치(256 KiB)와 subtree 인코딩 크기 모델로 넣어 계획합니다.
  기본 모양(View당 약 500 node)은 View subtree가 임계치 아래라 oversized chunk가 없습니다.
- **on-demand CLI**: `cargo run -p stellar-bench-fixtures --release -- <tier> [--seed N]`가 요약
  JSON 한 줄(문서·node·chunk 수, payload bytes, content hash, 소요 ms)을 출력합니다. 파일은 쓰지
  않습니다. 인자는 엄격하게 해석합니다(알 수 없는 인자·중복·잘못된 숫자 거부).
- 명세 검증: 빈 문서 목록, node 0, 문서 64개 초과, 총 500만 초과(overflow 포함), depth 1..=64 밖,
  잘못된 fan-out은 생성 전에 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-bench-fixtures` | 완료 |
| `cargo test -p stellar-bench-fixtures` | unit 10 passed, `synthetic_scale` 2 passed·2 ignored(대형 tier), 0 failed |
| `cargo test -p stellar-bench-fixtures --release --test synthetic_scale -- --ignored` | 2 passed(20만·100만 tier), 0 failed, 24.01s |
| `cargo clippy -p stellar-bench-fixtures --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-bench-fixtures -- --check` | 통과 |
| `git diff --check` | 통과 |
| `cargo run -q -p stellar-bench-fixtures --release -- nodes-10k --seed 1` (2회) | 두 번 모두 `sha256:f83d583f48c34d35ea1edbdca253f987d2e9827d78bbefdcc4f161154895680c`, 10,000 node, chunk 42, 1,617,153 bytes |
| `cargo run -q -p stellar-bench-fixtures --release -- project-1m` | 9 문서, 1,000,000 node, chunk 4,010, 160,958,942 bytes, 14,390 ms |

| 검사 | 결과 |
|---|---|
| 1만 tier: 정확한 node 수·모든 node 종류·modifier·참조 포함, loader round-trip 레코드 동일, split 계획 oversized 0, 같은 seed 재생성 hash 동일, 다른 seed hash 다름(`nodes_10k_tier_is_reproducible_valid_and_round_trips`) | 통과 |
| 축소 프로젝트 변형: 문서 사이 id 유일, 문서별 node 수 정확, hash 재현, 이름·규모 변경 시 hash 변경(`project_smoke_variant_keeps_ids_unique_across_documents`) | 통과 |
| 20만 tier: node 수, split oversized 0, hash 재현(`nodes_200k_tier_is_reproducible`, `--ignored`) | 통과 |
| 100만 tier: node 수·문서 수, hash 재현(`project_1m_tier_is_reproducible`, `--ignored`) | 통과 |
| tier 규모가 아키텍처 §5.3과 일치, 잘못된 명세 거부, CLI 인자 엄격 해석·JSON escape(unit) | 통과 |

## 차이(작업 지시와 다른 점)

- 20만·100만 tier 테스트는 debug 빌드에서 오래 걸려 `#[ignore]`로 두고, CI에는 같은 생성 경로의
  1만 tier와 축소 프로젝트 변형을 둡니다. 대형 tier는 위 표처럼 `--release -- --ignored`로 실행해
  확인했습니다.
- content hash golden 값은 테스트에 고정하지 않았습니다. hash가 문서 chunk 저장 형식에도 의존하므로,
  다른 작업의 정당한 저장 형식 변경이 이 crate 테스트를 깨지 않게 하려는 선택입니다. 대신 같은
  seed 재생성·별도 프로세스 실행에서 hash가 같음을 확인했습니다(위 CLI 2회 실행).

## 남은 위험 / 한계

- 문서 serializer는 아직 View 하나를 subtree chunk 하나로 저장합니다. split planner 계획은 요약으로만
  보고하며 serializer 출력에 반영되지 않습니다(serializer 쪽 split 적용은 해당 작업 몫).
- ComponentInstance는 영속 레코드 payload variant가 없어 이 문서 fixture에 들어가지 않습니다.
  컴포넌트 규모 fixture는 XVI.3.2에서 IX.2 scene 표현으로 만듭니다.
- 문서 chunk 저장 형식이 바뀌면 같은 seed의 content hash도 바뀝니다. 측정 결과에는 hash와 함께
  generator version과 commit을 기록해야 합니다(XVI.4.5).
