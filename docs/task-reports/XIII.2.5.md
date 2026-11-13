# 작업 보고서: XIII.2.5 다중 파일 event coalescing

- WBS: XIII.2.5
- 작업명: 짧은 시간의 다중 파일 event를 root manifest current pointer 변경 단위로 coalesce, 개별
  chunk event만으로 reload하지 않음
- 상태: 완료 (`[x]`)
- 선행: V.4.3, XIII.2.4(이 lane에서 `[~]` — macOS 부분 완료)

## 필수 실행 환경

- `DEV-MAC` (coalescing 로직은 플랫폼 독립, 실제 감시 E2E는 `cfg(target_os = "macos")`).

## 변경 파일 목록

- `crates/project/src/watcher/coalesce.rs` (신규: `ChangeCoalescer`, `ExternalChange`, 테스트)
- `crates/project/src/watcher/own_save.rs` (테스트 모듈·fixture 상수를 `pub(crate)`로 — coalesce 테스트에서
  같은 저장 fixture 재사용, 테스트 내용 변경 없음)
- `crates/project/src/watcher/mod.rs` (`coalesce` 모듈 추가)
- `docs/task-reports/XIII.2.5.md`(신규), 계획서(XIII.2.5 `[ ]`→`[x]`).

## 구현 요약

- `ChangeCoalescer::push(events, now)`는 event를 모으고, `poll(now, ledger)`는 마지막 event 뒤
  조용한 구간(`quiet_window_nanos`)이 지났을 때만 결과를 냅니다. 시간은 단조 nanoseconds로
  주입합니다(`ManualClock`과 같은 단위).
- **저장소 영역(`design/`)** event(chunk·manifest·임시 파일·pointer)는 그 자체로 결과를 만들지
  않습니다. current pointer가 바뀌었을 수 있을 때만 XIII.2.4 tracker로 판정해, 외부 writer의 새
  generation이면 `ExternalChange::Generation` **하나**(그 구간의 저장소 경로 목록 포함)를 냅니다.
  자기 저장과 pointer 불변은 결과 없음입니다.
- **완성된 snapshot만**: 새 generation은 V.4.4 `verify_generation`(manifest hash·project id·chunk
  존재)을 통과할 때만 냅니다. manifest·chunk가 아직 없으면 기다렸다가 다음 event 뒤 다시 확인하고,
  손상(hash 불일치 등)이면 `InvalidGeneration`(자동 reload 금지), pointer 없음·형식 오류는
  `PointerUnavailable`입니다.
- 저장소 밖 파일(`stellar.yaml`, 에셋, 외부 symlink target root)은 구간마다 `Files` 하나로 묶습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 225 passed, 0 failed; fuzz 통합 4 passed) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |
| `cargo test -q -p stellar-project real_external_save` 3회 반복 | 3회 모두 1 passed |

| 검사 | 결과 |
|---|---|
| **한 save가 한 external change batch**: 실제 저장 파이프라인으로 만든 외부 generation의 실제 순서 event(chunk 2·manifest·임시 파일·pointer rename)가 구간 전에는 결과 0, 구간 뒤 `Generation` 정확히 1개, 이후 0(`one_external_save_is_one_change_batch`) | 통과 |
| **실제 FSEvents E2E**: 외부 writer 저장 1회 → `Generation` 정확히 1개(늦은 event 추가 확인 포함)(`real_external_save_produces_one_batch`) | 통과 |
| chunk·manifest event만, 자기 저장 → 결과 없음(`chunk_only_events_and_own_saves_do_not_reload`) | 통과 |
| 한 구간 안의 저장 2회 → 최신 generation 1개, 구간을 넘긴 저장 → 각각 1개(`saves_within_one_window_coalesce_and_separate_saves_do_not`) | 통과 |
| pointer가 먼저 바뀌고 chunk가 늦게 오는 경우 기다렸다가 1개(`incomplete_generation_waits_for_its_chunks`) | 통과 |
| 손상 generation → `InvalidGeneration`, pointer 삭제 → `PointerUnavailable(Missing)`(`corrupt_generation_and_missing_pointer_are_reported_not_reloaded`) | 통과 |
| 저장소 밖 파일·외부 root 경로 묶음(`non_storage_files_are_grouped_per_quiet_window`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 XIII.2.4는 이 lane에서 `[~]`(macOS 부분 완료, Windows adapter 보류)입니다. 이 작업은 macOS
  부분만 필요하고 로직이 플랫폼 독립이라 `[x]`로 둡니다.
- "짧은 시간"의 구간 길이는 기술 아키텍처에 수치가 없어 상수로 정하지 않고 생성자 인자로 받습니다.
  테스트는 50ms(결정론 테스트)·150ms(실제 감시)를 썼습니다. 제품 기본값은 자동 저장 수치와 함께
  벤치마크로 정합니다(기술 아키텍처 42장 2·9번).

## 남은 위험 / 한계

- 계속 event가 이어지면(대규모 Git checkout 등) 조용한 구간이 오지 않아 결과가 미뤄집니다. 최대 지연은
  두지 않았습니다(완성된 snapshot 단위 전환 원칙 우선).
- 미완성 generation을 기다리는 동안 새 event가 오지 않으면 결과가 나오지 않습니다. 넘침·rescan(XIII.2.6)이
  보완합니다.
