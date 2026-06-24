# 작업 보고서: II.2.4 scene·renderer-core·renderer-metal·renderer-d3d12 골격

- WBS: II.2.4
- 작업명: scene/렌더러 골격 생성, 플랫폼 backend 게이트
- 상태: 완료 (`[x]`)
- 선행: II.2.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/{scene,renderer-core,renderer-metal,renderer-d3d12}` (각 Cargo.toml + src/lib.rs)
- `Cargo.toml` (members), `Cargo.lock`
- `docs/task-reports/II.2.4.md`, 계획서(II.2.4 `[ ]`→`[x]`)

## 구현 요약

- 의존: renderer-core → scene → core-types. metal/d3d12 → renderer-core. renderer-core는 플랫폼 backend를 역참조하지 않음.
- 플랫폼 backend는 feature 없이 `cfg(target_os = "macos"/"windows")`로 게이트 → 비대상 플랫폼에서 내용 제외.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| `cargo tree -p stellar-renderer-core` | scene→core-types (backend 역참조 없음) |
| macOS: metal 내용 활성 / d3d12 빈 crate | 빌드 OK |
| `cargo check --workspace --locked` / fmt / clippy | 통과 |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- "빌드 대상 제외"는 per-target feature가 아니라 `cfg(target_os)` 게이트로 구현(비대상 플랫폼에서 crate 내용이 컴파일에서 제외됨).
