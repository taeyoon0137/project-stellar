# 작업 보고서: VIII.8.4 orphan 재연결·명시 삭제 command

- WBS: VIII.8.4
- 작업명: orphaned binding 재연결과 명시 삭제 command
- 상태: 완료 (`[x]`)
- 선행: VIII.8.1(`[x]`, 이 lane), Phase VII(phase 단위 선행 — 완화 적용, 아래 "차이" 참고)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/bindings/src/orphan_commands.rs`(신규) — `OrphanCommand`(Reconnect·Delete·Orphanize·Restore),
  `OrphanCommandError`와 테스트.
- `crates/bindings/src/orphan.rs` — command용 내부 변경 helper(`live_mut`, `insert_orphan`, `remove_orphan`).
- `crates/bindings/src/lib.rs` — `orphan_commands` 모듈 등록.
- `docs/task-reports/VIII.8.4.md`(신규), 계획서(VIII.8.4 `[ ]`→`[x]`).

## 구현 요약

- **command**: Reconnect(새 source·기대 타입으로 live 복귀, ID·target·owner 유지) ↔ Orphanize(재연결 전 orphan을 원래
  source·원인 그대로 복원), Delete(명시 삭제) ↔ Restore. `apply`는 검증 후 적용하고 inverse를 돌려주며 실패 시 저장소 불변.
- **재연결 검증**: orphan 존재, 같은 target에 live binding 없음, 기대 타입 = target 현재 선언 타입, cycle 없음(통합 graph),
  다시 연결한 binding이 현재 상태에서 깨지지 않음(VIII.8.1 `break_cause`: target·참조 존재와 type).
- **값**: 재연결 후에는 live binding이 값을 만들고(live value), undo로 orphan이 되면 선언 기본값(VIII.8.2)으로 돌아갑니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-bindings` | 종료 코드 0 |
| `cargo test -p stellar-bindings` | 첫 실행 1 실패(기대 타입이 선언 타입과 달라도 재연결이 통과하던 검증 누락 발견) → 선언 타입 일치 검사(`ExpectedTypeMismatch`) 추가 후 종료 코드 0 (48 unit passed) |
| `cargo clippy -p stellar-bindings --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-bindings -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **재연결 후 live value, undo 후 orphan 상태**(`reconnect_gives_live_value_and_undo_restores_orphan`): orphan 상태 p1 = 기본값 4px·p2 = 5px → `t2 + 1px` 재연결 후 p1 = 8px(live)·p2 = 9px → undo 후 저장소가 orphan 상태와 동일하고 p1 = 4px → redo 후 재연결 상태와 동일 | 통과 |
| 명시 삭제·undo·redo(`explicit_delete_and_undo`): 삭제 후 orphan 없음·p1 값 없음, undo로 같은 orphan 복원 | 통과 |
| 거부 시 불변(`invalid_reconnects_are_rejected_without_changes`): 여전히 없는 토큰 `SourceMissing`, 기대 타입 ≠ 선언 타입, source 결과 타입 불일치, cycle, 없는 orphan·live, 중복 복원, 같은 target live 존재 | 통과 |

## 차이(작업 지시와 다른 점)

- **Phase VII 선행 완화**: VIII.5.5·VIII.6.6과 같이 Phase VII command 기반이 없어(VII.1.x 전부 `[ ]`, `stellar-commands`
  의존 불가) orphan 전용 closed enum command로 구현했습니다. Phase VII 구현 시 옮기는 재작업이 필요합니다.
- 신규 모듈이므로 테스트는 구현과 함께 작성했습니다(구현 전 상태는 컴파일 불가).

## 남은 위험 / 한계

- 재연결 검증은 매번 통합 graph를 만듭니다(O(토큰 + binding)).
