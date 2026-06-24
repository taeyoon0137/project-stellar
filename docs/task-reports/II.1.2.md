# 작업 보고서: II.1.2 제품 workspace 확장

- WBS: II.1.2
- 작업명: xtask-only workspace를 제품 workspace로 확장(profile·metadata·lint)
- 상태: `[~]` (DEV-MAC 검증 완료. `[CI-MAC][CI-WIN]` 검증은 CI에서 확보 — 미완료)
- 선행: II.1.1 (`[~]`)

## 필수 실행 환경

- `DEV-MAC`, `CI-MAC`, `CI-WIN`.

## 변경 파일 목록

- `Cargo.toml` (workspace.package, workspace.lints.rust, profile.dev/release 추가)
- `tools/xtask/Cargo.toml` (`[lints] workspace = true`)
- `docs/task-reports/II.1.2.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: II.1.2 `[ ]` → `[~]`)

## 구현 요약

- `[workspace.package]`: edition 2021, rust-version 1.96.0, repository.
- `[workspace.lints.rust]`: `unused = "warn"` (unsafe 강화는 II.1.4).
- `[profile.dev]`/`[profile.release]` 기본 선언(panic 정책은 II.1.3).
- member는 여전히 `xtask` 하나. 기능 crate 미추가.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| `cargo metadata --no-deps` member | `["xtask"]` 하나 |
| `cargo fmt --all -- --check` | 통과 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 종료 0 |
| `cargo test --workspace --locked` | 95 passed |

## 미완료(연기) — `[~]` 사유

- `[CI-MAC][CI-WIN]`에서 동일 검증은 GitHub Actions에서만 확인 가능. 확보 후 `[x]`.

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 없음.
