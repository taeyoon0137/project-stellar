# 작업 보고서: IX.7.3 VariantOverrideSet

- WBS: IX.7.3
- 작업명: VariantOverrideSet — sparse property patches, node add/delete/move/replace, slot changes
- 상태: 완료 (`[x]`)
- 선행: VI.2, IX.3(section 단위 선행 — 완화 적용, 아래 "차이" 참조)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/variant_override.rs`(신규) — `VariantOverrideSet`, `StructurePatch`,
  `VariantNode`, `SlotChange`, `VariantOverrideError`, `validate_references`, encode/decode.
- `crates/document/src/component/mod.rs` — `pub mod variant_override;` 등록.
- `docs/task-reports/IX.7.3.md`(신규), 계획서(IX.7.3 `[ ]`→`[x]`).

## 구현 요약

- **희소 patch**: `PropertyPath → TypedValue` property patch, node별 구조 patch 하나(`StableId →
  StructurePatch`: `Add{node, parent, order_key}`·`Delete`·`Move{parent, order_key}`·`Replace{node}`),
  `(인스턴스, slot) → SlotChange`(`Set(SlotOverride)`·`Reset`). 모든 구조 override는 StableId 기반이며
  map 오름차순으로 canonical 직렬화합니다.
- **reference validation**(`validate_references(base, view_root, library)`): base View 구조(scene,
  `view_root` subtree)에 대해 최종 구조(추가·이동·삭제·교체 반영)를 계산해 검사합니다.
  - 구조: 대상이 View 안 base node인지, root 변경 금지, 새 id 중복·key 불일치, 부모가 최종 구조에 살아
    있는지(삭제·교체된 subtree 아래 금지), 부모 종류(Frame·Fragment·ComponentInstance), cycle.
  - property patch: 대상이 최종 구조에 살아 있는지(삭제·교체 subtree 제외, 이동해 나온 node는 유효).
  - slot change: 대상이 최종 구조의 ComponentInstance인지(같은 set이 추가한 인스턴스 포함), 컴포넌트가
    slot을 선언했는지, Set content가 최종 구조에서 그 인스턴스 소유(부모 = 인스턴스)인지(IX.3.2 규칙).
- **직렬화**: canonical LE. PropertyPath는 root·key 목록·tail(index/key)을 LE로 기록합니다. 비오름차순·
  알 수 없는 tag·잘못된 값·trailing bytes를 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 184 passed, 통합 4 passed, doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 모든 patch 종류를 담은 set round-trip, 재인코딩 byte-identical, 설정 순서 무관, 빈 set·Reset(`variant_override_set_round_trips`) | 통과 |
| 절단·trailing·알 수 없는 tag·비오름차순 거부(`malformed_bytes_are_rejected`) | 통과 |
| 추가·삭제·이동·교체·slot change가 섞인 유효 set의 참조 검증 통과(`valid_references_pass`) | 통과 |
| 없는/다른 View node 대상, root 변경, id 중복·key 불일치, 삭제 subtree 아래 추가, 삭제 subtree patch(이동해 나오면 유효), 자식 불가 부모, cycle, 교체 node patch 거부(`dangling_and_invalid_references_are_rejected`) | 통과 |
| 비인스턴스 slot change, 없는 slot, 비소유 content 거부, 같은 set이 옮기거나 추가한 인스턴스·content 인정(`slot_change_references_are_validated`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 `VI.2`(전부 `[x]`)와 `IX.3`은 section 단위 선행이라 사용자 지시(2026-11-14)에 따라 완화 규칙으로
  판단했습니다. IX.3.1~IX.3.3은 이 lane에서 먼저 완료했습니다.
- lane 경계(component 모듈 전용)에 따라 variant 모듈을 `component/variant_override.rs`에 두었습니다.
  slot change가 컴포넌트 Slot 모델에 의존한다는 점도 근거입니다.
- property 값은 core-types `TypedValue`이며, 대상 속성의 type 검사는 property metadata registry
  (VI.3.5 등, 종류별)와 resolver(IX.7.4 이후)의 몫입니다.

## 남은 위험 / 한계

- VariantCondition(IX.7.2), resolver·specificity(IX.7.4~IX.7.6), ComponentInstance 캡슐화 연결(IX.7.7)은
  후속입니다. 현재 reference validation은 정의(가상) node를 base node로 보지 않으므로 그런 patch를
  `UnknownTarget`/`DanglingPatch`로 거부하지만, 전용 진단은 IX.7.7이 제공해야 합니다.
- 최종 구조에서 인스턴스 소유 node가 모두 투영됐는지(미투영 소유 node)는 variant 적용 결과에 IX.3.2
  validator를 돌려 확인해야 합니다.
