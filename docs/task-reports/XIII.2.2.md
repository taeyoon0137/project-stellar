# 작업 보고서: XIII.2.2 macOS 파일 감시 adapter

- WBS: XIII.2.2
- 작업명: macOS watcher adapter를 구현하고 프로젝트 root, 외부 symlink target allowlist를 감시
- 상태: 완료 (`[x]`)
- 선행: XIII.2.1(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (macOS FSEvents 실제 stream fixture는 `cfg(target_os = "macos")`, normalize 규칙
  테스트는 플랫폼 독립).

## 변경 파일 목록

- `crates/project/src/watcher/roots.rs` (신규: `WatchRoots`, `WatchRoot`, `WatchRootKind`,
  `WatchRootError`, 경로 변환·파일 root 형제 판정, 테스트)
- `crates/project/src/watcher/fsevents.rs` (신규: FSEvents flag, `RawFsEvent`, `PathProbe`,
  `Normalizer`, macOS `FsEventsWatcher`·`WatcherError`, 실제 stream fixture 테스트)
- `crates/project/src/watcher/mod.rs` (`roots`·`fsevents` 모듈 추가)
- `crates/project/src/watcher/event.rs` (문서 주석의 깨진 intra-doc link 한 줄 수정)
- `docs/task-reports/XIII.2.2.md`(신규), 계획서(XIII.2.2 `[ ]`→`[x]`).

## 구현 요약

- **감시 root**: `WatchRoots`가 프로젝트 root(id 0)와 외부 symlink target allowlist(id 1부터)를
  실제 경로로 보관합니다. 프로젝트 root 안의 경로는 외부 대상으로 받지 않고, 중복 등록은 기존 id를
  돌려줍니다. 파일 target은 부모 디렉터리를 감시하고 그 파일 경로만 받습니다(형제 알림은 버림).
  경로가 여러 root에 속하면 가장 구체적인 root로 바꿉니다.
- **FSEvents adapter**: root마다 FSEvents stream 하나(`NoDefer | WatchRoot | FileEvents`, latency
  20ms, 직렬 dispatch 대기열). callback은 원시 알림을 공유 대기열에 넣기만 하며 대기열 상한을 넘으면
  버리고 `Overflow(QueueFull)`을 한 번 보고합니다. 새 crate 의존 없이 CoreServices·CoreFoundation
  framework와 libdispatch C 함수만 선언했습니다. 정리는 stop → invalidate → release → 대기열 동기
  비움 → callback 문맥 해제 순서입니다.
- **normalize**(`Normalizer`, 플랫폼 독립): drop·재검사·mount·root 변화는 넘침으로, 연속 id의
  rename 두 건은 `Renamed`로 짝짓고, 존재하지 않으면 `Removed`입니다. FSEvents 항목 flag가 누적되는
  특성(최근 만든 파일을 고쳐도 생성 flag가 옴) 때문에, 생성 flag가 있어도 이번 감시에서 이미 보고한
  경로이거나 생성 시각(birth time)이 감시 시작보다 이르면 `Modified`로 보고합니다. 표현할 수 없는
  경로는 버리지 않고 `Overflow(UnrepresentablePath)`입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 214 passed, 0 failed; fuzz 통합 4 passed) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |
| `cargo test -q -p stellar-project watcher::fsevents` 6회 반복 | 6회 모두 7 passed(실제 stream 테스트 flake 없음) |

| fixture | 결과 |
|---|---|
| **프로젝트 root 실제 FSEvents**: create(`Created`)·감시 전부터 있던 파일 modify(`Modified`)·rename(`Renamed{from,to}`)·delete(`Removed`), 모든 경로가 root 0 기준(`project_root_create_modify_delete_rename`) | 통과 |
| **외부 allowlist 디렉터리**: modify·rename·delete가 root 1 기준으로 수신, allowlist 밖 디렉터리 알림 없음, 넘침 없음(`external_allowlist_targets_are_watched_and_others_are_not`) | 통과 |
| 외부 파일 target만 감시(형제 파일 알림·넘침 없음)(`external_file_target_is_watched_alone`) | 통과 |
| normalize 규칙: flag→event, 누적 flag 보정, 넘침, root 사이 rename 짝(`normalization_*`, `accumulated_flags_*`, `rename_across_roots_*`) | 통과 |
| 감시 root 변환·중복·제거·프로젝트 내부 거부·UTF-8 아닌 이름(`watcher::roots::tests`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 감시 대상 동적 추가·해제는 XIII.4.4 범위라 이 작업은 시작 시 받은 allowlist만 감시합니다.
- allowlist는 호출자가 넘깁니다. 비신뢰 프로젝트에서 외부 target을 감시할지는 XIII.4.4에서 capability
  matrix(XVI.7.1)로 정합니다.

## 남은 위험 / 한계

- FSEvents는 같은 경로의 짧은 시간 변화를 묶어 보고하므로 "생성 후 즉시 수정"은 `Created` 하나로,
  "삭제 후 즉시 재생성"은 `Created`로 보일 수 있습니다. 수신자(XIII.2.4~XIII.2.6)는 event를 "이 경로를
  다시 확인" 신호로 다룹니다.
- 생성 시각 비교는 시스템 시계에 의존합니다. 감시 시작 직전·직후에 만든 파일은 판정이 바뀔 수
  있습니다(내용 확인은 hash로 하므로 결과 정확성에는 영향 없음).
- Windows adapter(XIII.2.3)는 ADR-0006에 따라 보류입니다.
