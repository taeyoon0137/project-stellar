# 작업 보고서: XXI.1.1 `.stellar-plugin` package와 `plugin.yaml` schema

- WBS: XXI.1.1
- 작업명: `.stellar-plugin` directory package와 `plugin.yaml` schema 정의(id, version, entry HTML, permissions, host API version, optional WASM modules)
- 상태: 완료 (`[x]`)
- 선행: G8(공개 API ADR) — [ADR-0097](../adr/ADR-0097-plugin-host-core.md)
- 결정: ADR-0097 결정 1·2

## 필수 실행 환경

- `DEV-MAC` (Rust, 로컬 검증).

## 변경 파일 목록

- `crates/plugin-host/`(신규 crate `stellar-plugin-host`): `Cargo.toml`, `src/lib.rs`, `src/manifest.rs`, `src/path.rs`,
  `src/permission.rs`, `src/json.rs`(mcp-core 사본), `src/generated_files.rs`, `src/bin/bindgen.rs`.
- `crates/plugin-host/tests/`: `manifest_fixture.rs`, `schema_drift.rs`, `json_copy_sync.rs`,
  `fixtures/hello.stellar-plugin/`(plugin.yaml, ui/index.html, wasm/add.wasm).
- `schemas/plugin-manifest.schema.json`(생성).
- `docs/adr/ADR-0097-plugin-host-core.md`(신규).
- 등록: `Cargo.toml` workspace member, `Cargo.lock`, `tools/xtask/src/crate_graph.rs`의 허용 edge 한 항목,
  `docs/architecture/crate-dependency-graph.md` 표·graph 줄.

## 구현 요약

- `PluginManifest::parse`가 제한 YAML loader(`stellar_project::yaml::load_restricted`)로 읽고 필드를 엄격 해석합니다. 모르는 key,
  필수 누락, 형식 오류, `manifest_version` 1 이외는 거부합니다.
- 경로 필드는 `PackagePath::parse` 하나로 해석합니다(절대·`..`·`.`·빈 구성 요소·`\`·`:`·제어 문자·길이·깊이 거부).
- 권한은 닫힌 enum 7종(`document.read`, `document.apply`, `git.read`, `file`, `network`, `terminal`, `clipboard`)입니다.
- JSON Schema는 `manifest_json_schema()`가 같은 상수로 만들고 `stellar-plugin-bindgen`이 씁니다. drift test가 byte 비교합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-plugin-host` | 통과(manifest·path·permission unit, fixture 2, drift 1, json 사본 2) |
| `cargo run -p stellar-plugin-host --bin stellar-plugin-bindgen -- --check` | drift 0 |
| `cargo clippy -p stellar-plugin-host --all-targets -- -D warnings` | 경고 0 |
| `cargo xtask validate-crate-graph` | 통과 |

## 차이(작업 지시와 다른 점)

- schema는 손으로 쓰지 않고 Rust 상수에서 생성합니다(해석기와 schema가 갈라지지 않게).

## 남은 위험 / 한계

- JSON Schema 검증기를 저장소에 두지 않아 fixture를 schema 검증기로 돌리지 않았습니다. schema 속성·권한 enum이 해석기 표와
  같은지만 test로 확인합니다.
