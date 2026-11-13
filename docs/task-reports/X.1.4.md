# 작업 보고서: X.1.4 LayoutRevision과 node별 input hash/output cache 타입

- WBS: X.1.4
- 작업명: `LayoutRevision`과 node별 input hash/output cache 타입 정의
- 상태: 완료 (`[x]`)
- 선행: II.3.4(revision newtype, `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/revision.rs`(신규) — `LayoutRevision` newtype, `checked_next`, `Display`, 문서·scene revision 혼용 `compile_fail` doctest.
- `crates/layout/src/cache.rs`(신규) — `LayoutInputHash`, `LayoutInputHasher`(FNV-1a 64), `LayoutCacheEntry<T>`, `CacheMissReason`, `CacheLookup<'a, T>`, `LayoutCacheStats`, `LayoutCacheError`, `LayoutCache<T>`, 단위 테스트.
- `crates/layout/src/lib.rs` — `pub mod revision;`, `pub mod cache;` 선언, crate 문서 갱신.
- `docs/task-reports/X.1.4.md`(신규), 계획서(X.1.4 `[ ]`→`[x]`).

## 구현 요약

- **`LayoutRevision(pub u64)`**: layout pass가 결과를 확정한 번호입니다. II.3.4의 `DocumentRevision`·
  `SceneRevision`과 다른 타입이라 비교·대입이 컴파일 단계에서 실패합니다(`compile_fail` doctest 2개).
  `checked_next`는 `u64` 상한에서 감싸지 않고 `None`을 반환합니다.
- **`LayoutInputHash` / `LayoutInputHasher`**: node 하나의 layout 입력을 64비트로 요약합니다. 알고리즘은
  FNV-1a 64로 고정했고, 다중 바이트 값은 little-endian, float은 `FiniteF32` 비트(`-0.0`→`+0.0`),
  가변 길이 바이트열은 `u64` LE 길이 프리픽스로 공급합니다. 같은 입력·같은 필드 순서는 플랫폼과
  무관하게 같은 hash가 됩니다. 런타임 cache key 전용이며 저장용 `ContentHash`(V.1.5)를 대신하지
  않습니다.
- **`LayoutCache<T>`**: node(`StableId`)별 entry(입력 hash, `LayoutRevision`, output) 하나를 보관합니다.
  `lookup`은 입력 hash가 같으면 `Hit`, 다르면 `Miss(InputChanged { cached })`, entry가 없으면
  `Miss(NotCached)`를 반환하고 hit/miss를 포화 집계합니다. `insert`는 기존 entry보다 오래된 revision을
  `LayoutCacheError::RevisionRegressed`로 거부하고 cache를 바꾸지 않습니다. output 타입은 generic이라
  `LayoutResult`(X.1.1) 정의 전에 구조를 선점하지 않습니다. 순회 API를 두지 않아 `HashMap` 순서가
  결과에 영향을 주지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 종료 코드 0 (15 unit passed + 2 compile_fail doctest passed, 0 failed) |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **동일 입력 hash에서 cache hit**(`same_input_hash_hits_cache`: 같은 입력을 다시 hash → 같은 값 → `Hit`, output·revision 일치, stats hits=1) | 통과 |
| 입력 변경 시 `Miss(InputChanged { cached })`(`changed_input_misses_with_cached_hash`) | 통과 |
| 미캐시 node `Miss(NotCached)`, node별 entry 분리 | 통과 |
| revision 역행 저장 거부·기존 entry 유지, 같은 revision 재저장 허용 | 통과 |
| FNV-1a 64 공개 test vector(빈 입력, `"a"`, `"foobar"`) golden | 통과 |
| hasher 결정성·필드 순서 민감성·`-0.0`=`0.0`·길이 프리픽스 경계 구분 | 통과 |
| `LayoutRevision`↔`DocumentRevision` 비교, `SceneRevision` 대입 `compile_fail` | 통과(컴파일 실패 확인) |

- `compile_fail` doctest가 import 오류가 아니라 타입 불일치로 실패하는지 확인하려고, 같은 import로
  `.0` 값만 비교하는 양성 doctest를 임시로 추가해 통과(컴파일 성공)를 확인한 뒤 제거했습니다.

## 차이(작업 지시와 다른 점)

- node 식별자는 영속 `StableId`를 사용했습니다. 런타임 `RuntimeEntityId` 기반 key가 필요하면 X.7.2
  cache key 정의 시 확장합니다.
- revision 역행 거부는 cache 단위 단조성 검사입니다. `DocumentRevision`·입력 hash 기반 worker result
  guard는 X.7.5 소관이라 구현하지 않았습니다.

## 남은 위험 / 한계

- 64비트 hash라 이론적 충돌 가능성이 있습니다. 충돌 시 잘못된 cache hit가 날 수 있으므로, 정확성이
  더 필요하면 X.7.2에서 key에 입력 요약 외 식별 정보를 추가하거나 비교 검증을 붙여야 합니다.
- hash에 넣을 필드와 순서는 호출자(X.7.x 측정·배치 구현)가 고정해야 합니다. 이 작업은 hasher·cache
  타입만 정의합니다.
