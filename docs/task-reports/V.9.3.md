# 작업 보고서: V.9.3 복구 최신성 probe

- WBS: V.9.3
- 작업명: 정상 저장 generation보다 recovery revision이 최신인지 비교하는 probe
- 상태: 완료 (`[x]`)
- 선행: V.9.2, V.4.4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/recovery_probe.rs`(신규) — `assess_snapshot`,
  `probe_recovery_snapshots`, `SnapshotFreshness`, `StaleReason`,
  `RecoveryProposal`.
- `crates/project/src/lib.rs` — 모듈 등록.
- `docs/task-reports/V.9.3.md`(신규), 계획서(V.9.3 `[ ]`→`[x]`).

## 구현 요약

- **판정 규칙(`assess_snapshot`, 순수 함수)**: 스냅샷의 base manifest hash가
  프로젝트의 current pointer와 일치하고 revision > 0일 때만 Fresh(제안). base가
  다르면(스냅샷 이후 정상 저장 진행) `BaseGenerationMoved`, revision 0이면
  `NoUnsavedChanges`로 Stale — 제안하지 않습니다. 저장 이력 없는 프로젝트는
  base None 스냅샷만 제안됩니다.
- **`probe_recovery_snapshots`**: 프로젝트의 스냅샷 세션을 모두 읽어 최신
  스냅샷만 제안 목록으로 반환합니다. 해석 불가 payload·형식 외 파일은 제안에서
  제외합니다. current pointer 형식 위반·I/O는 오류로 보고합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-project` | 130 passed, 0 failed |
| `cargo fmt -p stellar-project -- --check` | 통과 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 오래된 snapshot(과거 base·revision 0) 미제안, 최신만 제안(`stale_snapshots_are_not_proposed`) | 통과 |
| 판정 matrix 5조합(`assess_matrix`), 해석 불가 스냅샷 제외, 빈 저장소 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 정상 종료 marker와 cleanup(V.9.4)이 후속입니다.
