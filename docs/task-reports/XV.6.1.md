# 작업 보고서: XV.6.1 local override 매핑과 스키마 검증

- WBS: XV.6.1
- 작업명: `stellar.yaml` local override가 특정 dependency logical name을 local Stellar project root로 매핑
- 상태: 완료 (`[x]`)
- 선행: XV.1.1(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/dependencies/src/local_override.rs`(신규) — `LocalOverrideRoot`, `LocalOverrideMap`,
  `local_overrides`, lexical 정규화.
- `crates/dependencies/src/declaration.rs` — `LocalOverridePath::new` 경로 형식 검사 추가,
  `is_absolute`, `components`.
- `crates/dependencies/src/node.rs` — `ValueError::{InvalidOverridePath, OverrideInsideProject}`.
- `crates/dependencies/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XV.6.1.md`(신규), 계획서(XV.6.1 `[ ]`→`[x]`).

## 구현 요약

- **매핑**: 사용 프로젝트 선언의 `local_override`를 logical name → `LocalOverrideRoot { declared, root }`
  로 모읍니다. 상대 경로는 사용 프로젝트 root 기준, 절대 경로는 그대로이며 lexical로 `..`을 접어
  정규화합니다. library 매니페스트의 전이 override는 resolver가 무시하므로 매핑 대상이 아닙니다.
- **스키마 검사**: 경로 형식(구분자 `/`만 — `\` 금지(아키텍처 19.2), 빈 component·끝 `/`·`.`
  component 금지, 제어 문자 금지)은 파싱 시점(`LocalOverridePath::new`)에, 위치(사용 프로젝트 root
  자신·내부 금지 — 자기 참조와 프로젝트 안 의존성 checkout 방지)와 같은 이름 중복은
  `local_overrides`에서 `dependencies.<name>.local_override` 경로와 함께 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 52 passed, 0 failed; doctest 1 passed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 형제·상위·절대 경로와 `..` 경유 경로가 정규화된 root로 매핑, override 없는 의존성은 제외, 이름 순(`override_maps_logical_name_to_local_project_root`) | 통과 |
| 프로젝트 내부(`design`, `libs/ds`, `../app`, `../app/vendor/ds`, 절대 내부, `sub/../../app`) 거부, 접두사만 같은 형제(`../app-lib`) 허용(`override_inside_or_equal_to_project_is_rejected`) | 통과 |
| 형식 위반(`a//b`, `x/`, `./ds`, `../ds/.`, `a\b`, `/`, 제어 문자) 거부, `DeclNode` 스키마 경로에서 거부, 같은 이름 override 중복 거부(`override_path_syntax_is_validated`) | 통과 |
| 기존 XV.1.1·XV.1.4 테스트(override `../icons`, 절대 경로) 유지 | 통과 |

## 차이(작업 지시와 다른 점)

- override는 XV.1.1의 entry 필드(`local_override`)로 선언합니다. 별도 최상위 매핑을 두지 않아
  logical name 매핑이 구조적으로 보장됩니다.

## 남은 위험 / 한계

- 위치 검사는 lexical입니다. symlink를 거쳐 프로젝트 안을 가리키는 경우와 대상이 실제 Stellar
  프로젝트인지는 XV.6.2(probe)에서 확인해야 합니다.
- `stellar.yaml`은 Git으로 추적되므로 개인 로컬 경로가 commit될 수 있습니다. release 검증 정책은
  XV.6.5(선행 XV.6.3 차단) 범위입니다.

## 상태 정정(통합 리뷰, 2026-11-14)

- `[x]` → `[~]`: XV.1.1과 같은 이유로 실제 stellar.yaml 파서 연결이 남았습니다.

## 상태 복원(deps_wiring lane, 2026-11-14)

- `[~]` → `[x]`: 엄격 선행 XV.1.1 `[x]`.
- 실제 파서 경로 검증: 실제 파서가 읽은 `local_override`를 `Manifest::local_overrides`가 logical name → root로 매핑하고, 프로젝트 내부 대상·형식 위반(`\`, 빈 component, `.` component)을 경로와 함께 거부(`manifest::tests::local_override_schema_is_validated_through_stellar_yaml_parser`).
- 공통 근거: `Fix: stellar.yaml 의존성 선언을 실제 매니페스트 파서에 연결`(리뷰 P2), `Fix: stellar.lock.yaml entry 형식을 IV.1.6 모델 하나로 통합`(리뷰 P3, ADR-0021), `Fix: resolver library 매니페스트 읽기를 실제 stellar.yaml 파서로 연결`.
- 실행(`DEV-MAC`): `cargo test -p stellar-project` lib 261 passed·통합 17 passed, `cargo test -p stellar-dependencies` 57 passed·doctest 1 passed, `cargo clippy -p stellar-project -p stellar-dependencies --all-targets -- -D warnings` 경고 0, `cargo fmt -p stellar-project -p stellar-dependencies -- --check` 통과.
