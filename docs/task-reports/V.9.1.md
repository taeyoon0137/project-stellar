# 작업 보고서: V.9.1 RecoverySnapshotStore

- WBS: V.9.1
- 작업명: OS app data의 `RecoverySnapshotStore` 인터페이스 — 키 ProjectIdentity + WindowSessionId
- 상태: 완료 (`[x]`)
- 선행: IV.2.3, V.1.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/recovery.rs`(신규) — `RecoverySnapshotStore`,
  `RecoveryStoreError`.
- `crates/core-types/src/ids.rs` — `WindowSessionId` newtype 추가(V.9.1·VII.1.1
  공용 식별자).
- `crates/project/src/lib.rs` — `recovery` 모듈 등록.
- `docs/task-reports/V.9.1.md`(신규), 계획서(V.9.1 `[ ]`→`[x]`).

## 구현 요약

- **위치**: 프로젝트 저장소 밖, 플랫폼 셸이 주입하는 앱 데이터 경로 아래
  `recovery/<identity-hash>/<window-session-uuid>.starrecovery`.
- **경로 개인정보 정책**: 프로젝트 디렉터리 이름은 project StableId + canonical
  target 경로를 canonical 인코딩(V.1.2)한 content hash(V.1.5)입니다 — 프로젝트
  이름·경로 텍스트가 경로에 노출되지 않고, 같은 target의 symlink 별칭들은 같은
  디렉터리로 모입니다(IV.2.3 canonical target 활용).
- **인터페이스**: `snapshot_path`/`write_snapshot`(atomic write, V.2.5)/
  `read_snapshot`/`list_sessions`/`remove_snapshot`. payload 형식은 V.9.2 소관.
- **`WindowSessionId`**: StableId 기반 newtype을 core-types `ids`에 추가했습니다.
  VII.1.1(commands)의 WindowSessionId가 이 타입을 재사용할 수 있는 위치입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-project -p stellar-core-types` | 120 + 115 passed, 0 failed |
| `cargo test --workspace` | 실패 0 |
| `cargo fmt -- --check`(두 crate) | 통과 |
| `cargo clippy -p stellar-project -p stellar-core-types --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 경로에 프로젝트명·root 경로 텍스트 미노출(`paths_do_not_leak_project_name_or_path_text`) | 통과 |
| 같은 target symlink 별칭이 같은 디렉터리 공유 | 통과 |
| write/read/list/remove round-trip, atomic 덮어쓰기, 다중 세션 | 통과 |

## 차이(작업 지시와 다른 점)

- `WindowSessionId`는 VII.1.1 명세에 있는 타입이지만 V.9.1 키에 필요해 core-types
  `ids`에 먼저 정의했습니다(commands crate는 core-types를 의존하므로 재사용 가능).
- store는 crate 그래프상 storage가 ProjectIdentity를 알 수 없어 `project` crate에
  두었습니다(project는 storage 의존 허용).

## 남은 위험 / 한계

- canonical payload 정의(V.9.2), 최신성 probe(V.9.3), 정상 종료 marker(V.9.4)가
  후속입니다.
