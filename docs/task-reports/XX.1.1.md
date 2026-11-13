# 작업 보고서: XX.1.1 ExportRequest 정의

- WBS: XX.1.1
- 작업명: `ExportRequest` — View/selection source, TargetProfile, scale, background, output path, format(PNG/SVG) 정의와 schema validation
- 상태: 완료 (`[x]`)
- 선행: G8, Phase XI (gate·phase 단위 선행 — 완화 적용, 아래 "차이" 참조)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/export/src/request.rs`(신규) — `ExportRequest`, `ExportSource`, `TargetProfileKey`,
  `ExportScale`, `ExportBackground`, `OutputPath`, `ExportFormat`, `ExportRequestError`,
  field schema(`REQUEST_SCHEMA`, `from_fields`/`to_fields`).
- `crates/export/src/lib.rs` — 모듈 선언·재노출.
- `docs/task-reports/XX.1.1.md`(신규), 계획서(XX.1.1 `[ ]`→`[x]`).

## 구현 요약

- **검증된 구성 타입**: 각 값은 생성 시 검증합니다. 만들어진 `ExportRequest`는 항상 schema를
  만족합니다.
  - source: View 전체 또는 그 View 안 selection. selection은 정렬 집합(입력 순서 무관),
    1..=100000개, nil·중복 id 거부. 상한을 넘는 순간 순회를 멈춥니다.
  - target profile: `stellar.yaml` `target_profiles[].id` 문자열을 검증한 key(ASCII 영숫자 시작,
    `[A-Za-z0-9._-]`, 1..=64 byte).
  - scale: finite, 0.01..=16.
  - background: 투명 또는 색공간을 가진 단색(`Color`).
  - output path: 절대 경로, `..`·NUL·후행 구분자 거부, 파일 이름 필수, 4096 byte 이하.
  - format: PNG/SVG, 안정 tag(Png=1, Svg=2). output path 확장자와 일치해야 합니다(대소문자 무시).
- **field schema**: CLI·agent tool처럼 문자열 key/value를 받는 경계용입니다. 입력은 신뢰하지
  않습니다. 미지·중복·누락·허용되지 않는 field, field별 길이 상한(parse 전 검사), 지수·`inf`·
  `NaN`·`+`를 받지 않는 십진 문법을 적용합니다. 미지 field 이름은 64 byte(UTF-8 경계)로 잘라
  오류에 담습니다. `to_fields`는 schema 순서·기본값 명시의 canonical 출력이고 `from_fields`로
  같은 요청이 복원됩니다.
- 모든 오류는 typed enum(`ExportRequestError`)이며 한국어 사용자 메시지를 가집니다. 입력 처리에
  `unwrap`이 없습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-export` | 9 passed, 0 failed |
| `cargo fmt -p stellar-export -- --check` | 통과 |
| `cargo clippy -p stellar-export --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| schema 규칙별 위반 fixture 39건이 각각 기대 오류로 거부(`schema_violations_are_rejected`) | 통과 |
| 최소 field 기본값, 전체 field canonical round-trip(`minimal_fields_use_defaults`, `full_fields_round_trip_canonically`) | 통과 |
| 길이 상한이 parse 전에 적용, 미지 이름 절단(`untrusted_lengths_are_bounded_before_parsing`) | 통과 |
| selection 상한·nil·순서 무관(`selection_limits_are_enforced`) | 통과 |
| schema table 일관성, 극단 f32 canonical 표현 round-trip(`schema_table_is_consistent`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 `G8`(초기 릴리스 gate)과 `Phase XI`(렌더러)는 gate·phase 단위 선행이라 작업 지시에 따라
  완화했습니다. 이 작업은 요청 타입 정의라 렌더러·릴리스 산출물이 필요하지 않습니다.
- TargetProfile 정식 식별자 `TargetProfileId`(IX.10.1)가 아직 없어 매니페스트 profile id 문자열을
  검증한 `TargetProfileKey`로 받습니다. crate 의존 정책(ADR-0019)상 export는 target-profile crate에
  의존할 수 없으므로, 정식 타입이 생기면 host가 key로 변환해 넘기는 방식을 유지합니다.
- "schema validation"은 저장소 `schemas/`(보호 경로) 파일이 아니라 이 crate의 field schema
  (`REQUEST_SCHEMA`)와 타입 검증으로 구현했습니다.

## 남은 위험 / 한계

- 매니페스트(IV.1.1)는 profile id 문자 집합을 제한하지 않으므로, key 규칙에 맞지 않는 id를 가진
  profile은 내보내기 요청을 만들 수 없습니다. IX.10.1에서 id 규칙이 정해지면 맞춰야 합니다.
- scale·경로 길이 상한(0.01..=16, 4096 byte)은 구현 상수입니다. PNG pixel 크기 상한은 래스터 경로
  (XX.2.1)에서 따로 검사해야 합니다.
- 파일 존재 여부·쓰기 권한·확장자 외 경로 정책은 저장 단계(XX.2.3, XX.5.1)의 책임입니다.
