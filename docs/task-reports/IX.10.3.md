# 작업 보고서: IX.10.3 `stellar.yaml` profile 목록과 기본 profile

- WBS: IX.10.3
- 작업명: `stellar.yaml`에 project target profile 목록과 기본 profile을 저장하고 존재하지 않는 default 거부
- 상태: 부분 완료 (`[~]`) — 절 모델·검증 규칙·round-trip은 형식 중립 `ManifestNode`에서만 증명했고, 실제
  `stellar.yaml` 파서(project crate)는 새 profile 필드와 `default_target_profile`을 아직 받지 않아 이 형식을 저장할 수
  없습니다(아래 상태 정정).
- 선행: IX.10.2(이 lane `[x]`), IV.1.5(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/target-profile/src/manifest.rs`(신규) — `ProjectTargetProfiles`, `SectionError`, `KEY_TARGET_PROFILES`,
  `KEY_DEFAULT_TARGET_PROFILE`, `MAX_PROFILES`.
- `crates/target-profile/src/lib.rs` — 모듈 등록.
- `docs/task-reports/IX.10.3.md`(신규), 계획서(IX.10.3 `[ ]`→`[x]`).

## 구현 요약

- **저장 위치**: 매니페스트 최상위 `target_profiles`(시퀀스, 저장 순서 보존)와 새 키 `default_target_profile`(스칼라).
  기본 profile 키는 `target_profiles` 바로 뒤에 둡니다.
- **규칙**: id 유일(`DuplicateId`, 두 번째 entry 경로), profile이 있으면 기본 profile 필수(`MissingDefault` — 첫 항목을
  암시적으로 고르지 않음), 목록에 없는 기본 id 거부(`UnknownDefault`, 빈 목록 + 기본값 포함), 최대 64개
  (`TooManyProfiles`, entry 파싱 전에 검사). 생성자·`set_default`도 같은 규칙을 강제해 유효하지 않은 절을 만들 수 없습니다.
- **기록**: `write_into_root`는 두 키만 교체·삽입·삭제하고 project가 소유한 다른 최상위 키와 순서를 보존합니다.
  기본 profile이 없으면 키를 지웁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-target-profile` | 16 passed, 0 failed |
| `cargo clippy -p stellar-target-profile --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-target-profile -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| manifest round-trip: `schema_version`·`project`·`dependencies`를 포함한 최상위 노드 → 절 → 다시 기록한 노드의 canonical bytes 동일, 키 순서 고정, 모델→노드→모델 동일 (`manifest_round_trip_preserves_section_and_other_keys`) | 통과 |
| 기본 profile 키가 없던 새 프로젝트 노드에 삽입(`target_profiles` 뒤), 빈 절 기록 시 키 삭제, 매핑이 아닌 root 거부 (`write_inserts_default_after_profiles_and_removes_it_when_empty`) | 통과 |
| invalid default fixture: 목록에 없는 id, 빈 목록 + 기본값, 기본값 누락, 빈 문자열, 비스칼라, 생성자·`set_default` 거부 후 절 유지 (`invalid_default_fixtures_are_rejected`) | 통과 |
| 중복 id 경로, 65개 상한, `target_profiles` 누락·타입 불일치, entry index 경로 (`section_structure_violations_are_rejected`) | 통과 |
| 같은 id 교체 시 위치 보존 (`replace_profile_keeps_position`) | 통과 |

## 차이(작업 지시와 다른 점)

- round-trip은 `ManifestNode` 수준(매니페스트 최상위 전체 노드)에서 검증했습니다. `stellar.yaml` 텍스트 ↔ `YamlValue`
  ↔ `ManifestNode` 연결, project `Manifest`의 허용 키 목록에 `default_target_profile` 추가, 기존 문자열 `TargetProfile`
  대체는 project crate 작업 범위입니다(의존 방향상 이 crate는 project를 쓸 수 없음 — XV.1.1과 같은 방식). 텍스트 canonical
  writer(IV.1.5)는 이 노드를 그대로 기록하면 됩니다.
- 결정 사항(기본 profile 필수 규칙, 키 이름·위치)은 허용 경로 밖인 `docs/adr/` 대신 모듈 문서와 이 보고서에 기록했습니다.

## 남은 위험 / 한계

- 현재 project crate의 `Manifest::from_yaml`은 `default_target_profile`을 알 수 없는 키로 거부하고, `target_profiles[i]`의
  새 필드(`profile_schema_version` 등)도 거부합니다. project 연결 전까지 실제 파일에는 이 형식을 쓸 수 없습니다.
- `docs/spec/stellar-format.md` 14.5 표 갱신도 project 연결 작업과 함께 해야 합니다.

## 상태 정정(리뷰 반영, 2026-11-14)

- `[x]` → `[~]`: 리뷰(TP-1) 지적을 확인했습니다. 작업명의 "`stellar.yaml`에 저장"이 실제 파일 경로에서는 성립하지 않습니다.
  - project crate `crates/project/src/manifest.rs`의 `parse_target_profiles`는 `target_profiles[i]`에 `id`·`platform`·
    `minimum_os`·`baseline`만 허용합니다(`reject_unknown_keys`). 이 crate가 필수로 요구하는 `profile_schema_version`·
    `registry_version`·`apple_platform`·`api_level`·`ui_system_version`·`capability_set_version`과 최상위
    `default_target_profile`은 `UnknownKey`로 거부됩니다. `baseline`도 project에서는 자유 문자열, 이 crate에서는
    2000\~9999 연도입니다. 그래서 profile이 하나라도 있는 `stellar.yaml`은 두 파서 중 한쪽에서만 통과합니다.
  - crate graph(`tools/xtask/src/crate_graph.rs`)에서 `stellar-project`의 허용 의존에 `stellar-target-profile`이 없고,
    target-profile은 core-types만 허용합니다.
  - 이 연결을 맡은 다른 WBS 행은 계획서에서 찾지 못했습니다.
- `[x]`로 되돌리려면 다음이 필요합니다(이 행의 남은 범위).
  1. project → target-profile 의존 edge 결정(crate graph 변경이라 ADR 필요 — 이번 수정에서는 새 ADR을 만들지 않았습니다).
  2. project `Manifest`의 `target_profiles` 해석을 이 crate의 `ProjectTargetProfiles`로 바꾸고 `default_target_profile` 키를
     허용(`YamlValue` ↔ `ManifestNode` 변환).
  3. `docs/spec/stellar-format.md` §14.5 스키마 표 갱신.
  4. 실제 `stellar.yaml` 텍스트로 manifest round-trip과 invalid default fixture 재검증.
