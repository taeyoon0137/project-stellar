# 작업 보고서: XV.7.4 승인 후 lockfile·in-memory library snapshot 갱신과 override StableId 재적용

## WBS

- WBS: XV.7.4
- 작업명: 사용자 승인 후 lockfile과 in-memory library snapshot을 원자적으로 갱신, 사용 project document override는 StableId로
  재적용
- 상태: 완료 (`[x]`)
- 선행: XV.7.3(`[x]`), XV.4.5(`[x]`)
- 결정: [ADR-0137](../adr/ADR-0137-library-stored-refs-and-materialize-commands.md) 결정 4·5·6

## 변경 파일

- `crates/ops/src/library_materialize.rs` — `update`(`UpdateRequest`·`UpdateReport`·`DiffCounts`): 한 의존성만 다시 해석하고
  나머지는 lock commit, 저장된 모든 문서의 외부 instance를 후보에 대어 봄(`check_candidate_usages`), 끊기는 참조는 승인
  (`allow_orphans`) 없이 거부, `dry_run`, resource diff 개수(XV.7.2 `diff_library_snapshots`).
- `crates/ops/src/library_watch.rs`(신규) — `LibraryStamp`·`CacheLocation`: lock·`stellar.yaml`·cache key·entries 지문(받지 못한
  node가 있을 때만 cache index).
- `crates/ops/src/lib.rs` — 모듈 등록.
- `crates/runtime/src/library.rs` — `WindowLibraries`가 쓸 때마다 지문을 비교해 바뀌면 다시 로드(세대 목록, `&self`),
  `open_in_app_data`·`fixed`·`refresh`·`reloads`·`document_diagnostics`. `open_with` 대체.
- `crates/runtime/src/library_tests.rs`(신규).
- `crates/ops/src/library_materialize_tests.rs` — update 적용 시험.
- `docs/task-reports/XV.7.4.md`(신규).

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops --lib library_materialize` | 0 |
| `cargo test -p stellar-ops --lib library_watch` | 0 — 1 passed |
| `cargo test -p stellar-runtime --lib library::tests` | 0 — 1 passed |
| `cargo clippy -p stellar-ops -p stellar-runtime --all-targets --features stellar-ops/test-support -- -D warnings` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: 없음(DEV-MAC 로컬 시험, 로컬 `file://` 저장소)

## 테스트

| 검사 | 결과 |
|---|---|
| **update 후 snapshot 기대값**: v1 → v2 dry-run은 이전·새 commit·diff(added 1)·orphan 0을 내고 파일 불변. 적용하면 lock commit = v2, `stellar.yaml` 선택자 = v2, 저장된 Card 참조 유지. 다시 연 library는 v2 snapshot이고 Card 참조가 같은 StableId로 해석되어 문서 참조 진단 0(`update_reviews_then_applies_and_reapplies_stored_refs_by_stable_id`) | 통과 |
| v3(Card 삭제)는 승인 없이 `OrphansBlocked`(ComponentRemoved, Card instance), 승인하면 적용하고 끊긴 참조는 `library-ref/unresolved` 진단 | 통과 |
| **in-memory snapshot 갱신**: 열린 창(`WindowLibraries::open_in_app_data`)은 바뀐 것이 없으면(다른 열기가 cache index 최근 사용 시각만 바꿔도) 다시 로드하지 않고, 다른 프로세스의 update 뒤 다음 사용 때 v2 snapshot, import 뒤 새 저장 참조, 빈 cache 기기는 fetch 뒤 완전한 로드. 위치를 모르는 library는 다시 로드하지 않음(`open_window_reloads_when_lock_manifest_or_cache_changes`) | 통과 |
| 지문: lock·`stellar.yaml`·entry 공개는 바뀜, cache index는 완전한 로드에서 무시하고 불완전할 때만 봄(`library_watch::tests`) | 통과 |

## 성능

- 창의 지문 비교는 instance command 사전 검사·진단·상태 조회 때 파일 metadata 4\~5개입니다. 일반 편집 경로는 지문을 보지 않습니다.
  바뀌었을 때만 entry 검증 비용(ADR-0119 Performance)을 냅니다.

## 차이

- "원자적으로 갱신": `stellar.yaml`과 lock은 두 파일이라 한 rename으로 바꿀 수 없습니다. 모든 검증을 쓰기 전에 끝내고
  `stellar.yaml` → lock 순서로 쓰며 lock 쓰기가 실패하면 `stellar.yaml`을 되돌립니다(ADR-0137 결정 5, 실패 시험은 XV.7.5).
- "in-memory snapshot": 앱 UI가 없으므로 창 session의 `WindowLibraries`가 다음 사용 때 다시 로드하는 것으로 검증했습니다. 승인 화면은
  UI 재설계 때 `UpdateReport`(dry-run)를 입력으로 씁니다.
- "override StableId 재적용": override는 이미 property StableId로 저장되어 있어 문서를 다시 쓰지 않습니다. 새 snapshot에 같은 StableId로
  다시 대어지는지를 진단·사전 검사 경로로 확인했습니다. 텍스트 원본 library가 Property를 아직 담지 못해(ADR-0088 결정 2) property
  override가 있는 fixture는 이 시험에 없습니다(override 판정 자체는 XV.7.3 시험이 generation library로 확인).

## 남은 위험

- 창이 지난 로드 세대를 닫힐 때까지 들고 있습니다(lock·cache 변경 횟수만큼).
- render·semantic snapshot을 새 library로 다시 만드는 앱 경로(외부 instance 펼치기)는 앱 runtime이 library component를 scene에
  쓰게 될 때 연결합니다.
