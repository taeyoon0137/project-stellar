# 작업 보고서: XXVIII.2.4 버전 호환성 matrix와 deprecation 정책

- WBS: XXVIII.2.4
- 작업명: version compatibility matrix와 deprecation policy 문서화
- 상태: 완료 (`[x]`)
- 선행: V.8.6(기준 commit에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (문서 작업, 로컬 검증).

## 변경 파일 목록

- `docs/spec/stellar-format-compatibility.md`(신규) — 버전 축 목록, 호환성 판정 matrix, 축별 reader
  동작, 변경 종류별 호환성, 과거 fixture 검증, deprecation 정책, 정하지 않은 사항.
- `docs/task-reports/XXVIII.2.4.md`(신규), 계획서(XXVIII.2.4 `[ ]`→`[x]`).
- 코드 변경 없음. 문서만 변경했습니다.

## 구현 요약

- **버전 축 12개**: chunk·manifest format version, storage 문서 스키마(`LATEST_SCHEMA` = 2), chunk payload
  스키마(`DOCUMENT_CHUNK_SCHEMA_VERSION` = 1), hash 알고리즘, chunk kind, 문서 ChunkId domain,
  `stellar.yaml`·`stellar.lock.yaml` 스키마, 미연결 lock 모델, 복구 스냅샷, 신뢰 목록. 축마다 코드 상수와
  현재 비교 규칙을 적었습니다.
- **판정 matrix**: `assess_compatibility`의 5단계 규칙과 테스트 `compatibility_matrix`의 6개 사례.
- **축별 reader 동작**: 기준 commit의 실제 동작을 적었습니다. format version 불일치 manifest는 이전 정상
  세대로 읽기 전용 fallback, chunk는 읽기 시점에 실패, 스키마 버전·required kind는 열기에서 검사하지 않음,
  YAML 키 추가는 구버전에서 열기 실패.
- **deprecation 정책 D1~D8**: Accepted ADR(0004, 0007, 0008, 0010, 0011, 0012)의 결정만 모았습니다. ADR에
  없는 7개 항목은 "정하지 않은 사항"으로 남겼습니다.
- **§36 차이 G1~G4**: 네 버전 기록, 구버전 편집 금지, 자동 migration 연결, 원자성(충족).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo xtask test-migrations` | `migration golden fixture 통과: 2 fixtures`, 종료 코드 0 |
| `cargo test -p xtask migration` | `fixture_set_is_not_empty_and_targets_latest`, `all_golden_fixtures_migrate_to_latest_snapshot` 2 passed, 0 failed |
| `cargo test -p stellar-storage migration` | unit 14 passed(compat 4, registry 7, runner 3), `migration_git_fixture` 1 passed, 0 failed |
| `cargo test -p stellar-storage golden` | 6 passed(envelope header, manifest layout·round trip, kind code, chunk path, canonical primitive fixture), 0 failed |
| `cargo xtask validate-storage-fixtures` | 종료 코드 0 |
| 명세·코드 일치 검사 스크립트(scratchpad `check_spec.py`, 세 명세 전체) | 222개 검사 통과, 0 실패, backtick 식별자 500개 존재 확인, 종료 코드 0 |
| 일치 검사 스크립트 음성 테스트(format version 값을 일부러 틀린 사본) | 주입한 오류를 실패로 보고 |
| scratch 실행 파일 `probe`(작업 트리 밖) | manifest `schema_version` 99·kind `0x80000099`로 열기 성공, `blake3:`·`SHA256:` prefix `UnknownAlgorithm`, `stellar.yaml` 버전 99·lock 버전 7 수용 |
| `git diff --check` | 출력 없음 |
| `cargo xtask validate-task-report docs/task-reports/XXVIII.2.4.md` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| 버전 축 표의 상수 값 9개(`CHUNK_FORMAT_VERSION`, `MANIFEST_FORMAT_VERSION`, `LATEST_SCHEMA`, `DOCUMENT_CHUNK_SCHEMA_VERSION`, `CURRENT_SCHEMA_VERSION`, `CURRENT_LOCK_SCHEMA_VERSION`, `LOCK_SCHEMA_VERSION`, `RECOVERY_PAYLOAD_VERSION`, `TRUST_STORE_SCHEMA_VERSION`)가 코드와 같음 | 통과 |
| §3 판정 표가 `compat.rs`의 순서(선언 → format → reader 최소 → writer 최소)와 같음 | 수동 대조 통과 |
| §3 예시 6개가 `compatibility_matrix` 테스트 사례와 같음 | 수동 대조 통과, 테스트 통과 |
| 과거 fixture `v1-minimal`, `v1-two-views`가 `LATEST_SCHEMA` = 2로 변환되고 snapshot 일치 | 통과 |
| §4.2 "스키마 버전 비교 없음", §4.3 "required kind 거부 없음" | 재현됨 |
| §4.1 "manifest format 불일치 시 fallback" | 코드 대조(`open_project_storage`가 `load_current_generation`의 모든 오류에서 scanner로 넘어가고, scanner는 같은 `verify_generation`을 씀) |
| §7 D1~D8이 각 ADR 본문의 결정과 같음 | 수동 대조 통과 |

## 차이(작업 지시와 다른 점)

- 작업 지시의 "공개"는 저장소 안 명세 문서 작성으로 해석했습니다. 외부 공개는 기술 아키텍처 §35의 공개
  전 조건 미충족으로 하지 않았습니다(XXVIII.3.x).
- deprecation 정책은 기존 ADR 결정만 모았습니다. 지원 기간, migration step 제거 조건, deprecated 표시
  방법 등은 새 아키텍처 결정이라 ADR 없이 정하지 않고 §8에 남겼습니다(이 작업의 허용 경로는
  `docs/spec/`뿐입니다).
- §4.1의 manifest format version 불일치 fallback 동작은 실행 재현이 아니라 코드 대조로 확인했습니다.

## 남은 위험 / 한계

- 현재 구현은 §36의 "새 스키마를 구버전에서 편집 금지"를 강제하지 않습니다(G2). 스키마 버전을 올린
  세대를 구버전 앱이 편집 가능 상태로 엽니다. 최소 reader·writer 버전을 디스크에 기록하는 결정과 열기
  경로 연결이 필요합니다(ADR 필요).
- 과거 fixture는 storage 스키마 1 → 2(noop) 두 개뿐이고 payload 변환이 있는 실제 스키마 변경 fixture는
  없습니다. format version 2 이상의 fixture도 없습니다.
- YAML 스키마 버전은 호환성 판단에 쓰이지 않아, 키 추가가 구버전 앱에서 바로 열기 실패가 됩니다.
- `stellar-dependencies` lock·선언 모델과의 통합이 정해지지 않았습니다.
