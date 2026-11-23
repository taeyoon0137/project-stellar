# 작업 보고서: XV.4.3 lockfile가 있으면 선택자가 바뀌지 않은 의존성은 network 없이 lock commit 우선 사용

## WBS

- WBS: XV.4.3
- 작업명: lockfile가 존재하면 선언 selector가 바뀌지 않은 dependency는 network 없이 lock commit을 우선 사용
- 상태: 완료 (`[x]`)
- 선행: XV.4.2(`[x]`), XV.3.3(`[x]`)
- 결정: [ADR-0137](../adr/ADR-0137-library-stored-refs-and-materialize-commands.md) 결정 3·4

## 변경 파일

- `crates/dependencies/src/lock_preference.rs`(신규) — `LockPreferringResolver`: 기존 lock entry의 (repository 비교 키, 선언
  선택자) → exact commit pin. 서로 다른 commit을 가리키는 pin은 버림. `refresh`로 update 대상만 다시 해석.
- `crates/dependencies/src/lib.rs` — 모듈 등록.
- `crates/project/src/library_resolve.rs`(신규) — `CachedManifestSource`(cache entry의 `stellar.yaml`을 먼저 읽음),
  `ManifestEdit`(`stellar.yaml` 의존성 편집, 읽은 bytes hash가 그대로일 때만 씀).
- `crates/project/src/lib.rs` — 모듈 등록.
- `crates/ops/src/library_materialize.rs`(신규) — 제품 명령 `add`·`fetch`·`import`·`remove_imports`와 공용 단계(graph 해석 →
  materialize → entry 검증 → lock).
- `crates/ops/src/library.rs` — `LibraryCache::{create_os_default, keyer}`.
- `crates/ops/src/library_fixture.rs`(신규, `test-support`) — 실제 Git 텍스트 원본 library·사용 프로젝트 fixture.
- `crates/ops/src/library_materialize_tests.rs`(신규), `crates/ops/src/lib.rs`(모듈 등록).
- `docs/task-reports/XV.4.3.md`(신규).

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies --lib lock_preference` | 0 — 2 passed |
| `cargo test -p stellar-project --lib library_resolve` | 0 — 1 passed |
| `cargo test -p stellar-ops --lib library_materialize` | 0 — add·refs 시험 통과(이 행 범위) |
| `cargo clippy -p stellar-dependencies -p stellar-project -p stellar-ops --all-targets --features stellar-ops/test-support -- -D warnings` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: 없음(DEV-MAC 로컬 시험, 로컬 `file://` 저장소)

## 테스트

| 검사 | 결과 |
|---|---|
| **offline cache fixture**: ds를 add(lock·cache 생성)한 뒤 ds 저장소를 지우고 chips를 add하면 성공. ds는 lock commit으로 답함(`pinned` 1), 매니페스트는 cache entry에서 읽음, ds entry는 `CommitHit`(Git 없음), ds lock commit 그대로, lock entry 2개(`add_stores_declaration_and_lock_and_reuses_lock_commits_offline`) | 통과 |
| `fetch`는 cache에 있는 commit을 모두 `CommitHit`로 끝냄. 빈 cache 기기에서 저장소가 없으면 `Materialize(Mirror)` 실패, lock·`stellar.yaml` 불변 | 통과 |
| 단위: 같은 저장소 다른 표기도 같은 pin, 선택자가 바뀌면 안쪽 해석기, `refresh`는 lock을 쓰지 않음, 서로 다른 commit pin·lock 없음은 안쪽 해석기(`lock_preference::tests` 2건) | 통과 |
| `add`: library id는 해석한 commit의 `project.id`, 같은 이름 거부·export되지 않는 component 거부는 파일 불변 | 통과 |
| `ManifestEdit`: canonical 쓰기, 다시 열면 바꿀 것 없음, 되돌리기, 연 뒤 다른 writer가 바꾸면 쓰지 않음 | 통과 |

## 성능

- 해당 없음(인터랙션 경로 아님). lock commit을 쓰는 의존성은 Git 프로세스를 띄우지 않습니다.

## 차이

- 계획서 행은 "resolve" 동작만 말합니다. 제품 명령(`stellar library add|fetch`, CLI 등록은 별도 커밋)이 이 해석기를 씁니다.
- 매니페스트를 cache entry에서 읽으려면 entry가 있어야 합니다. cache에 없는 lock commit은 mirror를 fetch합니다(network 필요).

## 남은 위험

- lock entry의 선언 선택자와 repository 비교 키가 같으면 같은 commit을 씁니다. 같은 저장소의 tag를 다른 commit으로 옮긴 경우(force
  push)는 `stellar library update`로만 다시 해석합니다(의도한 동작).
- MCP 도구는 더하지 않았습니다(mcp-core 소유 밖).
