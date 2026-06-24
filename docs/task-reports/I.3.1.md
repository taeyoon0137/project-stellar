# 작업 보고서: I.3.1 버전 manifest와 validator

- WBS: I.3.1
- 작업명: `config/versions.yaml` 생성 + `versions` validator
- 상태: 완료 (`[x]`)
- 선행: I.2.4, I.1.7 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `config/versions.yaml` (신규)
- `tools/xtask/src/versions.rs` (신규)
- `tools/xtask/src/lib.rs` (`pub mod versions` 추가)
- `docs/task-reports/I.3.1.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.3.1 `[ ]` → `[x]`)

## 구현 요약

- `validate_versions(content, toolchain_channel) -> Vec<VersionError>`.
- Rust는 exact만 허용 + `rust-toolchain.toml` channel(1.96.0)과 일치. 그 외 도구는 exact XOR range, 빈 값 금지, 중복 금지.
- 코드: `duplicate-tool`, `empty-version`, `exact-and-range`, `neither-exact-nor-range`, `rust-requires-exact`, `rust-mismatch`, `unknown-field`, `missing-rust`.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 정상 / 실제 versions.yaml(채널 일치) | 통과 |
| duplicate·empty·exact+range·rust mismatch | 각 코드 |
| `cargo fmt/clippy/test -p xtask` | fmt 0, clippy 0, test 69 passed |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 현재 실제 사용 도구는 Rust뿐이라 versions.yaml에 rust만 기록했습니다.
