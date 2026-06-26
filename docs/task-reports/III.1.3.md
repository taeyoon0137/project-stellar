# 작업 보고서: III.1.3 RuntimeEntityId(generational)

- WBS: III.1.3
- 작업명: `RuntimeEntityId { slot, generation }` 추가 + 저장·FFI 비노출(visibility 제한), slot 재사용 시 옛 generation lookup 실패
- 상태: 완료 (`[x]`)
- 선행: III.1.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/core-types/src/ids.rs` (`RuntimeEntityId`, `RuntimeEntityAllocator`, compile_fail visibility doctest, 단위테스트)
- `docs/task-reports/III.1.3.md`, 계획서(III.1.3 `[ ]`→`[x]`)

## 구현 요약

- `RuntimeEntityId { slot: u32, generation: u32 }`. 필드는 `pub(crate)` — 타입은 공개(런타임 crate가 불투명 핸들로 보유 가능)지만 slot/generation은 crate 외부(storage/ffi)에서 접근 불가. 직렬화하지 않음.
- `RuntimeEntityAllocator`(generational index): `allocate`(재사용 slot 우선), `is_live`(generation 일치), `deallocate`(generation +1로 옛 핸들 무효화 후 slot 재사용 풀에).
- compile_fail doctest로 외부 crate에서 `id.slot` 접근이 컴파일 실패함을 검증(visibility 제한 증명).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 삭제 후 slot 재사용 시 옛 generation lookup 실패, 새 핸들만 live | 통과 |
| 중복 deallocate 실패, deallocate 직후 옛 핸들 무효 | 통과 |
| 신규 slot 순차 할당(0,1, generation 0) | 통과 |
| 외부 crate에서 `id.slot` 접근 컴파일 실패(compile_fail doctest) | 통과 |
| `cargo test -p stellar-core-types`(19 unit + 3 doctest) / fmt / clippy `-D warnings` | 통과 |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- "visibility 제한"은 `pub(crate)` 필드 + 직렬화 없음 + FFI 미노출로 구현했습니다. 타입 자체는 공개해 런타임 crate가 핸들을 보유할 수 있게 하되, slot/generation은 crate 내부에서만 접근합니다.
