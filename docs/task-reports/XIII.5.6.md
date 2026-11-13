# 작업 보고서: XIII.5.6 asset content GC

- WBS: XIII.5.6
- 작업명: project에서 참조되지 않는 asset content GC를 generation/chunk GC와 별도 mark-sweep으로
  구현(Git tracked history 미고려, current project refs만)
- 상태: 완료 (`[x]`)
- 선행: XIII.5.3, V.7.4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 파일 시스템 검증).

## 변경 파일 목록

- `crates/assets/src/gc.rs`(신규) — `mark_live_content`, `sweep_unreferenced_assets`,
  `AssetSweepOutcome`.
- `crates/assets/src/lib.rs` — `gc` 모듈 등록.
- `docs/task-reports/XIII.5.6.md`(신규), 계획서(XIII.5.6 `[ ]`→`[x]`).

## 구현 요약

- **별도 mark-sweep**: storage V.7 chunk/manifest GC(`design/` 아래)와 독립적으로 `assets/`
  아래 content 파일만 다룹니다. `design/objects`의 파일은 건드리지 않습니다.
- **mark**: 현재 project 참조(`AssetRecord` iterator, 예: `AssetCatalog`)의 content hash
  집합입니다. Git tracked history는 고려하지 않습니다. undo history가 되돌릴 수 있는 record
  (XIII.5.5 `previous`)도 호출자가 mark 입력에 넣어야 한다고 모듈 문서에 명시했습니다.
- **sweep**: `assets/<소문자 hex 2자>/`의 일반 파일만 봅니다. `AssetObjectPath::parse`로
  canonical content 이름인 것 중 hash가 live면 보존(같은 content를 여러 AssetId가 공유해도
  hash 하나로 보호), 아니면 mtime + grace period가 지난 뒤에만 삭제합니다. XIII.5.4의 강제
  종료 잔여 temp(`.…tmp-…`)도 grace 이후 정리합니다. 그 밖의 파일·디렉터리·symlink·비canonical
  이름(대문자 확장자 등)은 절대 삭제하지 않습니다. mtime 조회·삭제 실패는 결과에 기록하고
  다음 실행으로 이월합니다(idempotent).
- **시작 조건(V.7.4)**: storage `GcScheduler::try_begin_gc`가 발급한 `GcGuard`를 인자로
  요구합니다(idle + 저장 없음 증거). dedup import는 기존 파일 mtime을 바꾸지 않으므로
  import 호출자가 `note_activity`/저장 guard로 GC와 겹치지 않게 해야 한다고 명시했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-assets` | lib 90 passed + integration 1 passed + doctest 2 passed(compile_fail), 0 failed |
| `cargo clippy -p stellar-assets --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-assets -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **live shared content 미삭제**: 공유 content의 AssetId 하나를 지워도 남은 AssetId가 참조하는 파일 보존, orphan만 삭제, 재실행 idempotent(`live_shared_content_is_never_deleted`) | 통과 |
| grace 이내 unreferenced content 보존(import 직후·저장 전 보호) | 통과 |
| stale temp 삭제·grace 이내 temp 보존, README·비shard 디렉터리·하위 디렉터리·대문자 확장자·symlink 미삭제 | 통과 |
| 저장 중 GC guard 발급 거부(V.7.4), `design/objects` chunk 미삭제 | 통과 |
| `assets/` 없음 → no-op, shard·temp 이름 규칙 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- grace period는 인자로 받습니다. config(`config/storage-gc.yaml`의 `grace_period_seconds`
  공유 또는 asset 전용 값)에서 읽어 연결하는 것은 project 계층 작업입니다.
- mark 입력의 완전성(열린 문서·닫힌 문서·undo history·clipboard 등)은 호출자 책임입니다.
  누락하면 live content가 grace 이후 삭제될 수 있습니다.
- Git이 추적하는 content 파일을 지우면 working tree 변경으로 보입니다(지시대로 Git history는
  고려하지 않음). 사용자가 커밋해야 저장소에서도 제거됩니다.

## 후속 수정(리뷰 반영): symlink `assets/`·shard 디렉터리 거부

- **문제**: `collect`가 `read_dir(project_root/assets)`로 symlink `assets/`를 따라갔습니다.
  commit된 symlink가 grace 이후 canonical 이름 파일 삭제를 프로젝트 밖으로 돌릴 수 있었습니다.
- **수정**: `sweep_unreferenced_assets`가 `Result<AssetSweepOutcome, AssetGcError>`를 돌려줍니다.
  `assets/` 자체나 shard 이름(소문자 hex 2자) 항목이 symlink면(`lstat`·`DirEntry::file_type`,
  따라가지 않음) `AssetGcError::SymlinkedAssetsDirectory { path }`입니다. 목록 수집이 삭제보다
  먼저 끝나므로 오류 시 아무 것도 삭제하지 않습니다. shard 이름이 아닌 symlink는 기존처럼
  무시합니다(사용자 파일).
- **검증**: `symlinked_assets_root_or_shard_is_never_followed`(gc) — symlink shard·symlink
  `assets/` 모두 typed 오류, 프로젝트 밖 canonical 이름 파일과 프로젝트 안 orphan 모두 보존,
  비shard 이름 symlink는 무시하고 정상 sweep. 기존 GC 테스트는 `Ok` 결과로 같은 동작.

## 후속 수정(리뷰 반영): temp 정리 대상을 writer 이름 형식과 정확히 일치

- **문제**: GC가 `.`로 시작하고 `.tmp-`를 포함한 모든 숨김 파일을 stale temp로 보고 grace 이후
  삭제했습니다. writer(XIII.5.4)의 실제 형식 `.<canonical 이름>.tmp-<pid>-<n>`보다 넓어
  `.DS_Store.tmp-1-2` 같은 사용자 파일도 지울 수 있었습니다.
- **수정**: writer(`store::temp_path_for`) 옆에 같은 형식의 해석기 `store::parse_temp_file_name`을
  두고 GC가 그것만 씁니다. 인정 조건: 선행 `.` + **이 shard의 canonical content 이름**
  (`AssetObjectPath::parse`) + `.tmp-` + 선행 0 없는 `u32` pid + `-` + 선행 0 없는 `u64` 순번.
  표식 문자열은 상수 하나(`TEMP_MARKER`)로 writer와 공유합니다.
- **검증**: `writer_temp_names_round_trip_through_parser`(writer 출력이 해석기로 같은 hash, 다른
  shard는 거부), `shard_and_temp_name_rules`(비슷한 이름 16종 거부), `temp_cleanup_matches_writer_pattern_exactly`
  (writer temp만 삭제, `.DS_Store.tmp-1-2`·`.notes.tmp-1-1`·순번 누락·선행 0·`.bak` 삽입 이름 보존).
  `cargo test -p stellar-assets` lib 122 passed + integration 1 passed + doctest 7 passed,
  clippy 경고 0, fmt 통과, `validate-crate-graph` 통과.

## 후속 수정(보안 리뷰 반영): 삭제 직전 identity 재확인(TOCTOU 완화)

- **문제**: 목록 수집(`lstat`·`DirEntry`) 뒤 경로 기반 `remove_file`이라, 그 사이 shard·`assets/`를
  symlink로 바꾸면 삭제가 다른 위치로 갈 수 있었습니다.
- **수정**: 목록 수집 때 파일·shard·`assets/`의 `(dev, ino)`를 기록하고, 각 삭제 직전에 파일을 다시
  `lstat`해 같은 일반 파일인지(grace 판정 mtime도 이 값 사용), `assets/`·shard가 같은 실제 디렉터리인지
  확인합니다. 다르면 지우지 않고 `carry_over`로 이월합니다.
- **검증**: `entries_swapped_after_listing_are_not_deleted`(파일 rename 교체, shard를 밖 디렉터리
  symlink로 교체, `assets/` 교체 — 모두 삭제 없음·이월, 밖 파일 보존). 기존 GC 테스트 그대로 통과.
- **남은 한계**: 재확인 직후와 `remove_file` 사이의 짧은 창은 남습니다(std에 `unlinkat` 없음, XIII.5.2
  후속 수정 절 참조).
