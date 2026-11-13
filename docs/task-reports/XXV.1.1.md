# 작업 보고서: XXV.1.1 영속 history journal schema와 version

- WBS: XXV.1.1
- 작업명: session-only HistoryEntry의 stable journal schema와 version 정의
- 상태: 완료 (`[x]`)
- 선행: Phase VII(phase 단위 선행 — 완화 적용, 아래 "차이" 참조), V.8.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/history/src/journal.rs`(신규) — `HistoryJournal`, `JournalHeader`, `JournalError`, 버전 상수, `encode_entry`/`decode_entry`,
  테스트 2건.
- `crates/history/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XXV.1.1.md`(신규), 계획서(XXV.1.1 `[ ]`→`[x]`).

## 구현 요약

- **schema**(canonical V.1: LE 고정 폭, `u32` 길이 프리픽스, trailing 거부): magic `STLRHJNL` + format version + schema version + 최소 reader
  version + 창 id + base generation(`StorageGeneration`) + 항목 수 + 항목*. 항목은 sequence + origin(agent provenance 포함) + grouping key(tag
  0..=5) + payload(tag 1 문서: 정방향·역방향 `CommandTransaction` canonical bytes, tag 2 선택: 정방향·역방향 `SetSelection`) + 선택 복원 정보
  (VII.5.5). command·선택 bytes는 commands crate의 canonical codec(VII.1.2~VII.3.2)을 그대로 씁니다.
- **version 규칙**(V.8.1과 같은 판정): format version은 reader 지원값과 정확히 같아야 하고, reader schema가 journal의 최소 reader version보다
  낮으면 거부, 최소 reader > schema 선언은 모순으로 거부합니다. 현재 값은 모두 1입니다.
- **무결성**: 항목 sequence가 단조 증가하지 않으면 거부합니다. 알 수 없는 command kind(registry 밖)는 command codec이 거부합니다.
- journal은 project serializer(문서 chunk·manifest)와 별개이며, 기록 위치·복원 조건·봉인·보존 정책은 XXV.1.2~XXV.1.5의 몫입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-history` | 완료 |
| `cargo test -p stellar-history` | unit 26 passed, 0 failed |
| `cargo clippy -p stellar-history --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-history -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 일반·선택·agent origin 키보드 반복·텍스트·pointer gesture·scrub 항목 6개(선택 복원 정보 포함) journal round-trip·재인코딩 동일, 항목 단위 round-trip, AreaSelection·key 없는 signature 왕복(`journal_round_trips_every_entry_kind`) | 통과 |
| magic·format 불일치, 미래 schema(최소 reader 2) 현재 reader 거부·reader 2 허용, 모순 선언, 절단·trailing, sequence 역순 거부(`version_rules_and_malformed_input_are_rejected`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 `Phase VII`은 phase 단위 선행이라 사용자 지시(2026-11-14)에 따라 완화했습니다. 필요한 코드(HistoryEntry, command codec)는 이 lane에서
  모두 구현했고, Phase VII 중 VII.6.3만 `[~]`(CI fuzz 등록 미완)입니다.
- V.8.1의 `FormatVersions`는 storage crate에 있어 history가 의존할 수 없으므로(허용 edge 밖), 같은 판정 규칙을 journal 전용 상수로 두었습니다.

## 남은 위험 / 한계

- journal을 실제 파일로 기록·복원하는 경로(ignored path/OS app data, Git 추적 금지 검사, base 불일치 봉인)는 XXV.1.2 이후입니다.
