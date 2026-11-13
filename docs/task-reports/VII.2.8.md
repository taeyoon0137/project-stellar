# 작업 보고서: VII.2.8 DuplicateSubtree

- WBS: VII.2.8
- 작업명: `DuplicateSubtree` — IdRemapTable로 내부 ID/참조를 새로 생성
- 상태: 완료 (`[x]`)
- 선행: III.1.5, VII.2.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/duplicate.rs`(신규) — `DuplicateSubtree`(plan/validate/invert/apply/codec), payload·modifier 참조 재매핑,
  테스트 2건.
- `crates/commands/src/command.rs`, `crates/commands/src/kind.rs`, `crates/commands/src/codec.rs`, `crates/commands/src/lib.rs` —
  `DuplicateSubtree = 0x0040` 등록·직렬화·dispatch.
- `docs/task-reports/VII.2.8.md`(신규), 계획서(VII.2.8 `[ ]`→`[x]`).

## 구현 요약

- **id 재매핑**: subtree의 모든 영속 id(node, modifier, path point·handle)에 core-types `IdRemapTable`(III.1.5)로 새 id를 줍니다.
  `DuplicateSubtree::plan`은 결정적 `StableIdGenerator`(III.1.2)·`MonotonicClock`으로 쌍을 만들고, command는 old → new 쌍을 그대로
  담아 redo가 같은 id를 다시 만듭니다.
- **참조 재매핑**: Frame clip·Boolean 피연산·Mask/Clip 원본이 subtree 안을 가리키면 새 id로(내부), 밖을 가리키면 원래 대상으로
  (외부) 둡니다(`IdRemapTable::remap`). 복제본 root는 새 부모·새 OrderKey를 받고, 자손은 재매핑한 부모와 원래 key를 씁니다.
- **검증·적용**: 쌍이 subtree id를 정확히 덮는지(누락·여분 거부), 새 id가 NIL·중복이 아닌지 확인한 뒤, 재매핑 레코드로 만든
  `InsertNode`의 검증·적용 경로를 그대로 지납니다(문서 id 공간 미사용, 부모 container, 형제 key, 깊이, 참조 종류, modifier 규칙).
  자기 subtree 안으로의 복제도 새 node라 허용됩니다.
- **inverse**: 복제본 root의 `DeleteSubtree`.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-commands` | 완료 |
| `cargo test -p stellar-commands` | unit 55 passed, doctest 3 passed, 0 failed |
| `cargo clippy -p stellar-commands --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-commands -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 내부 clip·mask(→ Path)와 외부 mask(→ 13)를 가진 subtree 복제: 쌍 9개(node 3, path 4, modifier 2), 새 id 집합 = 문서에 추가된 id, 원본 id와 서로소, 내부 참조 → 복제본, 외부 참조 유지, path id 재매핑, 원본 불변, 문서 validator 통과, undo 후 snapshot 동일, codec round-trip(`duplicate_remaps_internal_ids_and_references`) | 통과 |
| 쌍 누락·사용 중 새 id·새 id 중복·형제 key 충돌 거부(문서 불변), 자기 subtree 안 복제 허용(`duplicate_rejects_bad_remap_and_placement`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 컴포넌트 인스턴스 등 Phase IX 참조는 아직 `DocumentModel` 레코드에 없어 재매핑 대상이 아닙니다.
