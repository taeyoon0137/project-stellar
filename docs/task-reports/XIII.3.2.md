# 작업 보고서: XIII.3.2 ExternalChangePending 상태와 세 snapshot

- WBS: XIII.3.2
- 작업명: 로컬 변경이 있으면 LoadedBase, CurrentMemory, ExternalDisk snapshot을 보존한
  `ExternalChangePending` 상태를 생성
- 상태: 완료 (`[x]`)
- 선행: XIII.2.5(이 lane에서 완료), V.4.4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/external_change.rs` (신규: `DiskSnapshot`, `MemorySnapshot`, `SnapshotError`,
  `SnapshotHashes`, `ExternalChangePending`, `WindowDiskState`, `ExternalChangeDecision`,
  `ExternalChangeError`, `WindowExternalState`, 테스트)
- `crates/project/src/lib.rs` (`pub mod external_change;` 추가)
- `docs/task-reports/XIII.3.2.md`(신규), 계획서(XIII.3.2 `[ ]`→`[x]`).

## 구현 요약

- **LoadedBase·ExternalDisk**(`DiskSnapshot`): V.4.4 `verify_generation`을 통과한 완성된 generation의
  manifest hash와 manifest. generation 파일은 content-addressed·불변이라 hash로 보존되며,
  `retained_manifests()`(base, external)를 GC 보존 목록(V.7.2 `mark_reachable`)에 넣어 지워지지 않게
  합니다.
- **CurrentMemory**(`MemorySnapshot`): 창의 메모리 상태를 저장소 관점(logical chunk 상태 전체 + root
  document 목록)으로 캡처한 불변 snapshot(`Arc` 공유). hash는 같은 상태로 만든 SavePlan(V.5.1)의
  manifest content hash라 같은 상태는 항상 같은 hash입니다.
- **상태 기계**(`WindowExternalState`): `Clean → mark_dirty → Dirty`. 외부의 새 generation(XIII.2.5
  `ExternalChange::Generation`)을 받으면 clean 창은 `CleanWindow`(reload는 XIII.3.1), 로컬 변경이 있는
  창은 메모리를 캡처해 세 snapshot을 보존한 `ExternalChangePending`으로 바꾸고 세 hash를
  `Pending(SnapshotHashes)`로 돌려줍니다. 이미 대기 중이면 LoadedBase는 유지하고 ExternalDisk만 최신으로
  바꿉니다. 외부 generation이 검증되지 않거나 캡처가 실패하면 상태를 바꾸지 않습니다. 대기 중에는 저장
  성공 기록(`mark_saved`)으로 상태가 풀리지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 238 passed, 0 failed; fuzz 통합 4 passed) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **세 snapshot hash 기록**: 로컬 변경 + 외부 generation → `Pending` 결정과 대기 상태가 같은 세 hash(base = 연 generation, memory = 같은 상태의 저장 계획 manifest hash, external = 외부 generation)를 기록, 세 hash 서로 다름, 세 snapshot 내용 보존, GC mark에 두 디스크 generation 포함, 대기 중 `mark_saved` 거부(`local_changes_create_pending_state_with_three_snapshot_hashes`) | 통과 |
| 같은 메모리 상태 → 같은 hash, 다른 상태 → 다른 hash(`same_memory_state_gives_same_hash`) | 통과 |
| clean 창은 대기 상태가 아님(메모리 캡처 없음), base와 같은 generation은 `Unchanged`(`clean_window_is_not_pending_and_base_is_unchanged`) | 통과 |
| 대기 중 더 새 외부 generation → LoadedBase 유지, ExternalDisk 갱신(`later_external_generation_keeps_loaded_base`) | 통과 |
| 검증 안 되는 외부 generation → 상태 불변(`unverifiable_external_generation_leaves_state_unchanged`) | 통과 |
| XIII.2.5 coalescer 결과가 대기 상태를 만듦(`coalesced_external_generation_drives_pending_state`) | 통과 |

## 차이(작업 지시와 다른 점)

- 창 세션(XII.2)과 문서 runtime이 아직 없어 CurrentMemory는 문서 노드가 아닌 저장소 관점(logical chunk
  상태)으로 캡처합니다. 문서 thread가 저장 직렬화와 같은 입력으로 캡처하도록 연결하는 것은 XII.1·XIII.1
  작업입니다.

## 남은 위험 / 한계

- 메모리 snapshot은 chunk payload 전체를 들고 있으므로 큰 문서에서는 메모리를 차지합니다(공유 `Arc`로
  복사는 없음).
- GC가 `retained_manifests`를 실제로 보존 목록에 넣는 연결은 GC 실행 경로(V.7) 호출자가 합니다.
