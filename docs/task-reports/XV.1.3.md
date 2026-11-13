# 작업 보고서: XV.1.3 의존성 선언 충돌 validation

- WBS: XV.1.3
- 작업명: 같은 logical name 중복, 같은 LibraryId 다른 selector, 다른 LibraryId 같은 URL을 구분해 validation
- 상태: 완료 (`[x]`)
- 선행: XV.1.1(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/dependencies/src/validation.rs`(신규) — `RepositoryUrl::comparison_key`, `ConflictKind`,
  `ConflictSeverity`, `DeclarationConflict`, `DeclarationReport`, `validate_declarations`.
- `crates/dependencies/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XV.1.3.md`(신규), 계획서(XV.1.3 `[ ]`→`[x]`).

## 구현 요약

- **충돌 종류와 심각도**
  - 같은 logical name 중복 → 오류(dependency path·lock key 식별자).
  - 다른 LibraryId가 같은 repository → 오류(한 저장소 root는 한 프로젝트, 아키텍처 19.1).
  - 같은 LibraryId·다른 선택자 → 경고(여러 버전 동시 로드 허용·자동 통합 금지, 아키텍처 30.3).
  - 같은 LibraryId·같은 선택자·다른 이름 → 경고(같은 node로 dedup되는 중복 선언).
- **repository 비교 키**: 끝의 `/`·`.git` 제거, `scheme://[user@]host`와 scp 형식
  `[user@]host:path`의 scheme·host만 소문자화. 경로·user 대소문자는 보존합니다.
- 이름·LibraryId·repository 키별로 `BTreeMap` 그룹을 만들어 판정하므로 결과는 선언 순서와
  무관하게 결정적이고, 종류 → 이름 순으로 정렬합니다. 충돌 메시지에는 URL을 넣지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 12 passed, 0 failed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| conflict matrix 8행(clean, 이름 중복, 같은 lib 다른/같은 선택자, 다른 lib 같은 URL, 정규화 후 같은 URL, 같은 lib 다른 URL, 복합) 기대 충돌과 정확히 일치, 각 행 선언 역순에서도 동일(`conflict_matrix_fixture`) | 통과 |
| URL 정규화 동치 5쌍·비동치 4쌍(`repository_comparison_key_normalization`) | 통과 |
| 오류/경고 분류, 충돌 메시지에 URL 미포함(`severities_and_messages_without_urls`) | 통과 |

## 차이(작업 지시와 다른 점)

- 계획 행의 세 충돌 외에 "같은 LibraryId·같은 선택자·다른 이름"을 경고로 추가했습니다. matrix에서
  같은 LibraryId 칸을 선택자 동일/상이로 빠짐없이 구분하기 위함입니다.

## 남은 위험 / 한계

- repository 정규화는 문자열 수준입니다. 같은 저장소의 https·ssh 주소처럼 다른 transport 표기는
  다른 저장소로 판정합니다(해석 후 동일 commit이면 node dedup으로 합쳐짐).
- 전이 의존성(라이브러리 내부 선언) 사이의 충돌은 resolver 그래프 단계(XV.2)에서 다룹니다.
