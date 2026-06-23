# 작업 보고서: I.1.7 Rust 도구체인 고정

- WBS: I.1.7
- 작업명: `rust-toolchain.toml`로 Rust 1.96.0 + rustfmt + clippy 고정
- 상태: 완료 (`[x]`)
- 선행: I.1.6 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`. Windows 완료를 의미하지 않음.

## 변경 파일 목록

- `rust-toolchain.toml` (신규)
- `docs/task-reports/I.1.7.md` (신규, 본 보고서)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.1.7 `[~]` → `[x]`)

## 고정 내용

```toml
[toolchain]
channel = "1.96.0"
components = ["rustfmt", "clippy"]
profile = "minimal"
```

- `config/versions.yaml`은 만들지 않음(계획 I.3.1 소관).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 | 종료 코드 |
|---|---|---|
| `rustup show active-toolchain` | `1.96.0-...-darwin (overridden by rust-toolchain.toml)` | 0 |
| `rustc -Vv` | `release: 1.96.0` | 0 |
| `cargo -V` | `cargo 1.96.0` | 0 |
| `cargo fmt --version` | `rustfmt 1.9.0-stable` | 0 |
| `cargo clippy -V` | `clippy 0.1.96` | 0 |

## 남은 경고 수

- 0

## 성능 수치

- 해당 없음

## 작업 지시와 다른 점

- 계획서 검증 명령의 `cargo fmt -V`는 cargo fmt가 받지 않는 형식이라 동등한 `cargo fmt --version`을 사용했습니다(결과 동일, 종료 0).
