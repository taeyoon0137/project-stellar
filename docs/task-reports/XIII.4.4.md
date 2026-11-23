# 작업 보고서: XIII.4.4 외부 symlink target 감시 동적 등록·해제

- WBS: XIII.4.4
- 작업명: 외부 symlink target 변경을 watcher에 동적으로 등록·해제
- 상태: 완료 (`[x]`) — 2026-11-23 library-finish, 아래 "작업 (2026-11-23, library-finish)" 절. 이전 기록: `[~]` 유지 — 남은 범위·결함 있음(2026-11-23 재판정, 아래 "재판정" 절). 이전 기록: 부분 완료 (`[~]`) — macOS adapter로 구현·검증 완료, 선행 XIII.2.3(Windows adapter)은 ADR-0006으로
  `[!]`
- 선행: XIII.2.2(이 lane에서 완료), XIII.2.3(`[!]`, ADR-0006), XIII.4.1

## 필수 실행 환경

- `DEV-MAC` (등록 판단 로직은 플랫폼 독립, 실제 감시 E2E는 `cfg(target_os = "macos")`).

## 변경 파일 목록

- `crates/project/src/watcher/targets.rs` (신규: `desired_external_targets`, `ExternalTargets`,
  `ExternalTargetWatch` trait, `ExternalTargetSync`, `SyncReport`, `SyncError`, macOS adapter 구현, 테스트)
- `crates/project/src/watcher/fsevents.rs` (`FsEventsWatcher::add_external_target`·`remove_external_target`,
  대기열 알림에 stream root 표시, 해제된 root의 늦은 알림 버림, 겹친 root의 중복 알림 제거)
- `crates/project/src/watcher/mod.rs` (`targets` 모듈 추가)
- `docs/task-reports/XIII.4.4.md`(신규), 계획서(XIII.4.4 `[ ]`→`[~]`).

## 구현 요약

- **감시할 target**: `desired_external_targets(resolver)`는 resolver 트리 순회(XIII.4.2)에서 프로젝트
  안에서 밖으로 처음 넘어가는 항목의 실제 target을 모읍니다(외부 디렉터리 하위 항목은 그 root가 포함,
  `.git` 제외, 같은 target으로 가는 링크들은 하나로). **비신뢰 프로젝트는 비어 있습니다** — 외부
  target의 하위 이름·변화도 외부 내용이므로 capability matrix(XVI.7.1)의 외부 symlink 읽기 금지를
  따릅니다.
- **동적 등록·해제**: `ExternalTargetSync::sync`는 원하는 target과 현재 감시 root를 비교해 새 target은
  `add_external_target`(새 FSEvents stream), 더 이상 가리키지 않는 target은 `remove_external_target`
  (stream 정지)으로 맞춥니다. 해제된 root에서 이미 대기열에 들어온 알림은 버리므로 넘침으로 바뀌지
  않습니다.
- **다시 맞추는 시점**: `needs_resync`는 넘침, 알려진 링크(또는 그 상위 경로)의 변화, 새로 생기거나
  바뀐 경로가 symlink일 때만 `true`입니다. 일반 파일·외부 root 안의 변화는 다시 훑지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 232 passed, 0 failed; fuzz 통합 4 passed) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |
| `cargo test -q -p stellar-project watcher::` 4회 반복 | 4회 모두 33 passed |

| 검사 | 결과 |
|---|---|
| **target replace 후 event 수신**(실제 FSEvents): 처음 target 안의 생성 수신 → 링크를 다른 외부 디렉터리로 교체 → 링크 event로 새 target 등록·이전 target 해제 → 새 target의 생성 수신, 이전 target 변화는 수신 안 함(`replaced_target_is_watched_after_dynamic_registration`) | 통과 |
| 감시할 target 계산: 최상위 외부 target만, 내부 링크 경유 합침, `.git` 제외, 비신뢰는 비어 있음(`desired_targets_are_top_level_external_targets`) | 통과 |
| 교체·삭제·비신뢰 전환에 따른 등록·해제(`sync_registers_and_unregisters_as_links_change`) | 통과 |
| 재동기화 판단: 일반 파일·외부 root 변화는 아님, 링크·상위 디렉터리·새 symlink·넘침은 맞춤(`resync_is_needed_only_for_link_changes_or_overflow`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 XIII.2.3(Windows watcher adapter)은 ADR-0006으로 `[!]`입니다. macOS adapter로 검증을 마쳤지만
  Windows adapter의 동적 등록은 확인할 수 없어 `[~]`로 둡니다.
- 비신뢰 프로젝트의 외부 target 감시 금지는 작업 지시에 없지만 XIII.4.5·XVI.7.1 정책과 맞추기 위해
  추가했습니다.

## 남은 위험 / 한계

- 재동기화는 프로젝트 트리 전체를 다시 훑습니다. 링크가 많이 바뀌는 큰 프로젝트에서는 비용이 큽니다.
- 외부 target 디렉터리 자체가 이동·삭제되면 FSEvents의 root 변화(넘침)로 보고되며, 그 뒤 rescan
  (XIII.2.6)과 재동기화가 필요합니다.
- Windows 재개 시 같은 E2E를 Windows adapter로 다시 검증해야 합니다.

## 재판정 (2026-11-23, ledger-verification)

- 판정: `[~]` 유지 — 남은 범위·결함 있음.
- 근거: 동적 등록·해제 API와 실제 FSEvents E2E는 있습니다. 그러나 제품 감시 경로가 이 API를 부르지 않고, 선행 XIII.2.3이 `[!]`라 `[~]`를 유지합니다.
- 판정 기준: 행 지시·검증 열의 불변식을 구현 코드에서 확인했고, 테스트 통과는 보조 근거로만 썼습니다. 계획서 §1.2 규칙 4(선행이 모두 `[x]`여야 완료)를 적용했습니다. 기준 커밋은 `main` `ce60bb92`입니다.

### 확인한 코드 위치

- `crates/project/src/watcher/targets.rs`: `desired_external_targets`, `ExternalTargetSync::sync`, `needs_resync`, 비신뢰 프로젝트는 빈 target.
- `crates/project/src/watcher/fsevents.rs` `add_external_target`·`remove_external_target`.
- `ExternalTargetSync`·`desired_external_targets`를 부르는 곳이 `targets.rs` 밖에 없습니다(`crates/project/src/external_change.rs`·`window_session.rs`·runtime 모두 미사용).

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test --workspace`(DEV-MAC, 샌드박스 해제) | 테스트 binary 142개 모두 ok, 4136 passed, 0 failed |
| `cargo test -p stellar-project` | 392 passed, 0 failed |

### 남은 조건과 finding

- 결함 finding(window-session-stale lane 소유 경로 `crates/project/src/{external_change,window_session}.rs`, `watcher/`): 창 session의 외부 변경 감시 loop가 넘침·링크 변화 event 뒤 `ExternalTargetSync::sync`를 불러야 제품에서 외부 symlink target 변경을 받습니다.
- Windows 선행 재작성안: XIII.4.4의 선행에서 XIII.2.3을 빼고 "XIII.2.2, XIII.4.1"로 둡니다.

## 작업 (2026-11-23, library-finish): 제품 감시 경로 연결 재판정

### 한 일

- 코드는 바꾸지 않았습니다(문서만 변경). 재판정 당시 결함이었던 "제품 감시 경로가 `ExternalTargetSync::sync`를 부르지 않음"은
  `stellar-runtime` 외부 변경 감시(`crates/runtime/src/app/autosave/external.rs`, 커밋 `ba9dc68e`·`734b8314`)가 해소했습니다:
  `ProjectWatch::start`가 처음 열 때 `resync_targets`로 맞추고, `poll`이 `needs_resync`(넘침·링크 변화) 뒤 다시 맞춥니다. 비신뢰
  프로젝트는 resolver가 빈 target을 줍니다.
- 계획서·원장 선행은 XIII.2.2, XIII.4.1뿐이고(XIII.2.3 없음) 둘 다 `[x]`입니다.

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-project watcher::targets` | `replaced_target_is_watched_after_dynamic_registration` 포함 통과 |

| 검사 | 결과 |
|---|---|
| **target replace 후 event 수신**(실제 FSEvents, 기존 시험) | 통과 |
| 제품 경로 호출 확인: `external.rs`의 `ExternalTargetSync::new()`·`sync`·`needs_resync` 호출(코드 읽기) | 확인 |
