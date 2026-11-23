# 작업 보고서: XVI.11.1 새 프로젝트→Frame·Text·Image→Component·Token→Save→Close→Reopen E2E

## WBS

- WBS: XVI.11.1
- 작업명: 새 프로젝트→Frame·Text·Image→Component·Token→Save→Close→Reopen E2E
- 상태: 부분 완료 (`[~]`) — View Variant는 영속 형식이 없어 저장·다시 열기 대상이 아니고, "양 플랫폼"은 ADR-0006에 따라 macOS만입니다.
- 선행: Phases 3\~12
- 결정: [ADR-0111](../adr/ADR-0111-headless-release-e2e-scenarios.md)
- lane: `headless-e2e`

## 변경 파일

- `tests/integration/tests/e2e_new_project_reopen.rs`: 시나리오
- `tests/integration/tests/e2e_common/mod.rs`: 앱 runtime helper·테스트 HOME 격리(공용)
- `tests/integration/Cargo.toml`·`Cargo.lock`: dev 의존 등록(공용)

코드 변경은 테스트뿐입니다(제품 코드 변경 없음).

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-integration-tests --locked` | 0 — 전체 통합 테스트 통과(e2e 6개 파일 포함) |
| `cargo clippy -p stellar-integration-tests --tests --locked` | 0 — 경고 0 |
| `cargo xtask validate-adr --base 682c7b0c` | lane 결과 보고에 적음 |
| `STELLAR_CHECK_BASE=682c7b0c cargo xtask check-core` | lane 결과 보고에 적음 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: Apple Silicon macOS 로컬 실행, debug build, 시스템 Git. 앱 UI 없이 앱 runtime(`AppRuntime`)과 headless ops만 씁니다.
  테스트 HOME·전역 Git 설정은 테스트 프로세스 임시 폴더로 격리합니다(ADR-0111 결정 2).

## 테스트

| 테스트 | 결과 |
|---|---|
| `new_project_edit_save_close_reopen_keeps_semantic_snapshot` | 통과 — 앱 생성 경로로 새 프로젝트(최초 commit) → Frame·속성 2개·Text·PNG Image(history 5항목) → Save Now(텍스트 원본·cache, generation 2) → `component-source create`·token set 원본 파일 → 창 저장이 두 파일을 보존(generation 3) → 창·프로젝트 닫기·runtime 종료 → 새 runtime에서 다시 열기: 문서 JSON·`TextProject::semantic_key`·headless snapshot 동일, history 0, Save Now 쓸 것 없음 → `stellar apply` 인스턴스 삽입 뒤 앱 다시 열기가 원본에 반영, 다시 닫고 열어도 같음 → HEAD·index 불변 |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음).

## 차이

- View 변수·Variant(IX.7)는 메모리 command만 있고 문서·텍스트 원본·chunk 형식이 없어 만들지 않았습니다(ADR-0111 결정 3).
- Token set 생성 command가 없어 텍스트 원본 파일을 codec 공개 encoder로 썼습니다(사용자·AI가 Git 원본을 직접 고치는 경로).
- "양 플랫폼": Windows는 ADR-0006으로 보류라 macOS만 실행했습니다.

## 남은 위험

- View Variant 영속 형식(문서 레코드·텍스트 원본 v4)이 생기면 이 시나리오에 Variant 생성·다시 열기를 더해야 행을 완료할 수 있습니다.
- 앱 창에는 token·component를 만드는 runtime 요청이 없습니다. UI 재설계 때 같은 원본 파일을 쓰는 요청이 필요합니다.
