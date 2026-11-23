# 작업 보고서: XXI.4.1 versioned Host API IDL과 TypeScript·Rust binding 생성

- WBS: XXI.4.1
- 작업명: Host API를 versioned IDL/WIT 또는 동등 schema로 정의하고 TypeScript/Rust bindings 생성
- 상태: 완료 (`[x]`)
- 선행: XXI.1.1
- 결정: [ADR-0097](../adr/ADR-0097-plugin-host-core.md) 결정 1·6

## 필수 실행 환경

- `DEV-MAC` (Rust, 로컬 검증. TypeScript 형식 검사는 deno).

## 변경 파일 목록

- `schemas/plugin-host-api.idl.json`(신규, 원본 IDL) — `idl_version` 1, `host_api_version` 1, record type 24개, namespace 7개.
- `crates/plugin-host/src/idl.rs`(신규) — IDL 해석·검사, `ParamSpec`, 결과 형태 검사 `conforms`.
- `crates/plugin-host/src/bindgen.rs`(신규) — Rust·TypeScript 생성기.
- `crates/plugin-host/src/generated.rs`, `crates/plugin-host/src/generated/host_api.rs`(생성) — `HOST_API_VERSION`, `Namespace`, `Method`(이름·권한·매개변수 표·결과 type).
- `crates/plugin-host/bindings/stellar-host-api.ts`(생성) — type 선언, `MethodMap`, `createHostClient`.
- `crates/plugin-host/src/generated_files.rs`, `src/lib.rs` — 생성 표와 module 등록. `HOST_API_VERSION`은 생성 상수를 다시 냅니다.

## 구현 요약

- IDL은 JSON 문서이고 type·namespace·method·field·param 이름 중복, 모르는 type, 모르는 권한, record 매개변수, 고빈도 namespace
  (`pointer`, `render`, `frame`, `input`, `animation`)를 거부합니다.
- 생성 Rust는 method마다 wire 이름, namespace, 필요 권한, 매개변수 표, 결과 type을 냅니다. dispatcher(XXI.4.2)가 이 표만 씁니다.
- 생성 TypeScript는 record interface, method별 params interface, `MethodMap`, namespace별 `HostClient`, id 부여·batch·조각 재조립을
  하는 `createHostClient(transport)`를 냅니다.
- 생성 파일은 `stellar-plugin-bindgen`이 쓰고 `tests/schema_drift.rs`가 byte 비교합니다. 생성 Rust module은 `#[rustfmt::skip]`으로
  rustfmt가 바꾸지 않게 했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-plugin-host` | 전체 통과(IDL unit 2, 생성기 unit 1, drift 1 포함) |
| `cargo run -p stellar-plugin-host --bin stellar-plugin-bindgen -- --check` | drift 0 |
| `deno check crates/plugin-host/bindings/stellar-host-api.ts` | 통과 |
| `cargo clippy -p stellar-plugin-host --all-targets -- -D warnings` | 경고 0 |

## 차이(작업 지시와 다른 점)

- WIT 대신 JSON IDL과 자체 생성기를 택했습니다(ADR-0097 Alternatives).

## 남은 위험 / 한계

- TypeScript 형식 검사는 deno가 있는 환경에서만 했습니다. 저장소 test는 byte drift만 검사합니다.
