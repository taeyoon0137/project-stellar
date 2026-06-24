# 작업 보고서: I.3.2 의존성 정책과 validator

- WBS: I.3.2
- 작업명: `config/dependencies.yaml` 생성 + validator, bootstrap 의존성 정식 이전·파일 삭제
- 상태: 완료 (`[x]`)
- 선행: I.3.1, I.2.3 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `config/dependencies.yaml` (신규, yaml-rust2 승인 항목)
- `config/bootstrap-dependencies.yaml` (삭제)
- `tools/xtask/src/dependencies.rs` (신규)
- `tools/xtask/src/lib.rs` (`pub mod dependencies` 추가)
- `docs/task-reports/I.3.2.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.3.2 `[ ]` → `[x]`)

## 구현 요약

- 필드: name, ecosystem, version, license, approval(approved/pending/rejected), purpose, replaceable(bool).
- `validate_dependencies(content, direct_dep_names)`: 승인 집합과 cargo direct 집합 비교. 코드: `unknown-dependency`(미승인 direct), `unused-approval`, `missing-field`, `unknown-field`, `invalid-approval`, `invalid-replaceable`.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| `cargo tree -p xtask --depth 1` direct dep | `yaml-rust2 v0.11.0` (승인 목록과 일치) |
| bootstrap-dependencies.yaml 존재 | 삭제됨 |
| unknown direct dep / missing-field fixture | 각 코드 |
| `cargo fmt/clippy/test -p xtask` | fmt 0, clippy 0, test 74 passed |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- license policy(허용·거부 목록)는 I.3.3에서 같은 validator에 확장합니다.
