# 작업 보고서: III.6.3 DocumentSnapshotView 읽기 전용

- WBS: III.6.3
- 작업명: 읽기 전용 `DocumentSnapshotView` 추가, 가변 arena 참조 외부 비노출
- 상태: 완료 (`[x]`)
- 선행: III.6.2(`NodeColumns`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/snapshot.rs` (신규: `Document`, `DocumentSnapshotView`, compile_fail doctest, 단위 테스트)
- `crates/document/src/lib.rs` (`pub mod snapshot;`)
- `docs/task-reports/III.6.3.md`(신규), 계획서(III.6.3 `[ ]`→`[x]`)

## 구현 요약

- `Document`가 `NodeColumns`를 **소유**하고, 외부에는 읽기 전용 `DocumentSnapshotView`만
  제공합니다. 가변 arena 참조(`&mut NodeColumns`)나 가변 접근자를 공개 API로 노출하지
  않습니다.
- 변경은 `Document`의 `&mut self` 메서드(`insert_node`/`remove_node`/`set_parent`)로만
  가능합니다. 읽기 전용 소비자는 `&Document` → `snapshot()`으로만 접근합니다(borrow checker가
  강제).
- `DocumentSnapshotView<'a>`는 공유 참조 `&'a NodeColumns`만 보유하며 변경 API가 없습니다
  (`get`/`contains`/`len`/`is_empty`). `Copy`라 값 복사가 저렴합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-document` (11 unit + 2 doctest) | 0 |
| `cargo fmt -p stellar-document -- --check` | 0 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| 스냅샷이 문서 상태(get/contains/len)를 정확히 읽음 | 통과 |
| view 반납 후에만 변경 가능(borrow 규칙) | 통과 |
| **compile_fail: view에 변경 메서드 없음** | 통과 |
| **compile_fail: `&Document`로는 변경 메서드 호출 불가** | 통과 |

## 차이(작업 지시와 다른 점)

- "DocumentSnapshotView"를 제공하기 위해 소유자 `Document`를 함께 도입했습니다(가변 arena를
  소유·은닉하고 읽기 전용 view만 노출하는 경계 역할). 외부 직접 변경 불가는 두 개의
  `compile_fail` doctest로 검증합니다(외부 crate처럼 컴파일되는 doctest 컨텍스트 사용).

## 남은 위험 / 한계

- `Document`는 현재 최소 노드 저장(공통 필드)만 다룹니다. 종류별 payload, 트리 무결성(부모-자식
  일관성), 트랜잭션은 후속 Phase(VI, VII)에서 추가합니다.
- 변경 메서드(`insert_node` 등)는 `pub`이지만 `&mut Document`를 요구하므로, 읽기 전용
  소비자(공유 참조 보유)는 호출할 수 없습니다. crate 가시성이 아니라 borrow checker로 경계를
  강제합니다.

## Phase III 완료

- 본 작업으로 Phase III(코어 식별자·값 타입·런타임 자료구조)의 III.6까지 모두 `[x]`가 됩니다.
  코어 primitive가 결정론적 round-trip과 compile-fail 타입 안전성 테스트를 통과합니다.
