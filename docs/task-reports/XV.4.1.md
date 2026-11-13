# 작업 보고서: XV.4.1 resolver 그래프 → lock resolved entry 변환

- WBS: XV.4.1
- 작업명: resolver graph를 `stellar.lock.yaml`의 resolved entries와 dependency paths로 변환
- 상태: 부분 완료 (`[~]`) — 변환과 형식 중립 트리(`DeclNode`) golden은 완료했지만, `stellar.lock.yaml`
  **텍스트** canonical golden은 project crate의 canonical YAML writer(IV.1.5)·lock 모델(IV.1.6) 연결이
  필요합니다.
- 선행: IV.1.6(`[x]`), XV.2.6(이 lane에서 완료)

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
