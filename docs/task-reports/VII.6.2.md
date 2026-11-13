# 작업 보고서: VII.6.2 random command sequence 전체 undo/redo property test

- WBS: VII.6.2
- 작업명: random valid command sequence 1,000개 생성, 전체 undo 후 초기 상태·전체 redo 후 최종 상태 확인
- 상태: 완료 (`[x]`)
- 선행: VII.6.1, VII.5.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/history/src/sequence_property.rs`(신규, `cfg(test)`) — `smoke_thousand_seed_undo_redo_sequences`.
- `crates/history/src/lib.rs` — 테스트 모듈 등록.
- `docs/task-reports/VII.6.2.md`(신규), 계획서(VII.6.2 `[ ]`→`[x]`).
- 별도 커밋 `5f689c4 Fix: undo·redo 선택 복원을 기록 당시 상태 그대로 적용` — 이 test가 찾은 결함 수정(아래 "차이").

## 구현 요약

- seed 0..1000마다 random valid document(VI.10.5)를 만들고 `WindowHistory`에 6단계를 기록합니다: 1/4 확률로 무작위 선택 event(변화가
  있을 때만 기록), 나머지는 commands `test_support::random_valid_request`(VII.6.1 생성기, 12종 command 무작위)로 만든 유효 command 하나짜리
  transaction.
- 검사: 기록 수 = undo stack 길이 → **전체 undo** 후 semantic snapshot·선택이 초기와 같고 더 undo할 것 없음 → **전체 redo** 후 snapshot·선택이
  최종과 같고 더 redo할 것 없음. 분포 회귀 방지로 문서 편집 > 3,000건·선택 > 500건을 확인합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-history` | 완료 |
| `cargo test -p stellar-history` | unit 24 passed, 0 failed(18.62s, debug — 1,000 seed smoke 포함) |
| `cargo clippy -p stellar-history --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-history -- --check` | 통과 |
| `cargo test -p stellar-commands`, clippy, fmt --check(Fix 커밋 확인) | unit 76 passed, 경고 0, 통과 |

| 검사 | 결과 |
|---|---|
| seed corpus 1,000개 전부: 전체 undo = 초기 snapshot·선택, 전체 redo = 최종 snapshot·선택, 문서 편집 > 3,000·선택 > 500(`smoke_thousand_seed_undo_redo_sequences`) | 통과 |

## 차이(작업 지시와 다른 점)

- 첫 실행에서 seed 96이 "전체 redo 후 선택 불일치"로 실패했습니다. 원인은 undo·redo의 선택 복원이 사용자 선택 경로의 숨김 정책 필터를 다시
  적용해, 기록 뒤 숨김 처리된 node가 복원 선택에서 빠지는 결함이었습니다. WBS 밖 수정으로 `Fix:` 커밋(5f689c4)을 분리해, 복원은 기록 당시
  선택을 그대로 쓰고(문서에 없는 id만 제외) undo·redo API에서 쓰이지 않게 된 선택 정책 인자를 제거했습니다. VII.4.6 테스트는 호출 인자만
  바뀌었고 검증 내용은 같습니다.
- 생성기는 commands의 숨김 공개 `test_support`(VII.6.1)를 재사용합니다.

## 남은 위험 / 한계

- CI는 현재 crate 테스트를 실행하지 않습니다(VI.10.5 보고서와 같은 상태). 이 smoke test는 `cargo test -p stellar-history`에 포함됩니다.
- 생성기가 만들지 않는 조합(preview·grouping 병합 항목)은 VII.4.x fixture가 덮습니다.
