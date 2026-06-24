# 작업 보고서: II.1.3 panic 정책

- WBS: II.1.3
- 작업명: release `panic=abort`, 테스트 unwind, FFI 경계 정책 문서
- 상태: 완료 (`[x]`)
- 선행: II.1.2 (`[~]` — workspace 확장 DEV-MAC 검증 완료, 본 작업은 그 위에서 진행)

## 필수 실행 환경

- `DEV-MAC` (verify 열에 CI 태그 없음).

## 변경 파일 목록

- `Cargo.toml` (`[profile.release] panic = "abort"`)
- `docs/architecture/panic-policy.md` (신규, 정책 문서)
- `tools/xtask/tests/release_profile.rs` (신규, profile 검사 테스트)
- `docs/task-reports/II.1.3.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: II.1.3 `[ ]` → `[x]`)

## 구현 요약

- release = `panic=abort`. test = unwind(profile.test 상속, panic 미재정의).
- FFI 경계 catch_unwind→InternalPanic 변환은 II.4.4에서 구현(정책 문서에 명시).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| release profile에 `panic="abort"` | 테스트 통과 |
| dev profile은 unwind 유지 | 테스트 통과 |
| `cargo fmt --all -- --check` | 통과 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 종료 0 |
| `cargo test --workspace --locked` | 95 + 2 passed |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- FFI catch_unwind 테스트는 ffi crate가 생기는 II.4.4 소관입니다(본 작업은 profile·정책·문서까지).
