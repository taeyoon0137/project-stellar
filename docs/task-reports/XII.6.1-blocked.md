# 차단 보고서: XII.6.1 New Project command

## WBS

- WBS: XII.6.1
- 작업명: New Project native command가 ProjectCreateRequest를 호출하고 폴더 선택과 이름 입력 결과만 core에 전달합니다.
- 상태: `[!]`
- 선행: IV.4.7 `[x]`, XII.2.2 `[x]`, XII.2.3 `[!]`

## 실패 선행조건

- XII.2.3(Windows 창 session)이 Windows 지원 보류(ADR-0006)로 `[!]`입니다. 이 행은 Windows `[!]` 행을 선행으로 두므로
  ADR-0006 아래에서는 완료로 올릴 수 없습니다(계획 개정 기록 2026-11-14 "새 macOS 행의 Windows `[!]` 선행 사슬").
- 2026-11-14 통합에서 macOS 구현만으로 `[~]`로 올렸고, 2026-11-15 검토에서 `[!]`로 고쳤습니다.

## 재현 명령

- `rg -n '^\| (XII\.2\.3|XII\.6\.1) ' docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` — XII.2.3 `[!]`.
- `cargo test -p stellar-ffi create_via_abi_reopens_in_an_editable_window_and_preserves_existing_folder` — macOS 구현의 C ABI
  검사입니다. 2026-11-15 DEV-MAC의 `cargo test --workspace`(종료 0)에 포함되어 통과했습니다.
- `scripts/macos/smoke.sh` — 2026-11-15 DEV-MAC 실행에서 `shell-integration ok: 176 checks`(메뉴·시작 창의 생성과 열기)는
  통과했고, 뒤이은 `--frame-insert-check`(Git 신원 없는 새 프로젝트)가 같은 오류로 두 번 실패했습니다.

## 필요한 결정과 환경

- `DEV-MAC`. Windows 셸 재개 또는 macOS 전용 선행으로 다시 잇는 계획 개정(ADR-0006 판정 규칙)이 필요합니다.

## 변경되지 않은 파일

- 선행 완료 전에 들어온 macOS 구현은 유지합니다: 생성 코어(`crates/project/src/create.rs`, `create/initial_document.rs`),
  runtime 비동기 요청과 `stellar_project_create` ABI(`5358b4bc`), 겹친 생성 거절과 ABI 기록(`f0a0a705`, ADR-0014 "XII.6.1 —
  새 프로젝트 생성" 절), 파일 메뉴 New Project(⌘N)와 시작 창 버튼.
- 첫 문서는 아직 generation 형식으로 저장합니다. 텍스트 원본 첫 저장은 V.10.7 범위입니다.
- 처음 통합 기록은 Codex 실행 결과만 근거로 했고 ADR 기록이 빠져 있었습니다(위 B7 커밋에서 보완).

## 재개 조건

- XII.2.3 차단이 풀리거나 계획 개정으로 macOS 선행만 남은 뒤, `--frame-insert-check` 실패를 고치고 smoke 전체와 create
  통합 검사를 다시 실행해 보고서를 완료 형식으로 씁니다.
- 2026-11-15 검토에서 지적한 "실제 창에서 Create 버튼이 비활성으로 남는 문제"는 이 검토에서 재현하지 않았습니다. 재개 때
  실제 창에서 확인합니다.
