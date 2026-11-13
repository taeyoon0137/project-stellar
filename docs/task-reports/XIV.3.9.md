# 작업 보고서: XIV.3.9 diff apply

- WBS: XIV.3.9
- 작업명: diff apply 함수를 구현해 base snapshot에 change list를 적용하고 target snapshot을 재현
- 상태: 완료 (`[x]`)
- 선행: XIV.3.8 `[x]`(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-diff/src/apply.rs`(신규) — `apply`, `ApplyError`.
- `crates/semantic-diff/src/snapshot.rs` — crate 내부 `get_mut`.
- `crates/semantic-diff/src/lib.rs` — `apply` module 선언.
- `docs/task-reports/XIV.3.9.md`(신규), 계획서(XIV.3.9 `[ ]`→`[x]`).

## 구현 요약

- **단계 적용**: 삭제 → 추가 → 위치 → 속성 순서로 적용해 목록 순서와 무관합니다(같은 id 교체
  포함).
- **base 검증(stale 거부)**: 삭제는 객체 전체가 기록과 같아야 하고, 추가는 id가 없어야 하며,
  위치는 from, 속성은 old(canonical 비교, 없음 포함)가 현재 값과 같아야 합니다.
- **목록 검증**: 같은 대상(객체 삭제·추가·위치, 속성 경로)을 두 번 다루는 목록, 종류 분류가
  대상과 맞지 않는 변경(token 아닌 객체의 ChangeToken, 같은 parent의 MoveObject 등), 효과 없는
  속성 변경을 거부합니다.
- **원자성**: base를 복제한 새 snapshot에 적용하고 실패하면 오류만 반환합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-diff` | 43 passed, 0 failed |
| `cargo fmt -p stellar-semantic-diff -- --check` | 통과 |
| `cargo clippy -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| property test 200 seed × 4쌍(무관한 문서 A↔B, 문서↔무작위 변형): `apply(diff(A,B), A) == B`, 역순 목록도 동일, 변형이 Add/Remove/Move/Reorder/SetProperty/AddToken을 실제로 생성(`apply_diff_reproduces_target_property`) | 통과 |
| stale base: 속성 old 불일치 `PropertyMismatch`, 삭제 대상 변경 `ObjectMismatch`, 이미 적용 `MissingObject`/`ObjectExists`(`stale_base_is_rejected`) | 통과 |
| 중복 대상·효과 없는 변경·잘못된 kind·같은 parent Move 거부, 실패 후 base 불변(`invalid_change_lists_are_rejected`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- apply는 snapshot 수준 정합(속성 root 등)만 검사합니다. parent 존재·참조 무결성 등 문서
  불변식은 candidate snapshot 검증(XIV.5.3·XIV.10.8)이 문서 validator로 확인해야 합니다.
