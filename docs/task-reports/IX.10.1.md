# 작업 보고서: IX.10.1 TargetProfileId·profile 종류·profile schema version

- WBS: IX.10.1
- 작업명: `TargetProfileId`, profile kind Web/Apple/Android, profile schema version 정의
- 상태: 완료 (`[x]`)
- 선행: IV.1.1(`[x]`), VIII.7.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/target-profile/src/node.rs`(신규) — 형식 중립 트리 `ManifestNode`(+ 비교·지문용 `canonical_bytes`),
  `SchemaError`, `ValueError`, 엄격 정수 파서와 스키마 헬퍼.
- `crates/target-profile/src/profile.rs`(신규) — `TargetProfileId`, `TargetProfileKind`, `ProfileSchemaVersion`,
  `ProfileHeader`(id·종류·schema version의 노드 변환).
- `crates/target-profile/src/lib.rs` — 모듈 등록, crate 문서.
- `docs/task-reports/IX.10.1.md`(신규), 계획서(IX.10.1 `[ ]`→`[x]`).

## 구현 요약

- **형식 중립 트리**: crate graph상 target-profile은 core-types에만 의존하므로 제한 YAML을 직접 읽지 않습니다.
  `ManifestNode`(스칼라/시퀀스/매핑, 키 순서 보존)는 project의 `YamlValue`와 1:1로 대응합니다(의존성 절의
  `DeclNode`와 같은 방식). `canonical_bytes`는 파일 형식이 아니라 비교·지문용 결정론 bytes이며 반복(비재귀)으로
  기록합니다.
- **`TargetProfileId`**: 기존 `stellar.yaml` `target_profiles[i].id` slug(예: `apple-desktop`). ASCII 영숫자 시작,
  영숫자·`-`·`_`·`.`만, 최대 64 byte. 오류에 입력 값을 담지 않습니다.
- **`TargetProfileKind`**: Web=1, Apple=2, Android=3. 표현식 환경 key `platform` variant 번호(VIII.7.1)와 같은 값으로
  고정했습니다. `stellar.yaml`에는 기존 `platform` 키에 이름(`web`·`apple`·`android`)으로 씁니다.
- **`ProfileSchemaVersion`**: entry 형식 버전, `CURRENT` = 1. 0과 이 build보다 새 버전은 해석하지 않고
  `UnsupportedSchemaVersion`으로 거부합니다. entry 키는 최상위 `schema_version`과 구분해
  `profile_schema_version`입니다.
- **`ProfileHeader`**: canonical 키 순서 `id`, `platform`, `profile_schema_version`. `read`는 종류별 필드 검사를
  호출자(IX.10.2)에게 남기고, `from_node`는 머리 외 키를 `UnknownKey`로 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-target-profile` | 7 passed, 0 failed |
| `cargo clippy -p stellar-target-profile --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-target-profile -- --check` | 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | exit 0 |

| 검사 | 결과 |
|---|---|
| numeric kind code·이름 golden(Web=1·Apple=2·Android=3, 미정의 code·대소문자 다른 이름 거부, CURRENT=1) (`kind_numeric_codes_are_golden`) | 통과 |
| 머리 노드 golden, 모델→노드→모델·노드→모델→노드 동일, 모든 종류 round-trip (`header_round_trip_is_golden`) | 통과 |
| id 규칙(빈 값·선행 기호·공백·비ASCII·64 byte 초과 거부) (`id_rules`) | 통과 |
| 누락·알 수 없는 종류·schema version 0/2/선행 0/비숫자·알 수 없는 키·타입 불일치를 경로와 함께 거부 (`header_violations_are_rejected_with_paths`) | 통과 |
| 엄격 정수 파서, canonical bytes 결정론·키 순서 민감·10만 단계 중첩 비재귀 기록 (`node::tests`) | 통과 |

## 차이(작업 지시와 다른 점)

- round-trip은 `ManifestNode` 수준에서 검증했습니다. `stellar.yaml` 텍스트 ↔ `YamlValue` ↔ `ManifestNode` 연결과
  기존 `project::manifest::TargetProfile`(문자열 필드)을 이 모델로 바꾸는 연결은 project crate 작업 범위입니다
  (의존 방향상 이 crate는 project를 쓸 수 없음, XV.1.1과 같은 방식).
- 결정 사항(id 규칙, kind code를 환경 `platform` variant와 일치, `profile_schema_version` 키 이름)은 이 lane의 허용
  경로 밖인 `docs/adr/`에 ADR로 기록하지 않고 모듈 문서와 이 보고서에 남겼습니다.

## 남은 위험 / 한계

- 기존 manifest 예제의 `platform` 외 필드(`minimum_os`·`baseline`)와 종류별 필수 필드는 IX.10.2에서 정의합니다.
- 환경 key `platform` variant 번호와의 일치는 이 crate의 golden test로만 고정되며, expressions crate 상수와의
  교차 검사는 두 crate를 함께 쓰는 상위 crate에서 추가해야 합니다.

## 후속 수정(리뷰 반영): `ManifestNode` 깊이 전제 문서화

리뷰(TP-6)에서 공개 타입 `ManifestNode`가 derive한 `Clone`·`PartialEq`·`Debug`와 drop이 재귀라 깊은 트리에서 스택이 넘칠 수
있는데 그 전제가 문서에 없다는 점이 지적되어 `Fix` commit으로 타입 문서에 깊이 전제를 적었습니다. `stellar.yaml`에서 온
트리는 project 제한 YAML loader의 중첩 상한(`MAX_NESTING_DEPTH` = 64)을 지키고, 프로그램으로 만드는 트리도 같은 상한 안에서
만들어야 하며, 깊이와 무관하게 반복으로 동작하는 것은 `canonical_bytes`뿐이라는 내용입니다.

- 코드 동작은 바꾸지 않았습니다(문서만). 반복 `Drop`을 직접 구현하면 하위 crate가 값을 이동하는 패턴 분해를 쓸 수 없게
  되고(E0509), `Clone`·`PartialEq`·`Debug`는 여전히 재귀라 문서화로 정리했습니다. 문서만 바꾼 수정이라 실패 테스트로 재현하지
  않았습니다(재현하면 테스트 프로세스가 스택 넘침으로 종료됨).

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-target-profile` | 42 passed, 0 failed |
