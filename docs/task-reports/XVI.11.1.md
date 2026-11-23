# 작업 보고서: XVI.11.1 새 프로젝트→Frame·Text·Image→Component·Token·Variant→Save→Close→Reopen E2E

## WBS

- WBS: XVI.11.1
- 작업명: 새 프로젝트→Frame·Text·Image→Component·Token·Variant→Save→Close→Reopen E2E
- 상태: 부분 완료 (`[~]`) — macOS 시나리오는 Variant까지 완료했습니다. "양 플랫폼"의 Windows는 ADR-0006으로 보류입니다.
- 선행: Phases 3\~12
- 결정: [ADR-0111](../adr/ADR-0111-headless-release-e2e-scenarios.md), [ADR-0126](../adr/ADR-0126-guides-grids-view-variables-persistence.md)
  (View 변수·Variant 영속 형식, ADR-0111 결정 3을 바꿈)
- lane: `headless-e2e`(1차), `guides-persist`(2차)

## 변경 파일

1차:

- `tests/integration/tests/e2e_new_project_reopen.rs`, `tests/integration/tests/e2e_common/mod.rs`, `tests/integration/Cargo.toml`·`Cargo.lock`

2차(View 변수·Variant 영속 형식, ADR-0126):

- `crates/document/src/view_variables.rs`(신규: 변수·Variant 단위 bytes, command bytes, 단위 목록에서 model 조립)
- `crates/document/src/model.rs`·`records.rs`·`record_codec.rs`·`validate.rs`, `crates/document/src/guides/persist.rs`(View root subtree 확장 절 tag 0x02)
- `crates/commands/src/guides.rs`(`edit-view-variants` 0x00B1), `snapshot_apply.rs`(외부 반영 tag 10)
- `crates/project/src/document_io/source_v4/view_data.rs`(View 파일 `variables`·`variants`), `view.rs`·`project.rs`
- `crates/semantic-diff/src/guides.rs`(View 변수 속성·Variant 객체와 조건 속성)
- `tests/integration/tests/e2e_new_project_reopen.rs`: 7b 단계(View 변수·Variant 생성 → 공용 저장 → 앱 다시 열기)

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-integration-tests --test e2e_new_project_reopen` | 0 — 시나리오 통과(시스템 Git 있음, 건너뛰지 않음) |
| `cargo test -p stellar-document --lib view_variables` | 0 — 단위 bytes·command bytes 왕복, orphan 조건 보존 조립 |
| `cargo test -p stellar-project --lib view_data` | 0 — 단일 View·다중 파일 원본과 chunk cache 왕복 |
| `cargo test -p stellar-commands --lib guides` | 0 — Variant 생성 command가 레코드에 남고 undo가 되돌림, 외부 반영 왕복 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- Apple Silicon macOS 로컬 실행, debug build, 시스템 Git. 앱 UI 없이 앱 runtime(`AppRuntime`)과 headless ops만 씁니다.

## 테스트

| 테스트 | 결과 |
|---|---|
| `new_project_edit_save_close_reopen_keeps_semantic_snapshot` | 통과 — 1차 단계(새 프로젝트 → Frame·속성·Text·Image → Save → Component·token 원본 → Close → Reopen → `stellar apply` 인스턴스)에 7b 단계를 더함: Boolean 변수 `dark`와 그 변수를 조건으로 카드 node override를 가진 Variant를 `edit-view-variants` command 두 개로 만들고 `ops::snapshot::save`로 원본·cache에 씀 → View 원본 파일에 `variables`·`variants` → 앱 다시 열기: 저장할 것 없음 → 다시 열린 문서의 View model과 원본 전체 의미(`TextProject::semantic_key`)가 같음 |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음).

## 차이

- View 변수·Variant 생성은 앱 runtime 요청이 아니라 같은 command를 headless 공용 저장으로 씁니다. 앱 창에서 Variant를 만드는 runtime
  요청은 UI 재설계 때 같은 `edit-view-variants` command로 연결합니다.
- Token set 생성 command가 없어 텍스트 원본 파일을 codec 공개 encoder로 씁니다(1차와 같음).
- "양 플랫폼": Windows는 ADR-0006으로 보류라 macOS만 실행했습니다.

## 남은 위험

- Variant override는 텍스트 원본에서 canonical bytes hex입니다. 사람이 읽는 patch 줄은 Variant 편집 입력과 함께 정합니다(ADR-0126).
- 앱 창에는 token·component·View 변수를 만드는 runtime 요청이 없습니다.
