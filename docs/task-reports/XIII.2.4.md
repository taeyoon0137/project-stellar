# 작업 보고서: XIII.2.4 자기 저장 event 식별

- WBS: XIII.2.4
- 작업명: Stellar 자신의 save가 만든 filesystem event를 generation/hash로 식별해 외부 변경으로
  재처리하지 않게 함
- 상태: `[~]` 유지 — 구현 판정 통과, 선행 미충족(2026-11-23 재판정, 아래 "재판정" 절). 이전 기록: 부분 완료 (`[~]`) — macOS 경로 구현·검증 완료, 선행 XIII.2.3(Windows adapter)은 ADR-0006으로
  `[!]`
- 선행: V.5.4, XIII.2.2(이 lane에서 완료), XIII.2.3(`[!]`, ADR-0006)

## 필수 실행 환경

- `DEV-MAC` (판정 로직은 플랫폼 독립, 실제 감시 E2E는 `cfg(target_os = "macos")`).

## 변경 파일 목록

- `crates/project/src/watcher/own_save.rs` (신규: `OwnSaveLedger`, `OwnSave`, `CurrentPointerTracker`,
  `PointerObservation`, `current_pointer_watch_path`, `may_change_current_pointer`, 테스트)
- `crates/project/src/watcher/event.rs` (`WatchPath::root_of` 추가)
- `crates/project/src/watcher/mod.rs` (`own_save` 모듈 추가)
- `docs/task-reports/XIII.2.4.md`(신규), 계획서(XIII.2.4 `[ ]`→`[~]`).

## 구현 요약

- **자기 저장 기록**: `OwnSaveLedger`는 pointer 전환(V.5.4)에 성공한 저장의 (generation, manifest
  hash)를 최근 64개 기록합니다. `record_saved(&SavedGenerationState)`로 commit 직후 기록합니다.
- **판정**: 감시 event가 current pointer를 바꿨을 수 있으면(`design/current` 자체·그 상위 경로·
  `design/current`로의 rename·넘침) `CurrentPointerTracker::observe`가 디스크 pointer를 다시 읽습니다.
  마지막 관찰 값과 같으면 `Unchanged`, ledger에 있으면 `OwnSave`, 없으면 `External`, 없어지면
  `Missing`, 형식 오류면 `Invalid`입니다. chunk·manifest·임시 파일 event만으로는 pointer를 읽지
  않습니다.
- 판정 기준은 content-addressed manifest hash(ADR-0004)와 generation이며 파일 이름·시각·프로세스를
  쓰지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 218 passed, 0 failed; fuzz 통합 4 passed) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |
| `cargo test -q -p stellar-project autosave_events_cause` 3회 반복 | 3회 모두 1 passed |

| 검사 | 결과 |
|---|---|
| **autosave 후 reload loop 0**: 실제 FSEvents 감시 중 실제 저장 파이프라인(V.5 `run_save`)으로 autosave 5회 → 매번 current pointer event를 받고도 reload 0, 다른 writer의 저장은 정확히 reload 1(`autosave_events_cause_no_reload_loop`) | 통과 |
| 자기·외부 generation 판정, 반복 관찰 `Unchanged`, 손상 `Invalid`, 삭제 `Missing`(`tracker_classifies_own_and_external_generations`) | 통과 |
| ledger 중복 무시·오래된 기록 제거·saved state 기록(`ledger_identifies_own_saves_and_evicts_old_ones`) | 통과 |
| pointer 관련 event 판정(chunk·manifest·임시 파일·외부 root는 무관)(`pointer_relevance_of_events`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 XIII.2.3(Windows watcher adapter)은 ADR-0006으로 `[!]`입니다. 이 작업의 판정 로직은 플랫폼
  독립이고 macOS adapter(XIII.2.2)로 E2E 검증했지만, Windows adapter가 같은 event를 내는지는 확인할 수
  없어 `[~]`로 둡니다.

## 남은 위험 / 한계

- Windows 재개 시 Windows adapter event로 같은 E2E(autosave 후 reload 0)를 다시 검증해야 합니다.
- 같은 프로젝트를 연 다른 Stellar 창의 저장은 이 session의 ledger에 없으므로 `External`로 판정됩니다.
  기술 아키텍처 22.5의 writer session metadata 구분(22.2 두 선택지 정책)은 다중 창 작업(XII.7)에서
  연결합니다.
- ledger는 최근 64개만 기억합니다. 그보다 오래된 자기 저장으로 pointer가 되돌아가면(예: Git
  checkout) 외부 변경으로 처리되며, 이는 의도된 보수적 동작입니다.

## 재판정 (2026-11-23, ledger-verification)

- 판정: `[~]` 유지 — 구현 판정 통과, 선행 미충족.
- 근거: 자기 저장 식별은 플랫폼 독립 판정(generation·manifest hash)이고 창 session에 연결되어 있습니다. 선행 XIII.2.3(Windows adapter)이 ADR-0006으로 `[!]`라 `[~]`를 유지합니다.
- 판정 기준: 행 지시·검증 열의 불변식을 구현 코드에서 확인했고, 테스트 통과는 보조 근거로만 썼습니다. 계획서 §1.2 규칙 4(선행이 모두 `[x]`여야 완료)를 적용했습니다. 기준 커밋은 `main` `ce60bb92`입니다.

### 확인한 코드 위치

- `crates/project/src/watcher/own_save.rs`: `OwnSaveLedger`(최근 64개), `CurrentPointerTracker::observe`(Unchanged·OwnSave·External·Missing·Invalid), 실제 FSEvents E2E `autosave_events_cause_no_reload_loop`.
- `crates/project/src/window_session.rs`: 창 session이 ledger를 소유하고 저장 성공 때 `record_saved`를 부릅니다(458행).

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test --workspace`(DEV-MAC, 샌드박스 해제) | 테스트 binary 142개 모두 ok, 4136 passed, 0 failed |
| `cargo test -p stellar-project` | 392 passed, 0 failed |

### 남은 조건과 finding

- Windows 선행 재작성안: XIII.2.4의 선행에서 XIII.2.3을 빼고 "V.5.4, XIII.2.2"로 둡니다. 판정은 adapter가 정규화한 event만 쓰므로 Windows adapter가 생기면 그 행(XIII.2.3)의 검증에 "autosave 후 reload 0" E2E를 더하면 됩니다(ADR-0006 결정 4의 macOS 부분 판정).
