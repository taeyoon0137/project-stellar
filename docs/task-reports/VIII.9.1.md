# 작업 보고서: VIII.9.1 synthetic token/binding graph 생성기

- WBS: VIII.9.1
- 작업명: 100만 token/binding edge synthetic graph 생성기, 활성 document 한도 fixture는 20만 node 기준으로 별도 생성
- 상태: 완료 (`[x]`)
- 선행: VIII.6.4(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/bindings/src/synthetic.rs`(신규) — `SyntheticConfig`(preset `EDGES_1M`, `ACTIVE_DOCUMENT_200K`), `generate`,
  `SyntheticGraph`(`edge_count`, `node_count`, `digest`, `expression`, `materialize`), `SyntheticRef`,
  `SyntheticProducer`, `SyntheticError`, `MaterializeError`와 테스트.
- `crates/bindings/src/lib.rs` — `synthetic` 모듈 등록.
- `docs/task-reports/VIII.9.1.md`(신규), 계획서(VIII.9.1 `[ ]`→`[x]`).

## 구현 요약

- **결정론**: 고정 seed SplitMix64. 같은 설정·seed는 같은 graph이며 `digest`(FNV-1a 64, 필드 LE 순서)로 고정합니다.
- **비순환·중복 없음**: 토큰 `i`는 앞선 토큰만, binding `j`는 토큰·앞선 binding target 속성·환경 입력만 참조합니다.
  producer 하나의 참조 대상은 서로 다릅니다.
- **정확한 간선 수**: 토큰 참조 수(0~`max_token_refs`)를 먼저 정하고 남은 간선을 binding에 고르게(몫 + 앞쪽 나머지)
  나눕니다. binding 하나가 감당할 수 없는 설정은 `TooManyEdges`로 거부합니다.
- **preset**: `EDGES_1M` = 토큰 100,000 + binding 200,000, 간선 정확히 1,000,000. `ACTIVE_DOCUMENT_200K` = 토큰 20,000 +
  binding 179,997 + 환경 입력 3 = node 정확히 200,000, 간선 500,000.
- **실체화**: `materialize()`가 실제 `TokenRegistry`(프로젝트 scope)·`BindingSet`을 만듭니다. 값은 Number, 표현식은
  참조들의 `max`(참조 없으면 0~999 상수)라 깊은 graph에서도 유한합니다. 환경 입력은 Number key(`os_version`·
  `api_level`·`density`)만 씁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-bindings` | 종료 코드 0 |
| `cargo test -p stellar-bindings` | 첫 실행은 golden digest 자리(0)로 실패 → 실제 digest(`0x86FA_5952_3FE2_EEB3`, `0x44AA_C72B_4135_F204`)를 고정한 뒤 종료 코드 0 (51 unit passed) |
| `cargo clippy -p stellar-bindings --all-targets -- -D warnings` | 첫 실행 `manual_checked_division` 1건 → `checked_div`로 수정 후 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-bindings -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **seed-deterministic**(`presets_are_seed_deterministic_with_exact_sizes`): `EDGES_1M` 두 번 생성 결과 동일·digest 동일·golden 일치, 간선 정확히 1,000,000; `ACTIVE_DOCUMENT_200K` node 정확히 200,000·간선 500,000·golden 일치 | 통과 |
| 다른 seed는 digest가 다름, 모든 참조가 앞쪽을 가리킴·중복 없음, 과도한 간선 설정 거부(`different_seeds_differ_and_references_point_backwards`) | 통과 |
| 실체화한 graph의 간선·node 수가 spec과 일치, 비순환, 모든 node 평가 성공, 같은 spec은 같은 registry·binding(`materialized_graph_matches_spec_and_evaluates`, 소규모 설정) | 통과 |

## 차이(작업 지시와 다른 점)

- 1M 간선 preset은 debug 테스트에서 spec 생성·digest만 검사하고, 실체화(표현식 AST 생성)는 소규모 설정으로 검사합니다.
  대규모 실체화·평가 측정은 VIII.9.2 benchmark에서 release 빌드로 합니다.
- 신규 모듈이므로 테스트는 구현과 함께 작성했습니다(구현 전 상태는 컴파일 불가).

## 남은 위험 / 한계

- golden digest는 생성 규칙(난수 소비 순서·배치·digest 형식)을 고정합니다. 규칙을 바꾸면 golden을 의도적으로 갱신해야
  합니다.
