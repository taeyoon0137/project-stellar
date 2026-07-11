# 작업 보고서: V.8.6 migration golden fixture runner

- WBS: V.8.6
- 작업명: `cargo xtask test-migrations` — migration golden fixture runner
- 상태: 완료 (`[x]`)
- 선행: V.8.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 실행).

## 변경 파일 목록

- `tools/xtask/src/migration_fixtures.rs`(신규) — fixture 정의와 runner.
- `tools/xtask/src/commands.rs` — `test-migrations` 명령 등록(COMMANDS 17개).
- `tools/xtask/src/lib.rs` — 모듈 등록.
- `docs/task-reports/V.8.6.md`(신규), 계획서(V.8.6 `[ ]`→`[x]`).

## 구현 요약

- **fixture 형식**: `(이름, 시작 스키마, 시작 chunk들, 최신 스키마 기대 chunk들)`.
  현재 과거 fixture 2종(v1 최소, v1 + view 2개)이며 새 스키마 버전이 생길 때마다
  fixture는 유지하고 기대 snapshot만 최신으로 갱신합니다.
- **runner 흐름**: fixture 문서를 실제 저장 파이프라인(V.5)으로 임시 프로젝트에
  구성 → 제품 registry(현재 noop v1→v2)로 최신 스키마(`LATEST_SCHEMA = 2`)까지
  migration(V.8.3 runner) → 결과 세대를 전체 검증 로드해 chunk snapshot을 기대값과
  비교합니다. 실패 fixture는 이름과 원인을 모아 exit 65로 보고합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo run -p xtask -- test-migrations` | exit 0, "2 fixtures 통과" |
| `cargo test -p xtask` | 114 passed, 0 failed |
| `cargo fmt -p xtask -- --check` | 통과 |
| `cargo clippy -p xtask --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 모든 과거 fixture가 최신 snapshot으로 정확히 변환(`all_golden_fixtures_migrate_to_latest_snapshot`) | 통과 |

## 차이(작업 지시와 다른 점)

- fixture를 외부 파일 대신 코드로 정의했습니다. binary chunk payload를 담아야
  해 코드 정의가 더 검증 가능하고, runner가 실제 파이프라인으로 fixture를
  구성하므로 포맷 변화에도 fixture가 깨지지 않습니다.

## 남은 위험 / 한계

- 실제 내용 변환 migration(v2 이후)이 추가되면 fixture의 기대 snapshot 갱신
  규율이 필요합니다. runner가 스키마 미도달·불일치를 모두 실패로 보고하므로
  누락은 CI에서 드러납니다(연결은 후속).
