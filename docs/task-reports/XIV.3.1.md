# 작업 보고서: XIV.3.1 SemanticSnapshot StableId index

- WBS: XIV.3.1
- 작업명: `SemanticSnapshot`을 프로젝트 metadata, documents, nodes, properties, tokens, components, assets refs의 StableId index로 정의(runtime cache 제외)
- 상태: 완료 (`[x]`)
- 선행: VI.10.4 `[x]`, XIV.2.6 `[x]`

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-diff/Cargo.toml` — document `test-support` feature를 `[dev-dependencies]`에서만 켬.
- `crates/semantic-diff/src/lib.rs` — module 선언, `core_types` 재노출.
- `crates/semantic-diff/src/value.rs`(신규) — `SemanticValue`, `ValueType`, canonical bytes·비교.
- `crates/semantic-diff/src/keys.rs`(신규) — snapshot 속성 key(문서 key 재사용 + 전용 대역).
- `crates/semantic-diff/src/snapshot.rs`(신규) — `ObjectKind`, `Origin`, `SemanticObject`, `SemanticSnapshot`, `SnapshotError`.
- `crates/semantic-diff/src/build.rs`(신규) — `SnapshotBuilder`, `ProjectMetadata`, `TokenEntry`, `AssetEntry`.
- `docs/task-reports/XIV.3.1.md`(신규), 계획서(XIV.3.1 `[ ]`→`[x]`).

## 구현 요약

- **모델**: snapshot은 `StableId → SemanticObject` BTreeMap입니다. 객체는 종류(`ObjectKind`:
  Project·Document·Page·View·Node(kind)·Modifier(kind)·Component·ComponentSlot·Token·Asset·
  ExpressionNode·Variant, 안정 numeric code), 범위(`Origin`: Project/Document/Component),
  위치(parent·OrderKey), `PropertyPath → SemanticValue` 속성 map을 가집니다. 속성 경로 root는
  소유 객체여야 합니다(`PropertyRootMismatch`).
- **값**: `SemanticValue`는 Bool·Uint·Text·Id·Ids·Floats(FiniteF32)·Typed(TypedValue)·Bytes의
  닫힌 집합이며 canonical LE bytes 형식을 가집니다. 재귀 구조가 없습니다.
- **구성**: `SnapshotBuilder`가 `DocumentRecords`(정의·node·modifier), `ComponentLibrary`(정의
  resource·정의 tree node·Slot, nested instance override 포함), 프로젝트 metadata, token, asset을
  받아 객체로 펼칩니다. node payload는 필드별 속성(Frame 7종은 문서 key `0x01xx` 재사용,
  Shape·Image 필드, Modifier는 `0x10xx` 재사용)으로, Path·Text는 내부 offset·point id가 묶여
  있어 payload canonical 인코딩 하나로 담습니다. 각 `add_*`는 원자적입니다.
- **asset refs index**: `asset_references()`가 asset StableId → 참조 속성 경로 집합을 반환합니다.
- **runtime cache 제외**: RuntimeEntityId·평가·layout cache를 담을 필드가 없고, 입력도 영속
  레코드뿐입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-diff` | 14 passed, 0 failed |
| `cargo fmt -p stellar-semantic-diff -- --check` | 통과 |
| `cargo clippy -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |
| `cargo test -p stellar-document production_code_does_not_use_test_support` | 2 passed(test-support는 dev-dependencies에서만) |
| `cargo run -q -p xtask -- validate-crate-graph` | exit 0 |

| 검사 | 결과 |
|---|---|
| HEAD/Index/WT fixture 3개 build, 각 snapshot이 View subtree chunk bytes 경유 재구성과 동일, staged/unstaged 차이 정확(`head_index_working_tree_snapshots_build`) | 통과 |
| 20 seed random document: 레코드 역순·역순 reload 모델(다른 runtime 이력)·chunk 경유 snapshot 동일(`record_order_and_runtime_history_do_not_matter`) | 통과 |
| 프로젝트 metadata·token·asset·컴포넌트 정의 tree 객체 구성, 중복 원자 거부(`project_tokens_assets_and_components`, `duplicate_metadata_names_are_rejected`) | 통과 |
| `ObjectKind` code 유일·round-trip, 속성 root 검사, 자식 `(OrderKey, StableId)` 순서 | 통과 |
| `canonical_eq` ⇔ canonical bytes 동일, bit-different float 구분 | 통과 |

## 차이(작업 지시와 다른 점)

- semantic-diff는 project·git-porcelain crate에 의존할 수 없어(II.2.8 그래프) Git materialize
  결과를 직접 읽지 않습니다. fixture는 HEAD/Index/WT 레코드를 View subtree chunk bytes로
  직렬화·복원하는 경로로 "materialized blob → loader → snapshot"을 모사했습니다. 실제 Git
  snapshot 연결은 Status 분류(XIV.4.1) 통합에서 project crate가 builder를 호출해 수행합니다.
- token 모델(VIII.5)이 아직 없어 `TokenEntry`는 이름·타입 id·literal 값·expression root만
  담는 입력 형식입니다. VIII.5.1이 정해지면 caller가 그 필드를 옮겨 담습니다.
- 문서 crate에 key가 없는 필드(공통 node 필드, Shape·Path·Text·Image 일부, 정의, token,
  asset)는 snapshot 전용 대역 `0xE0xx`~`0xEFxx`를 썼습니다. snapshot은 저장하지 않으므로 저장
  포맷 계약이 아니며, 문서 crate가 공식 key를 정하면 교체합니다.

## 남은 위험 / 한계

- 페이지·View·컴포넌트 Property/Slot 목록은 OrderKey 없는 순서 목록 하나의 속성이라, 양쪽이
  동시에 페이지를 추가하면 같은 속성 변경으로 비교됩니다(병합 시 conflict). 정의 모델에
  OrderKey가 생기면 객체 위치로 옮길 수 있습니다.
- scene ComponentInstance node는 `DocumentRecords`가 아직 표현하지 못합니다(문서 crate 한계).
  scene instance 입력은 XIV.3.6에서 별도 builder 입력으로 다룹니다.
