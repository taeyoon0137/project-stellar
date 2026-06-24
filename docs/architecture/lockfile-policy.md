# Lockfile 정책 (II.1.5)

## 원칙

- 저장소 루트의 `Cargo.lock` 하나가 제품 workspace 전체의 단일 lockfile입니다. Git 추적 대상입니다.
- 모든 CI Rust 명령은 `--locked`를 사용합니다(`cargo check/test/clippy --locked`).
- `--locked`는 lockfile이 없거나 `Cargo.toml`과 불일치하면 명령을 실패시킵니다.

## Drift 금지

- 의존성 변경은 전용 dependency-update WBS에서만 수행하며, 같은 변경에서 `Cargo.lock`을 갱신합니다.
- 그 외 작업에서 `Cargo.lock`이 바뀌면(무단 drift) `--locked` 검사가 실패해 병합을 차단합니다.
- lockfile 누락 시에도 `--locked`가 실패합니다(`cannot create the lock file ... because --locked was passed`).

## 검증

- `cargo check --workspace --locked` 통과 + `git diff --quiet Cargo.lock`(변경 없음)이면 정상입니다.
- `Cargo.lock`을 제거하거나 `Cargo.toml` 의존성을 바꾼 뒤 lockfile을 갱신하지 않으면 `--locked` 명령이 실패합니다.
