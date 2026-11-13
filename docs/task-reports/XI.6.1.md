# 작업 보고서: XI.6.1 Shape/Path fill tessellation interface와 cache key

- WBS: XI.6.1
- 작업명: Shape/Path fill tessellation interface(`FillTessellator`)와 cache key(`TessellationKey`) 정의
- 상태: 완료 (`[x]`)
- 선행: VI.5.6, VI.6.1 (`develop`에서 완료: `1360761`, `c341dce`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/scene/src/geometry.rs`(신규) — Scene path 입력 `PathCommand`(MoveTo/LineTo/CubicTo/Close),
  `FillRule`, 검증된 `PathGeometry`(subpath 규칙, `Arc` 공유, canonical bytes), `PathGeometryError`.
- `crates/scene/src/lib.rs` — `geometry` 모듈 등록, renderer-core용 core-types 기본 타입
  (`Vec2`, `FiniteF32`, `NonFiniteError`) 재노출.
- `crates/renderer-core/src/tessellation.rs`(신규) — `FillTessellator` trait, `FillStyle`,
  `FlattenTolerance`, `PathKey`, `TessellationStyle`, `TessellationKey`, `FillMesh`,
  `TessellationCache`(`CacheOutcome`, `CachedMesh`, `CacheStats`), `TessellationError`.
- `crates/renderer-core/src/digest.rs`(신규) — crate 내부 FNV-1a 64 digest.
- `crates/renderer-core/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XI.6.1.md`(신규), 계획서(XI.6.1 `[ ]`→`[x]`).

## 구현 요약

- **입력 타입(scene)**: crate 의존 정책상 scene/renderer-core는 document를 import할 수 없으므로
  document path 모델(VI.6.1 네 가지 command, VI.6.2 subpath 규칙·winding)과 같은 의미의
  renderer 쪽 입력을 scene에 독립 정의했습니다. Shape는 document 단계의 순수 평가(VI.5.6)로
  path command가 된 뒤 Scene에 들어오므로 Shape·Path가 하나의 `PathGeometry` 입력을 공유합니다.
  canonical bytes는 `u64` LE 개수 + (tag + `f32` LE 좌표)*이며 `Vec2`가 `-0.0`을 정규화해
  bit-stable합니다.
- **interface**: `FillTessellator::tessellate_fill(&PathGeometry, &FillStyle) -> Result<FillMesh, _>`.
  worker 실행(XI.6.3)을 위해 `Send + Sync`이며 결정론·무 panic을 계약으로 명시했습니다.
  `FillMesh::new`는 index 3의 배수·정점 범위를 검증합니다.
- **cache key**: `TessellationKey = PathKey(canonical bytes + FNV-1a 64 digest) + TessellationStyle`.
  style에는 mesh를 바꾸는 값(fill rule, 평탄화 허용 오차)만 넣고 paint·opacity·transform은
  제외했습니다. `PathKey`의 `Hash`는 digest만 쓰고 `Eq`는 원본 bytes까지 비교해 digest 충돌로
  다른 path가 hit되지 않습니다. `TessellationStyle`은 enum이라 stroke(XI.6.2)가 variant를
  추가해 같은 cache를 공유할 수 있습니다.
- **cache**: `TessellationCache::fill`이 조회 → miss면 tessellation → `Arc<FillMesh>`로 저장합니다.
  실패 결과는 저장하지 않아 다음 조회에서 재시도합니다. hit/miss 통계를 제공합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-scene` | 2 passed, 0 failed |
| `cargo test -p stellar-renderer-core` | 9 passed, 0 failed |
| `cargo clippy -p stellar-scene --all-targets -- -D warnings` | 경고 0 |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-scene -- --check` | 통과 |
| `cargo fmt -p stellar-renderer-core -- --check` | 통과 |
| `cargo xtask validate-crate-graph` | exit 0 |

| 검사 | 결과 |
|---|---|
| 독립 생성한 동일 path + 동일 style 재조회 시 hit, 같은 `Arc`, tessellator 1회 호출(`same_path_and_style_hits_cache`) | 통과 |
| Shape 평가 결과와 동일 command의 작성 Path가 같은 entry 공유(`shape_evaluated_and_authored_path_share_entry`) | 통과 |
| 좌표·command 구조·fill rule·허용 오차 변경은 각각 miss(`path_or_style_change_misses`) | 통과 |
| `-0.0` 좌표 key 동일(`negative_zero_coordinates_share_key`), 강제 digest 충돌 시 key 불일치(`digest_collision_does_not_alias_entries`) | 통과 |
| 실패 결과 미저장·재시도(`failed_tessellation_is_not_cached`), 허용 오차·mesh 계약(`tolerance_and_mesh_contracts`) | 통과 |
| subpath 규칙 거부(`subpath_rules_are_enforced`), canonical bytes golden(`canonical_bytes_are_bit_stable`) | 통과 |

## 차이(작업 지시와 다른 점)

- 실제 fill tessellation 알고리즘(곡선 평탄화·삼각분할)은 구현하지 않았습니다. 이 작업은
  interface와 cache key 정의 범위이며, 테스트는 호출 횟수를 세는 fan tessellator로 검증합니다.
- 작업 worktree 기준 commit(`784fbe7`)에는 선행 VI.5.6·VI.6.1 코드가 없고 `develop`에만
  있습니다. renderer 계층은 document를 의존하지 않으므로 의미만 맞춰 독립 타입으로 정의했습니다.

## 남은 위험 / 한계

- 평탄화 허용 오차가 key에 그대로 들어가므로 zoom마다 허용 오차를 연속값으로 바꾸면 매번
  miss가 납니다. zoom 구간별 허용 오차 양자화 정책은 후속(XI.6.3, XVI.5)에서 정해야 합니다.
- cache에 예산·eviction이 없습니다(XVI.5.2 priority 정책에서 추가).
- scene `PathGeometry`와 document `PathData`의 의미 동등성은 변환 계층(Scene 생성)이 생길 때
  양쪽 fixture로 교차 검증해야 합니다.
