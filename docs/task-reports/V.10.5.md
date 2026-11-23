# 작업 보고서: V.10.5 여러 의미 파일 저장 journal과 한쪽 세대 복구

## WBS

- WBS: V.10.5
- 작업명: 여러 의미 파일의 temp/fsync/rename 저장 journal과 앞/새 세대 한쪽 복구
- 상태: 완료 (`[x]`) — 이 행의 구현과 DEV-MAC 검증 기준입니다. 계획서·원장 상태와 선행 V.10.4 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: V.10.4. journal API는 codec과 독립인 bytes 수준이라 V.10.4를 기다리지 않고 구현했습니다(ADR-0078 결정 1). 선행 완료 판정은 통합 때 따릅니다.
- 결정: [ADR-0078](../adr/ADR-0078-multi-file-save-journal.md).

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일

- `crates/storage/src/journal/mod.rs`(신규): `commit`, `recover`, `JournalChange`(쓰기·삭제·이름 바꾸기), `JournalError`, 복구 결과 타입, 단계 관찰자.
- `crates/storage/src/journal/manifest.rs`(신규): Git 밖 manifest 텍스트 형식과 엄격한 해석.
- `crates/storage/src/journal/path.rs`(신규): 대상 상대 경로 규칙(`..`, 절대 경로, `.git`, `.stellar/` 거부).
- `crates/storage/src/journal/tests.rs`(신규): fault matrix와 CAS·symlink·권한·잠금·손상 journal 검사.
- `crates/storage/src/fs_guard.rs`: 만들지 않고 실제 디렉터리를 확인하는 `existing_real_dirs`, `FileIdentity::dev`, 검사 1개.
- `crates/storage/src/atomic_file.rs`: `sync_parent_dir`를 crate 공개로 바꿈(동작 같음).
- `crates/storage/src/source_file.rs`: 모듈 문서에서 여러 파일 거래를 journal로 안내.
- `crates/storage/src/lib.rs`: `pub mod journal;` 등록 한 줄.
- `docs/adr/ADR-0078-multi-file-save-journal.md`(신규), 이 보고서.

## 구현 요약

- 입력은 (상대 경로, 새 bytes 또는 삭제, 기대 원본 hash) 집합입니다. 이름 바꾸기는 삭제와 생성 두 항목입니다.
- 순서는 다음과 같습니다.
  1. 잠금을 잡고 남은 거래를 복구합니다.
  2. 모든 대상의 기대 hash를 비교합니다(CAS). 하나라도 다르면 아무것도 만들지 않고 `Stale`입니다.
  3. `.stellar/journal/tx-<id>/manifest`를 쓰고 fsync합니다.
  4. `orig/`·`new/` 사본을 쓰고 fsync합니다. 그다음 `orig/`·`new/`·거래 디렉터리·`.stellar/journal/`을 sync합니다(거래 디렉터리 항목 내구화).
  5. 대상마다 디렉터리 identity와 기대 hash를 다시 확인하고 rename·hard link·unlink합니다.
  6. 대상 디렉터리를 sync하고, 이 저장이 부모 디렉터리를 만든 대상은 루트까지 조상 디렉터리도 sync한 뒤 commit marker를 씁니다.
  7. 거래 디렉터리를 정리합니다.
- marker 전 실패는 같은 호출 안에서 앞 세대로 되돌립니다. 중단 뒤 복구는 marker 유무로 방향을 정하고 대상별 hash로만 판정합니다. 어느 세대도 아닌 대상(외부 수정 등)은 덮어쓰지 않고 거래를 `quarantine-<id>`로 보존합니다.
- 기본 열기·저장 경로 연결은 V.10.6 범위라 바꾸지 않았습니다. CLI·MCP 표면도 더하지 않았습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage --locked` | 0 — lib 288 passed(journal 20 포함), 통합 5+1, doctest 2 |
| `cargo test -p stellar-storage --locked journal` | 0 — 20 passed |
| `cargo clippy -p stellar-storage --all-targets --locked -- -D warnings` | 0 — 경고 0 |
| `cargo fmt -p stellar-storage -- --check` | 0 |
| 변형 검사: 복구 방향을 항상 앞 세대로 바꾸고 fault matrix 실행 | 실패(검사가 잘못된 방향을 잡음), 원래 코드로 되돌림 |

| 검사 | 결과 |
|---|---|
| 성공 실행 단계가 ADR 순서와 같음(`recorded_steps_follow_the_adr_order`, 20단계) | 통과 |
| 20단계 × {중단, 오류, 디스크 가득 참, 권한} 주입 → 재열기 결과가 앞/새 세대와 bytes 동일, marker 전은 앞·후는 새 세대, journal 잔여 0 | 통과 |
| commit 중단 지점 20곳마다 복구 단계별 중단·오류 주입 → 다음 복구가 같은 세대로 맞춤 | 통과 |
| marker는 있고 반영이 덜 된 거래 → 새 세대로 완료 | 통과 |
| 손상된 원본 사본은 쓰지 않고 그 대상 보존 + quarantine, 다음 저장 가능 | 통과 |
| 중단 뒤 외부 수정은 복구가 덮어쓰지 않음 | 통과 |
| 시작 CAS stale(교체·생성·삭제 대상) → 대상·부모 디렉터리·거래 디렉터리 생성 0 | 통과 |
| 반영 도중 외부 수정(교체·생성·삭제 대상) → 반영분 되돌림 후 stale | 통과 |
| 실제 권한 오류(읽기 전용 디렉터리) → 다른 디렉터리 반영분까지 되돌림 | 통과 |
| 교체·되돌림의 권한 비트 보존 | 통과 |
| symlink 디렉터리·대상 거부, 프로젝트 밖 파일 불변 | 통과 |
| 잘못된 경로·중복(대소문자)·크기 초과 → journal 생성 전 거부 | 통과 |
| 다른 writer가 잠금 보유 → `Busy` | 통과 |
| 변조 manifest(`../`, 거래 ID 불일치) → 대상 불변 + quarantine, manifest 없는 거래 버림 | 통과 |
| 기존 save fault matrix(`save::fault_matrix`) 회귀 | 통과 |

## 통합 검증

| 명령 | 결과 |
|---|---|
| `cargo xtask validate-adr` + ce60bb92 이후 변경 경로 10개 | 0 — storage 변경에 ADR-0078 있음 |
| `cargo xtask validate-adr crates/storage/src/journal/mod.rs`(ADR 없는 음성 대조) | 65 — `missing-adr` |
| `cargo xtask generate-ffi-header --check` | 0 — drift 없음(FFI 변경 없음) |
| `cargo xtask validate-task-report docs/task-reports/V.10.5.md` | 0 |
| `STELLAR_CHECK_BASE=ce60bb92 cargo xtask check-core`(샌드박스 해제) | 0 — fmt·clippy 통과, workspace 4,157 passed·0 failed·13 ignored, doctest 54 passed |
| `git diff --cached --check` | 0 |

## 성능

- 측정하지 않았습니다. 저장 I/O는 바뀐 파일 크기의 약 두 배(사본·새 내용)와 fsync입니다. 예산 실측은 V.10.7에서 합니다.

## 차이(작업 지시와 다른 점)

- 선행 V.10.4 완료 전에 착수했습니다. API가 codec과 독립이라는 lane 결정에 따른 것입니다.
- 디스크 가득 참은 실제 볼륨을 채우지 않고 `StorageFull` 오류 주입으로 검사했습니다. 권한 오류는 주입과 실제 읽기 전용 디렉터리 두 방식으로 검사했습니다.

## 남은 위험 / 한계

- 마지막 확인과 rename 사이의 비협조적 외부 writer race는 남습니다(`source_file`과 같음).
- 판정할 수 없는 대상이 있으면 그 대상만 외부 내용으로 남고 거래는 quarantine에 보존됩니다. 사용자에게 보여 주는 경로는 V.10.6 이후입니다.
- 저장 중 만든 빈 부모 디렉터리는 되돌릴 때 남습니다. Unicode 정규화만 다른 이름은 같은 대상으로 판정하지 못합니다.
- 기본 저장·열기·자동 저장 연결, 파일 감시와의 상호작용은 V.10.6 범위입니다.
- 디렉터리 sync 순서(거래 디렉터리 항목, 새로 만든 부모 디렉터리 항목)는 전원 차단을 흉내 내지 못하므로 테스트로 관찰하지 않았습니다. 코드 검토로만 확인했습니다.
- 새 파일 게시와 복구는 hard link를 씁니다. hard link를 지원하지 않는 볼륨(exFAT 등)에서는 새 파일이 있는 저장이 오류로 끝나고 앞 세대로 되돌아갑니다.
