# 작업 보고서: X.7.2 bottom-up 측정과 top-down 배치 분리, 측정 cache key

- WBS: X.7.2
- 작업명: bottom-up intrinsic measurement와 top-down placement를 분리하고 cache key를 정의
- 상태: 완료 (`[x]`)
- 선행: X.6.4(`[x]`, 이 lane), X.1.4(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/measure.rs`(신규) — `MeasureKey`, subtree input hash(`subtree_hashes`, 반복 post-order),
  계산 간 측정 cache `MeasureCache`(현재 입력에 맞지 않는 entry 정리), 테스트.
- `crates/layout/src/engine.rs` — 측정 단계 `Engine::measure`(in-run memo + 선택적 계산 간 cache)·`compute_measure`·
  `call_measurer`, `compute_layout_cached`, `LayoutStats`, `LayoutRun`, 공통 진입 `run`. 배치 방문 수 집계.
- `crates/layout/src/input.rs` — `node_bytes`(node canonical bytes, key용).
- `crates/layout/src/lib.rs` — `measure` 모듈.
- `docs/task-reports/X.7.2.md`(신규), 계획서(X.7.2 `[ ]`→`[x]`).

## 구현 요약

- **분리**: 측정(bottom-up)은 node content 크기·baseline을 subtree 입력과 요청(축별 가용 content 크기, mode)만으로
  계산하고 배치·진단을 만들지 않습니다(constraint 진단은 배치 단계 `constraint::place`, HugContent 순환 진단은
  `arrange`에서만 생성 — X.6.5). 배치(top-down)는 부모가 정한 rect로 위치를 정하고 측정 결과만 읽습니다. 그래서
  측정 결과를 key로 cache해도 결과·진단이 같습니다.
- **in-run memo**: 같은 (node, 가용 크기, mode) 측정은 한 계산에서 한 번만 계산합니다(container extent 포함 — 이전에는
  다른 `ChildConstraint`가 같은 content box를 만들면 다시 측정했습니다). 고정 intrinsic은 측정으로 세지 않습니다.
- **cache key** `MeasureKey` = (node `StableId`, subtree input hash, 가용 크기, mode, 측정기 revision). subtree hash는
  node canonical bytes + 자식 수 + visual order 자식 subtree hash를 `LayoutInputHasher`(primary·check)로 post-order
  반복 계산합니다. 측정은 subtree 밖 값에 의존하지 않으므로(부모 문맥은 가용 크기로만 들어옴) key가 같으면 결과가
  같습니다. 측정기 revision이 바뀌면 key가 달라집니다.
- **계산 간 cache**: `compute_layout_cached(input, measurer, &mut MeasureCache) -> LayoutRun { result, stats }`.
  성공하면 현재 입력에 없는 node·subtree hash가 바뀐 node·다른 측정기 revision의 entry를 지워 cache 크기를 현재 입력의
  측정 수로 제한합니다. cache가 없으면 subtree hash를 계산하지 않습니다.
- **집계** `LayoutStats`: 실제 측정 수, in-run memo hit, cache hit, 측정기 호출 수, 배치 방문 node 수(X.7.4
  benchmark 입력).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | unit 138 + `layout_corpus` 2 + doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| Stack·Grid·text·image 혼합 입력: cache 없이도 요청별 측정기 호출 1회(중복 0), 첫 cached 계산의 memo hit > 0·호출 수 = 고유 요청 수, cache entry 수 = 측정 수, 같은 입력 두 번째 계산은 측정 0·측정기 호출 0·결과 bytes 동일(`same_input_never_measures_twice`) | 통과 |
| leaf text 하나 변경 → 측정기 호출은 그 node만, 형제·Grid subtree는 cache hit, 측정 수 감소, 결과 = cache 없는 계산, cache 크기 불변(옛 entry 정리), 측정기 revision 변경 → 측정 콘텐츠 5개 전부 재측정, 다른 입력 → 그 입력 entry만 남음(`leaf_change_remeasures_only_its_ancestor_path`) | 통과 |
| subtree hash는 바뀐 leaf와 조상만 변경·결정적, key가 가용 크기·mode·revision 구분(`subtree_hash_is_cache_key_for_descendant_changes_only`) | 통과 |
| 깊이 256 사슬에서 cache가 전용 stack thread 경로로 넘어가고 두 번째 계산 측정 0(`max_depth_chain_does_not_overflow_default_thread_stack`) | 통과 |
| 기존 fixture·X.11.1 corpus golden | 통과(결과 변경 없음) |

## 차이(작업 지시와 다른 점)

- 측정은 필요할 때(top-down 배치가 요청할 때) 계산하는 lazy 방식입니다. "bottom-up"은 자식 측정이 부모 측정으로
  합성되는 데이터 흐름과 subtree hash 계산 순서를 뜻하며, 모든 node를 미리 측정하는 eager pass는 두지 않았습니다
  (Fixed subtree처럼 측정이 필요 없는 node까지 방문하지 않기 위해서).
- 크기 해석(`resolve_size`) memo는 계산 안에서만 유지합니다. 계산 간 재사용은 측정 단계에 한정합니다. dirty set 기반
  부분 배치(X.7.3)는 선행 X.7.1(선행 VII.1.4 미완료) 때문에 이 lane에서 다루지 않았습니다.

## 남은 위험 / 한계

- subtree hash는 cache를 쓰는 계산마다 전체 입력을 한 번 hash합니다(입력 크기에 선형). 증분 갱신은 X.7.3 소관입니다.
- cache 항목 수는 현재 입력의 고유 측정 요청 수만큼입니다. 큰 문서에서 메모리 상한(LRU 등)은 아직 없습니다.
- 측정기 결과가 revision 없이 바뀌면(예: 외부 폰트 상태) cache가 오래된 값을 돌려줄 수 있습니다. 측정기 구현은
  규칙이 바뀔 때 `revision()`을 바꿔야 합니다.
