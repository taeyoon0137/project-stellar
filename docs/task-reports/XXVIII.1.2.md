# 작업 보고서: XXVIII.1.2 파일 포맷 핵심 규칙 명세

- WBS: XXVIII.1.2
- 작업명: canonical encoding, chunk envelope, generation manifest, YAML, StableId, unknown field,
  migration 규칙 문서화
- 상태: 완료 (`[x]`)
- 선행: Phase V(phase 단위 선행 — 완화 규칙 적용. Phase V 행은 기준 commit에서 모두 `[x]`이고 필요한
  코드가 있습니다)

## 필수 실행 환경

- `DEV-MAC` (문서 작업, 로컬 검증).

## 변경 파일 목록

- `docs/spec/stellar-format.md`(신규) — 구현 기준선 규범 명세.
- `docs/task-reports/XXVIII.1.2.md`(신규), 계획서(XXVIII.1.2 `[ ]`→`[x]`).
- 코드 변경 없음. 문서만 변경했습니다.

## 구현 요약

- **근거**: 기준 commit `78a574a`의 코드(`stellar-storage`, `stellar-core-types`, `stellar-project`,
  `stellar-assets`, `stellar-dependencies`, `tools/xtask`)와 ADR-0004, ADR-0007~ADR-0012, ADR-0015를
  읽고 옮겨 적었습니다. 모든 절에 crate·module 근거를 달았습니다.
- **범위**: 프로젝트 폴더 배치, canonical primitive(R1~R6)와 tagged field, content hash, `StableId`·
  `ChunkId`·문서 ChunkId 유도(UUIDv8)·`AssetId`, chunk envelope·경로·쓰기·읽기 검증, chunk kind,
  chunk pointer·remap, ViewTree·DocumentManifest payload와 문서 chunk model, generation manifest,
  current pointer, 열기·저장 절차, 제한 YAML(loader·스칼라·writer)과 `stellar.yaml`·
  `stellar.lock.yaml` 스키마, unknown field 규칙, 버전·migration 규칙.
- **현재 구현 기록**: 규칙과 다른 관찰 동작은 "현재 구현"으로 분리했습니다(열기 경로가 kind registry·
  스키마 버전을 쓰지 않음, StableId version bit 미검사, YAML 버전 값 미비교, 선행 BOM 처리 등).
- **아키텍처 문서와의 차이**: §19 경로 이름, §20.3 `lock_version`, §36 버전 기록을 부록 B에 적었습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `git merge --ff-only 78a574a` | 종료 코드 0, HEAD `78a574a` |
| `cargo test -p stellar-storage` | unit 227 passed, `chunk_canonical_fuzz` 5 passed, `migration_git_fixture` 1 passed, doctest 2 passed, 0 failed |
| `cargo test -p stellar-project` | unit 248 passed, 통합 테스트 17 passed(`document_residency` 3, `document_round_trip` 5, `document_serialize` 5, `restricted_yaml_fuzz` 4), 0 failed |
| `cargo test -p stellar-core-types` | unit 116 passed, doctest 8 passed, 0 failed |
| `cargo test -p stellar-assets` | 97 passed, 0 failed |
| `cargo xtask validate-storage-fixtures` | 종료 코드 0 |
| `cargo xtask test-migrations` | `migration golden fixture 통과: 2 fixtures`, 종료 코드 0 |
| 명세·코드 일치 검사 스크립트(scratchpad `check_spec.py`, `stellar-format.md` 대상) | 71개 검사 통과, 0 실패, backtick 식별자 242개 존재 확인, 종료 코드 0 |
| 일치 검사 스크립트 음성 테스트(kind code·오류 variant 이름을 일부러 틀린 사본) | 주입한 오류를 모두 실패로 보고 |
| 동작 확인용 scratch 실행 파일(scratchpad `probe`, 작업 트리 밖) | 아래 "검사" 표의 현재 구현 항목 재현 |
| `git diff --check` | 출력 없음 |
| `cargo xtask validate-task-report docs/task-reports/XXVIII.1.2.md` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| magic `STARCHNK`·`STARMFST`, format version 1, 확장자, `design/current` 경로가 코드 상수와 같음 | 통과 |
| chunk kind 6종 code·이름·required 여부, required bit `0x80000000`가 `storage::chunk::kinds`와 같음 | 통과 |
| §6.1 golden header hex가 `golden_header_bytes_are_fixed` 기대값과 같음 | 통과 |
| §3 예시 bytes가 `canonical-primitives.yaml` golden bytes와 같음(7건) | 통과 |
| 문서 ChunkId domain 문자열·family 값·version 8 bit 조작이 `document_io::serialize`와 같음 | 통과 |
| `CURRENT_SCHEMA_VERSION`·`CURRENT_LOCK_SCHEMA_VERSION`·`LATEST_SCHEMA`·`DOCUMENT_CHUNK_SCHEMA_VERSION` 값 | 통과 |
| writer 인용 키워드 목록, implicit key 1024자 | 통과 |
| 명세에 적은 오류 variant·함수·상수 이름이 코드에 있음 | 통과 |
| 열기 경로가 manifest `schema_version` 99와 모르는 required kind `0x80000099`를 받음(§7, §16.2 "현재 구현") | 재현됨(`load_current_generation` Ok, `LazyGeneration::open` Ok) |
| 같은 pointer 반복 → `MisorderedPointers`, 같은 ChunkId 다른 hash 오름차순 → `PointerSet(ConflictingChunkId)`, 끝 잔여 → `TrailingBytes`(§10.2) | 재현됨 |
| current pointer CRLF·LF 없음·LF 두 개 → `InvalidFormat`, 정상 파일 72 byte(§11) | 재현됨 |
| 대문자 hex·`SHA256`·`blake3` prefix 거부(§4) | 재현됨 |
| UUIDv4 형태·대문자 문자열을 `StableId`로 받음(§5.1 "현재 구현") | 재현됨 |
| `take_fields`가 모르는 optional tag를 결과에서 뺌(§15) | 재현됨 |
| canonical writer 출력 `schema_version: "1"`(§14.5 예) | 재현됨 |
| `Manifest::from_yaml`이 `schema_version: 99`, `project.id: x`를 받음, `Lockfile::from_yaml`이 버전 7·임의 commit·hash를 받음(§14.5, §14.6 "현재 구현") | 재현됨 |
| 선행 BOM이 첫 키에 붙어 `UnknownKey`(§14.1) | 재현됨 |

## 차이(작업 지시와 다른 점)

- 검증 기준 "internal implementation과 spec fixture 일치"는 새 fixture를 만들지 않고 기존 golden
  fixture(`canonical-primitives.yaml`, envelope golden header, seed corpus, migration fixture)와 명세의
  일치로 확인했습니다. 공개용 conformance corpus는 XXVIII.2.1 범위입니다.
- 의미 모델 payload 내부(`DocumentDefinition`, `SubtreeRecords`, `ModifierStack`)는 XXVIII.1.3 범위로
  남겼습니다.

## 남은 위험 / 한계

- 명세는 기준 commit의 동작을 기록합니다. 코드가 바뀌면 같은 commit에서 명세를 고쳐야 합니다. 이를
  강제하는 CI 검사는 없습니다(일치 검사 스크립트는 scratchpad에 있고 저장소에 넣지 않았습니다).
- `stellar-dependencies`의 선언·lock 모델(XV.1.1, XV.4.1)과 열기 경로의 `stellar.yaml`·
  `stellar.lock.yaml` 스키마가 서로 맞지 않습니다. 명세는 열기 경로 스키마를 규범으로 적고 차이를
  기록만 했습니다. 통합 결정이 필요합니다.
- 기술 아키텍처 §19 경로 이름과 구현 경로의 차이는 결정되지 않았습니다(ADR-0010).
- 새 디렉터리 `docs/spec/`를 README 저장소 구성 tree에 반영하지 않았습니다. 허용 경로 밖
  (`resources/README.preset.md`)이므로 후속 작업이 필요합니다.
