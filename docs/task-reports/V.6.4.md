# 작업 보고서: V.6.4 chunk 분할 임계치 기록과 split planner

- WBS: V.6.4
- 작업명: 승인 임계치(ADR-0012, 262,144 bytes)를 `config/performance-budgets.yaml`에 기록하고
  의미 경계 기반 split planner 구현
- 상태: 완료 (`[x]`)
- 선행: V.6.3, ADR-0012 승인(Accepted 2026-11-14)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `config/performance-budgets.yaml` — `storage.chunk_split_threshold_bytes: 262144` 추가, 머리말
  항목 목록 갱신.
- `crates/storage/src/chunk/budget.rs`(신규) — `ChunkBudget`, `ChunkBudgetError`, 엄격한
  `storage:` 절 reader.
- `crates/storage/src/chunk/split.rs`(신규) — `SplitNode`, `SplitTree`, `ChunkCostModel`,
  `SplitPolicy`, `PlannedChunk`, `ChunkLayout`, `SplitPlan`, `plan_chunk_split`,
  `materialize_layout`.
- `crates/storage/src/chunk/mod.rs` — `budget`, `split` 모듈 등록.
- `docs/adr/ADR-0012-chunk-split-threshold.md` — "V.6.4 구현 기록" 추가(key 이름·위치, 비교 규칙,
  oversized 처리). 보호 경로 변경과 같은 커밋입니다.
- `docs/task-reports/V.6.4.md`(신규), 계획서(V.6.4 `[ ]`→`[x]`).

## 구현 요약

- **예산 기록**: key는 `config/performance-budgets.yaml`의 최상위 `storage:` 절
  `chunk_split_threshold_bytes`입니다. 기존 color reader(core-types 테스트)는 접두어만 보는
  느슨한 reader라서, storage에 엄격한 reader를 새로 두었습니다. 다른 최상위 절은 건너뛰고
  `storage:` 절에서는 절·key 누락, 중복, 알 수 없는 key, 공백 2칸이 아닌 들여쓰기·tab·중첩,
  10진 양의 정수가 아닌 값(부호·밑줄·단위·소수·선행 0·`u64` 초과)을 거부합니다.
- **입력 모델**: storage는 문서 노드 타입을 모르므로, 문서 계층이 노드마다 StableId·부모·의미
  경계 여부·`own_bytes`를 담은 `SplitTree`를 넘깁니다(부모가 자식보다 앞). payload 크기는 가산
  모델 `chunk_base_bytes + Σ own_bytes + child_ref_bytes × 분리된 자식 수`로 계산합니다.
- **split 규칙(결정론)**: 역순으로 한 번 순회하며 tree root와 의미 경계 노드에서만 판정합니다.
  region이 `> threshold`이면 region 안의 가장 가까운 의미 경계 자손을 region이 큰 순서(같으면 입력
  순서)로 분리하고, 임계치 이하가 되면 멈춥니다. 분리해도 줄지 않는 후보(region ≤ 참조 비용)는
  분리하지 않습니다.
- **임계치보다 큰 단일 노드**: 바이트 단위로 쪼개지 않습니다. 의미 경계로 더 나눌 수 없는 region은
  분리 가능한 후보를 모두 분리해 범위를 최소화한 뒤 chunk 하나로 두고 `oversized = true`로
  표시합니다. 오류가 아니며 저장을 막지 않습니다. 큰 노드가 의미 경계 안에 있으면 그 경계가 부모
  판정에서 가장 큰 후보가 되어 별도 chunk로 격리됩니다.
- **저장 중 병합 금지(V.6.5와의 경계)**: 이전 layout의 chunk root를 그대로 유지합니다(sticky).
  삭제됐거나 더 이상 의미 경계가 아닌 이전 root만 부모로 흡수하고 `released_roots`로 반환합니다.
  결과 layout을 다시 넘기면 같은 layout이 나옵니다(고정점).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-storage` | 적용 |
| `cargo test -p stellar-storage` | 212 passed(신규 20) + 통합 1 passed + doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo test -p stellar-core-types color` | 17 passed(공유 예산 파일의 color reader 영향 없음) |
| `cargo xtask validate-adr <이 커밋 경로>` | exit 0 |
| `cargo xtask sync-plan --check` | exit 0 |
| `git diff --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 승인 임계치(저장소 예산 파일에서 읽음)로 payload == 262,144는 분할 없음, 262,145는 가장 큰 의미 경계 자식만 분리(`threshold_boundary_fixtures_split_as_expected`) | 통과 |
| 의미 경계가 아닌 Group 아래 후보, 가장 가까운 경계 우선, 자손 선판정, 동률 입력 순서 | 통과 |
| oversized: Frame 안 큰 leaf 격리, 큰 의미 경계 노드 단독 chunk, root region 초과 시 후보 전부 분리, 이득 없는 후보 미분리 | 통과 |
| 이전 root 유지·release(삭제·비경계), tree root 무시 | 통과 |
| pseudo-random tree 60개 × 임계치 3종: layout 불변식(소속 유일·연결·root는 경계·payload 재계산 일치), oversized 비축소성, 결정론, 고정점 | 통과 |
| 예산 reader: 저장소 값 262,144, 다른 절 무시, 누락·중복·형식·값 오류 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 저장소에 엄격한 예산 reader가 없어 지시대로 storage에 최소 reader를 추가했습니다.

## 남은 위험 / 한계

- planner는 storage 내부 알고리즘입니다. 문서 serializer(`crates/project` `document_io`, 현재 View당
  subtree chunk 1개)가 `SplitTree`를 만들고 layout대로 subtree chunk와 ViewTree 참조를 기록하는
  연결은 이 작업 범위(storage)가 아닙니다. 후속 작업이 필요합니다.
- 가산 크기 모델이 실제 payload 길이와 같으려면 문서 계층이 노드 record를 독립적으로 인코딩하고
  `own_bytes`·`chunk_base_bytes`·`child_ref_bytes`를 정확히 넘겨야 합니다. 연결 시 실제 길이와
  일치하는지 검증이 필요합니다.
- ADR-0010 "정하지 않은 사항"은 아직 ADR-0012를 Proposed로 적고 있습니다. 이번 허용 경로가 아니라
  수정하지 않았습니다.
- 임계치 근거의 한계(1회 측정, 합성 데이터)는 ADR-0012 Consequences에 기록된 그대로입니다.
