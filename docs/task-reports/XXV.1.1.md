# 작업 보고서: XXV.1.1 영속 history journal schema와 version

- WBS: XXV.1.1
- 작업명: session-only HistoryEntry의 stable journal schema와 version 정의
- 상태: 완료 (`[x]`)
- 선행: Phase VII(phase 단위 선행 — 완화 적용, 아래 "차이" 참조), V.8.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/history/src/journal.rs`(신규) — `HistoryJournal`, `JournalHeader`, `JournalBase`, `JournalAccess`, `JournalError`, 버전 상수,
  `encode_entry`/`decode_entry`, 테스트 3건.
- `crates/history/src/lib.rs` — 모듈 등록.
- `crates/history/src/entry.rs`·`crates/history/src/history.rs` — 다음 sequence 조회(`SequenceAllocator::peek`, `History::next_sequence`,
  개정 시 추가).
- `docs/task-reports/XXV.1.1.md`(신규), 계획서(XXV.1.1 `[ ]`→`[x]`).

## 구현 요약

- **schema**(canonical V.1: LE 고정 폭, `u32` 길이 프리픽스, trailing 거부): magic `STLRHJNL` + format version + schema version + 최소 reader
  version + 최소 writer version + 창 id + base(`StorageGeneration` + manifest content hash 32바이트) + next sequence + undo 항목 수·항목* + redo
  항목 수·항목*. 항목은 sequence + origin(agent provenance 포함) + grouping key(tag 0..=5) + payload(tag 1 문서: 정방향·역방향
  `CommandTransaction` canonical bytes, tag 2 선택: 정방향·역방향 `SetSelection`) + 선택 복원 정보(VII.5.5). command·선택 bytes는 commands
  crate의 canonical codec(VII.1.2~VII.3.2)을 그대로 씁니다.
- **복원에 필요한 상태**: journal 하나가 창 history의 복원 가능한 전체 상태(undo stack, redo stack, 다음 sequence)를 담습니다.
  `HistoryJournal::capture`가 `History`에서 그대로 만듭니다. base는 그 상태가 기준으로 삼은 저장 세대이며, 두 값(generation 번호와 manifest
  hash)이 모두 같아야 같은 base입니다.
- **version 규칙**(V.8.1 `assess_compatibility`와 같은 판정): format version은 정확히 같아야 하고, 선언은 `최소 reader <= 최소 writer <=
  schema`여야 합니다(위반은 `InvalidVersions`). reader schema가 최소 reader보다 낮으면 거부, 최소 writer보다 낮으면 읽기 전용
  (`JournalAccess::ReadOnly`), 그 밖은 편집 가능입니다. 현재 값은 모두 1입니다.
- **무결성**: undo sequence 엄격 증가, redo sequence 엄격 감소(다음 redo가 가장 작음), 마지막 undo < 다음 redo, next sequence > 모든 항목
  sequence를 디코딩에서 검사합니다. 알 수 없는 command kind(registry 밖)는 command codec이 거부합니다.
- journal은 project serializer(문서 chunk·manifest)와 별개이며, 기록 위치·복원 조건·봉인·보존 정책은 XXV.1.2~XXV.1.5의 몫입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

최초 작업(커밋 `406b551`):

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-history` | 완료 |
| `cargo test -p stellar-history` | unit 26 passed, 0 failed |
| `cargo clippy -p stellar-history --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-history -- --check` | 통과 |

schema 개정(review JOURNAL-SCOPE 수정 커밋):

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-history` | 완료 |
| `cargo test -p stellar-commands -p stellar-history` | commands unit 79 passed·doctest 3 passed, history unit 29 passed, 0 failed |
| `cargo clippy -p stellar-commands -p stellar-history --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-commands -p stellar-history -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 일반·선택·agent origin 키보드 반복·텍스트·pointer gesture·scrub 항목 6개 중 2개를 undo한 history(undo 4·redo 2·next 7·선택 복원 정보 포함)의 journal round-trip·재인코딩 동일, 항목 단위 round-trip, AreaSelection·key 없는 signature, 빈 history 왕복(`journal_round_trips_undo_redo_and_next_sequence`) | 통과 |
| V.8.1 matrix: format 불일치 거부, schema 5·최소 reader 3·최소 writer 4 journal을 reader 5·4 편집 가능, 3 읽기 전용, 2 거부, 현재 reader 1 거부, 최소 reader > 최소 writer·최소 writer > schema 모순 거부(`version_rules_follow_v81_matrix`) | 통과 |
| magic·절단·trailing 거부, undo 역순·redo 순서 뒤집힘·redo가 undo보다 오래됨·next sequence 0 또는 기록값 이하 거부(`malformed_input_and_sequence_integrity_are_rejected`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 `Phase VII`은 phase 단위 선행이라 사용자 지시(2026-11-14)에 따라 완화했습니다. 필요한 코드(HistoryEntry, command codec)는 이 lane에서
  모두 구현했고, Phase VII 중 VII.6.3만 `[~]`(CI fuzz 등록 미완)입니다. 엄격한 Gate 충족(G4: CI-MAC·CI-WIN)은 아닙니다.
- V.8.1의 `FormatVersions`는 storage crate에 있어 history가 의존할 수 없으므로(허용 edge 밖), 같은 판정 규칙을 journal 전용 상수·판정으로
  두었습니다.
- **schema 개정(2026-11-14, review JOURNAL-SCOPE)**: 최초 schema는 undo 항목만 담고 redo stack·next sequence·최소 writer version이 없어 XXV.1.3
  복원이 redo 갈래와 sequence 연속성을 되살릴 수 없었습니다. 기록 경로(XXV.1.2) 이전이라 디스크에 기록된 journal이 없으므로 schema 1을 제자리에서
  고쳐 redo stack, next sequence, 최소 writer version을 넣었습니다. 같은 이유로 base를 generation 번호만이 아니라 manifest content hash와의
  쌍(`JournalBase`)으로 바꿨습니다 — generation 번호만으로는 다른 branch의 같은 번호 세대를 구분할 수 없습니다. `HistoryJournal.entries`는
  `undo`·`redo`로 나뉘었습니다.

## 남은 위험 / 한계

- journal을 실제 파일로 기록·복원하는 경로(ignored path/OS app data, Git 추적 금지 검사, base 불일치 봉인)는 XXV.1.2 이후입니다.
- base manifest hash의 의미(어느 값을 넣는지)는 기록하는 쪽 계약입니다. history crate는 storage에 의존하지 않아 값을 해석하지 않습니다.
