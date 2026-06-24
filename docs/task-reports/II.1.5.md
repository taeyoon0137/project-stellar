# 작업 보고서: II.1.5 단일 lockfile 정책

- WBS: II.1.5
- 작업명: `Cargo.lock` 단일 lockfile 유지 + 모든 CI Rust 명령에 `--locked`
- 상태: 완료 (`[x]`)
- 선행: II.1.2 (`[~]`)

## 필수 실행 환경

- `DEV-MAC` (verify 열에 CI 태그 없음).

## 변경 파일 목록

- `docs/architecture/lockfile-policy.md` (신규, 정책 문서)
- `.github/workflows/bootstrap.yml` (clippy 단계에 `--locked` 추가, 양 job)
- `docs/task-reports/II.1.5.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: II.1.5 `[ ]` → `[x]`)

## 구현 요약

- 루트 `Cargo.lock` 하나가 제품 workspace 단일 lockfile(추적 대상).
- CI Rust 명령은 `--locked`(check/test 기존, clippy 추가). dependency-update WBS 외 drift 금지.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| `Cargo.lock` 추적 | 확인 |
| `cargo check --workspace --locked` | 종료 0, `Cargo.lock` 변경 없음 |
| lockfile 누락 + `--locked` | 종료 101 (drift 차단: "cannot create the lock file ... --locked") |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 종료 0 |
| bootstrap.yml YAML | 유효 |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 없음.
