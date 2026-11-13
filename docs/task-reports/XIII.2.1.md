# 작업 보고서: XIII.2.1 플랫폼 독립 파일 감시 event 모델

- WBS: XIII.2.1
- 작업명: 플랫폼 독립 `ProjectFileWatcher` event에 create, modify, remove, rename, overflow 정의
- 상태: 완료 (`[x]`)
- 선행: Phase XII(완화 — 아래 "차이" 참조)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/watcher/mod.rs` (신규: `ProjectFileWatcher` trait, 재노출)
- `crates/project/src/watcher/event.rs` (신규: `FileWatchEvent`, `WatchPath`, `WatchRootId`,
  `OverflowReason`, `WatchEventKind`, `encode_events`/`decode_events`, `WatchEventCodecError`, 테스트)
- `crates/project/src/lib.rs` (`pub mod watcher;` 추가)
- `docs/task-reports/XIII.2.1.md`(신규), 계획서(XIII.2.1 `[ ]`→`[x]`).

## 구현 요약

- **event 다섯 종류**: `Created`, `Modified`, `Removed`, `Renamed { from, to }`, `Overflow(reason)`.
  넘침 사유는 `EventsDropped`·`RescanRequired`·`RootChanged`·`QueueFull`·`UnrepresentablePath`이며
  수신자는 넘침을 받으면 상태를 clean으로 가정하지 않고 다시 검사해야 합니다(XIII.2.6).
- **경로**: `WatchPath = (WatchRootId, 상대 경로)`. root 0은 프로젝트 root, 1 이상은 외부 symlink
  target 감시 root입니다. 상대 경로는 프로젝트 경로 규칙(`split_project_path`)을 따르고 빈 문자열은
  root 자신입니다. `starts_with`는 같은 root에서 컴포넌트 단위로 비교합니다.
- **adapter 인터페이스**: `ProjectFileWatcher { drain, wait(timeout) }` — 플랫폼 adapter가 OS 알림을
  이 모델로 normalize해 내보냅니다.
- **canonical 인코딩**(storage canonical R1~R3): `u8 version(1) · u32 LE 수 · event*`,
  event = `u8 tag · body`, path = `u32 LE root · u32 LE 길이 · UTF-8`. decoder는 미지원 version·알 수
  없는 tag/사유·잘린 입력·남는 bytes·UTF-8 오류·규칙 위반 경로를 거부하고, 주장한 event 수로
  과할당하지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 204 passed, 0 failed; fuzz 통합 4 passed) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **event model round-trip**: 다섯 종류·모든 넘침 사유·외부 root·root 자신·한글 경로 batch가 encode→decode로 동일, 결정론, 빈 batch(`event_batch_round_trips`) | 통과 |
| golden bytes 고정(`encoding_matches_golden_bytes`) | 통과 |
| 모든 잘린 접두사·남는 bytes·미지원 version·알 수 없는 tag/사유·규칙 위반 경로·UTF-8 오류·과장된 event 수 거부(`malformed_batches_are_rejected`) | 통과 |
| 경로 규칙과 `starts_with`(`watch_paths_follow_project_path_rules`), 종류·경로 접근자(`event_kinds_and_paths`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 "Phase XII"는 사용자 지시(2026-11-14)에 따라 완화했습니다. 이 작업은 플랫폼 독립 모델이며
  Phase XII(셸 통합) 코드가 필요하지 않습니다.
- 인코딩은 계획에 명시되지 않았지만 검증 기준 "round-trip"을 결정론적으로 확인하기 위해
  storage canonical 규칙으로 정의했습니다(공개 파일 포맷이 아닌 로그·trace·FFI 전달용).

## 남은 위험 / 한계

- event에 항목 종류(파일·디렉터리·링크)는 담지 않습니다. 필요한 수신자는 경로를 다시 검사합니다.
- Windows adapter(XIII.2.3)는 ADR-0006에 따라 보류이며, 같은 모델로 normalize해야 합니다.
