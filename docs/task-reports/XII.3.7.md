# 작업 보고서: XII.3.7 interaction preview transient 주입과 pointer up 확정

- WBS: XII.3.7
- 작업명: interaction preview를 transient state로 Scene builder에 주입하고 pointer up에서만 CommandTransaction 확정
- 상태: 부분 완료 (`[~]`)
- 결정: ADR-0084 결정 6
- 선행: VII.4.2(`[x]`), XI.2.1(이 lane). 이전 차단 보고서 `XII.3.7-blocked.md`의 재개 조건 1·2를 이 lane이 충족

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 측정 장비는 개발 장비(Apple M3 Max, 36GB)이고 기준 장비(M1 16GB)가 아닙니다.

## 변경 파일 목록

- `crates/scene/src/incremental.rs` — `ScenePreview`, `set_preview`·`clear_preview`·`commit`.
- `tools/xtask/src/replay/runner.rs` — 상태 기계 preview → scene preview, 뗌 결과 → `CommandTransaction` 하나.
- `docs/task-reports/XII.3.7.md`(신규).

## 구현 요약

- preview는 key별 대체 입력입니다. snapshot의 document revision은 그대로이고 scene revision만 올립니다(정의된 preview revision). source·history에 들어가지 않습니다.
- pointer up에서 `InteractionOutcome`(Drag·Resize)을 `CommandTransaction` 하나로 실행합니다. Drag는 transform이고 Resize는 transform과 width·height입니다. `IncrementalScene::commit`이 preview 제거와 확정 편집을 한 build로 반영합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-scene` | lib 151 passed, 통합 3 passed(1 ignored), 0 failed |
| `cargo test -p xtask --lib replay` | 7 passed, 0 failed |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `preview_changes_only_the_scene_revision_and_is_discarded_on_clear`: drag preview 500회 동안 document revision 그대로, scene revision 증가, 매번 rebuilt 1, source 그대로, clear 뒤 원래 hash. 종류가 다르거나 없는 node의 preview는 거부.
- `commit_clears_preview_and_applies_the_confirmed_edit_in_one_build`.
- `drag_commits_once_and_previews_keep_the_document_revision`(replay): 이동 60회 동안 document revision 0, 뗌 뒤 1(확정 1건). Pan은 문서와 scene을 바꾸지 않음.

## 성능

- preview 이동 한 번은 node 1개 재평가입니다(scene build p99 약 0.6ms, 2만 node).

## 차이(작업 지시와 다른 점)

- 앱(AppKit) 입력을 receipt와 함께 DocumentThread로 보내 실제 hit test로 상태 기계를 구동하는 재개 조건 3은 하지 않았습니다. 사용자 방향(2026-11-23)이 UI를 나중에 재설계하므로, core replay가 같은 순서를 구동합니다.

## 남은 위험 / 한계

- 앱 경로가 없으므로 `[~]`입니다. Rotate 확정 command와 history 항목 확인은 core replay에 없습니다(Drag·Resize만).
