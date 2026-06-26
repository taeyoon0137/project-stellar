# 작업 보고서: III.6.1 document generational arena

- WBS: III.6.1
- 작업명: `document`에 RuntimeEntityId 핸들 기반 generational arena 추가
- 상태: 완료 (`[x]`)
- 선행: III.1.3(`RuntimeEntityId`/`RuntimeEntityAllocator`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/core-types/src/ids.rs` (`RuntimeEntityAllocator::live_slot` 접근자 + 단위 테스트)
- `crates/document/src/arena.rs` (신규: `Arena<T>`, insert/get/remove/contains, 1M stress test)
- `crates/document/src/lib.rs` (`pub mod arena;`)
- `docs/task-reports/III.6.1.md`(신규), 계획서(III.6.1 `[ ]`→`[x]`)

## 구현 요약

- 사용자 결정(ADR 게이트 후 추천안): **RuntimeEntityId를 유일한 런타임 핸들**로 사용합니다.
- core-types `RuntimeEntityAllocator`에 `live_slot(id) -> Option<usize>`를 추가했습니다. live
  핸들이면 storage 인덱스(slot)를, stale 핸들이면 `None`을 반환하는 O(1) 접근자입니다
  (generation 일치 검사만). slot은 **in-process storage 인덱싱 전용**이며 직렬화·FFI에
  노출하지 않고, generation은 노출하지 않습니다(III.1.3 가시성 의도 유지).
- `document::arena::Arena<T>`: `RuntimeEntityAllocator` + 연속 `Vec<Option<T>>` 저장소.
  `insert`는 핸들을 반환하고 재사용 slot이 있으면 그 slot을 씁니다. 개별 노드를 `Box`로 따로
  할당하지 않습니다(공개 API에 box allocation 없음). `get`/`get_mut`/`contains`는 `live_slot`
  으로 검증하므로 stale 핸들 접근이 실패합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-core-types` (115 unit + 8 doctest) | 0 |
| `cargo test -p stellar-document` (4 unit) | 0 |
| `cargo fmt -p stellar-core-types/-document -- --check` | 0 |
| `cargo clippy -p stellar-core-types/-document --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| `live_slot` live→인덱스, stale→None, 재사용 후 옛 핸들 None | 통과 |
| arena insert/get/remove, 중복 제거 None | 통과 |
| slot 재사용 후 옛 핸들 접근 실패(generation 무효화) | 통과 |
| **100만 insert/remove stress: 모든 stale 핸들 접근 실패**, live 핸들 접근 성공 | 통과(~1s) |

## 차이(작업 지시와 다른 점)

- "generational arena wrapper"를 RuntimeEntityId 핸들 위에 wrapping 하는 형태로 구현했습니다.
  핸들 수명은 core-types 할당기가, 값 저장은 document `Arena<T>`가 담당합니다(`live_slot`로
  연결). 사용자 승인 설계입니다.
- core-types에 `live_slot` 접근자를 추가했습니다. slot/generation 필드 자체는 여전히
  crate-private이며, `live_slot`은 live 핸들의 storage 인덱스만 제한적으로 노출합니다.

## 남은 위험 / 한계

- `Arena<T>`는 AoS(`Vec<Option<T>>`)입니다. III.6.2에서 공통 node 필드를 SoA column으로
  저장하는 `NodeColumns`를 추가합니다(같은 `live_slot` 인덱싱 사용).
- 제거된 slot의 storage는 재사용 시 덮어쓰며, 그 전까지는 `None`으로 비워 둡니다(길이는
  줄이지 않음). 접근은 항상 liveness를 먼저 검사합니다.
