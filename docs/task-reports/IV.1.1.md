# 작업 보고서: IV.1.1 stellar.yaml 데이터 구조

- WBS: IV.1.1
- 작업명: `project` crate에 `stellar.yaml` 데이터 모델 + `YamlValue` 타입 + round-trip
- 상태: 완료 (`[x]`)
- 선행: Phase III

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/yaml/mod.rs`, `crates/project/src/yaml/value.rs` (신규: `YamlValue` + 헬퍼)
- `crates/project/src/manifest.rs` (신규: `Manifest`/`ProjectMeta`/`TargetProfile`/`Dependency`/`ManifestError`, `to_yaml`/`from_yaml`)
- `crates/project/src/lib.rs` (`pub mod yaml;`, `pub mod manifest;`)
- `docs/task-reports/IV.1.1.md`(신규), 계획서(IV.1.1 `[ ]`→`[x]`)

## 구현 요약

- `YamlValue`(Scalar/Sequence/Mapping) 값 트리 타입과 조회 헬퍼를 정의했습니다. 스칼라는
  문자열로 보존하며 암시적 타입 추론을 하지 않습니다(IV.1.4 대비).
- `stellar.yaml` 데이터 모델(아키텍처 20장): `schema_version`(u32), `project{id,name}`,
  `target_profiles[]`(플랫폼별 선택 필드 `minimum_os`/`baseline`), `dependencies`(이름→spec).
- `Manifest::to_yaml`(키 순서 고정) ↔ `Manifest::from_yaml`(검증·역변환)으로 round-trip.
  `from_yaml`은 필수 필드 누락·타입 불일치·알 수 없는 키·잘못된 schema_version(부호·소수점·
  공백·leading zero)을 구조화 오류로 거부합니다.
- **범위 분리**: IV.1.1은 데이터 모델 ↔ `YamlValue` round-trip만 담당합니다. 텍스트 → 값 트리
  제한 loader는 IV.1.2, 값 트리 → canonical 텍스트 writer는 IV.1.5에서 추가합니다(이번 작업은
  외부 의존성 없이 순수 Rust).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-project` (8 unit) | 0 |
| `cargo fmt -p stellar-project -- --check` | 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| 최소 예제(빈 profiles·deps) round-trip | 통과 |
| 전체 예제(2 profiles + 1 dependency, 선택 필드 보존) round-trip | 통과 |
| 필수 필드 누락·project 타입 불일치 거부 | 통과 |
| 알 수 없는 최상위 키 거부 | 통과 |
| 잘못된 schema_version(`01`,`-1`,`1.0`,`x`,``,` 1`) 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- round-trip을 텍스트가 아닌 데이터 모델 ↔ `YamlValue` 수준으로 구현했습니다. 텍스트 파서·
  canonical writer가 후속(IV.1.2/1.5)이라, 모델의 무손실 직렬화를 먼저 고정하는 분리입니다.
- `project.id`는 UUID 텍스트를 문자열로 보존합니다(StableId 매핑은 IV.2.3).
- 알 수 없는 키를 무시하지 않고 거부합니다(제한 스키마). 새 필드는 `schema_version` 상향과
  함께 도입합니다.

## 남은 위험 / 한계

- 텍스트 파싱(IV.1.2)·canonical 기록(IV.1.5) 전에는 텍스트 예제 round-trip을 직접 검증하지
  않습니다(모델↔YamlValue만). 두 작업 완료 후 텍스트 round-trip이 가능합니다.
- `target_profiles`는 플랫폼별 스키마 차이를 최소 공통 필드(`minimum_os`/`baseline`)로만
  표현합니다. 플랫폼별 상세 검증은 후속(Phase XX target profile)에서 다룹니다.
