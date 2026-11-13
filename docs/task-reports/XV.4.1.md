# 작업 보고서: XV.4.1 resolver 그래프 → lock resolved entry 변환

- WBS: XV.4.1
- 작업명: resolver graph를 `stellar.lock.yaml`의 resolved entries와 dependency paths로 변환
- 상태: 완료 (`[x]`) — 2026-11-14 deps_wiring lane에서 `stellar.lock.yaml` **텍스트** canonical
  golden을 추가했습니다(아래 "완료 보완"). 최초 작업 시점에는 `DeclNode` golden까지만 있어 `[~]`였습니다.
- 선행: IV.1.6(`[x]`), XV.2.6(`[x]`, 2026-11-14 상태 복원)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/dependencies/src/lock.rs`(신규) — `LOCK_SCHEMA_VERSION`, `LockEntry`, `LockDocument`
  (`from_graph`, `to_node`), golden 테스트.
- `crates/dependencies/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XV.4.1.md`(신규), 계획서(XV.4.1 `[ ]`→`[~]`).

## 구현 요약

- `LockDocument::from_graph`는 `DependencyGraph::sorted_entries()`(XV.2.6: canonical 경로, LibraryId →
  commit → path 정렬)만 입력으로 씁니다. 사용자 편집 필드를 받는 경로가 없습니다(IV.1.7 원칙).
- entry는 edge마다 하나: `logical_id`(경로 마지막 이름), `library_id`, `source`, `repository`,
  `requested`(선언 선택자, `version`과 같은 형식), `exact_commit`, `dependency_path`(시퀀스). 문서는
  `schema_version` + `resolved` 시퀀스이며 키 순서가 고정됩니다. IV.1.6 lock 모델과 겹치는 필드 이름
  (`logical_id`, `source`, `exact_commit`, `dependency_path`, `schema_version`)을 그대로 씁니다.
- content hash·library schema version·전이 의존성 기록과 누락 필드 validator는 XV.4.2입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 44 passed, 0 failed; doctest 1 passed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 다중 버전 fixture(a→c v1, b→c v2) lock 트리가 고정 golden(entry 4건, 키 순서·값·경로)과 일치, 선언 순서를 바꿔도 동일, exact commit 40자, logical_id = 경로 마지막(`canonical_lock_golden`) | 통과 |

## 차이(작업 지시와 다른 점)

- golden은 `DeclNode` 트리 수준입니다. 텍스트 golden은 project writer 연결 후 추가해야 하므로 `[~]`로
  두었습니다.
- 기존 project `Lockfile`(IV.1.6)은 알 수 없는 키를 거부하므로 `library_id`·`repository`·`requested`
  필드를 받으려면 project lock 모델 확장이 필요합니다(project crate 작업).

## 남은 위험 / 한계

- 아키텍처 20.3 예시는 `resolved`를 `name@version` 키 매핑으로 보여 주지만 IV.1.6 모델은 시퀀스입니다.
  이 작업은 IV.1.6(시퀀스)을 따랐습니다.

## 완료 보완(deps_wiring lane, 2026-11-14)

### 변경 파일 목록

- `crates/project/src/lock.rs` — 테스트 `canonical_lock_text_golden_from_resolver_graph`와 golden
  `CANONICAL_LOCK_GOLDEN`, resolver 입력 harness(`resolved_lock_text`).
- `docs/task-reports/XV.4.1.md`(이 절), 계획서(XV.4.1 `[~]`→`[x]`).

### 구현 요약

- 선행 Fix 커밋으로 lock 형식이 하나가 됐습니다(ADR-0016). project `Lockfile`(IV.1.6)은 이 작업의
  `LockEntry`를 resolved entry로 쓰고, `Lockfile::from_document(LockDocument::from_graph(..))`가
  resolver 결과를 그대로 받습니다. 사용자 편집 필드를 받는 경로는 없습니다(IV.1.7).
- golden 입력은 모두 실제 `stellar.yaml` **텍스트**입니다. 사용 프로젝트 선언(a: tag v1, b: branch
  main)과 library 매니페스트(a → c v1, b → c v2)를 제한 loader와 `Manifest::from_yaml`이 읽습니다.
  선택자 해석만 표 기반(결정적 commit)이고, 실제 Git 해석은 `manifest_source` 테스트가 다룹니다.

### 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-project` | lib 262 passed, 통합 17 passed, 0 failed |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-project -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **canonical lock golden**: resolver 그래프 → `Lockfile::write_checked` 텍스트가 고정 golden(entry 4건, 키 순서·값·dependency path·전이 dependencies·`requested: {branch: main}`)과 byte 일치, 선언 순서 반대로도 byte 동일, golden을 다시 읽어 기록해도 동일(`canonical_lock_text_golden_from_resolver_graph`) | 통과 |

### 차이

- content hash는 node별 결정적 값(commit 번호)입니다. 검증된 cache entry hash 공급(XV.3.3·XV.3.4)이
  아직 없기 때문이며, 이 작업 행의 범위(그래프 → resolved entries·dependency paths)에는 영향이 없습니다.
