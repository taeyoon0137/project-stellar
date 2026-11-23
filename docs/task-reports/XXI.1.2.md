# 작업 보고서: XXI.1.2 package 경로 탈출·symlink·중복 id·지원하지 않는 version 거부

- WBS: XXI.1.2
- 작업명: package path traversal, symlink escape, duplicate id, unsupported version 검증
- 상태: 완료 (`[x]`)
- 선행: XXI.1.1
- 결정: [ADR-0097](../adr/ADR-0097-plugin-host-core.md) 결정 3

## 필수 실행 환경

- `DEV-MAC` (Rust, 로컬 검증).

## 변경 파일 목록

- `crates/plugin-host/src/package.rs`(신규) — `PluginPackage::open`, `PluginPackage::resolve`, `scan_plugins`.
- `crates/plugin-host/src/lib.rs` — module 등록, `HOST_API_VERSION`.
- `crates/plugin-host/Cargo.toml` — `stellar-storage` 의존(package digest의 `ContentHash`).
- `crates/plugin-host/tests/package_validation.rs`(신규) — malicious fixture.

## 구현 요약

- package root가 symlink이거나 디렉터리가 아니면, 이름이 `.stellar-plugin`으로 끝나지 않으면 거부합니다.
- package 전체를 `symlink_metadata`로 훑고 symlink는 모두 거부합니다(밖·깨짐 `symlink-escape`, 안 `package-symlink`).
  FIFO 등 특수 파일, 경로 규칙에 맞지 않는 이름, 항목 4,096개·전체 256 MiB·파일 64 MiB·깊이 32 초과를 거부합니다.
- manifest 경로 필드의 traversal은 `PackagePath` 규칙으로 거부합니다. `entry`·`wasm[].path`는 훑은 파일 목록에 있어야 하고 WASM은
  magic·version 1 header를 확인합니다.
- `host_api`가 이 빌드 major와 다르면 `unsupported-host-api`, `manifest_version`이 1이 아니면 `unsupported-manifest-version`입니다.
- `scan_plugins`는 같은 id를 가진 package를 모두 `duplicate-id`로 거부합니다.
- package digest는 (상대 경로, 내용 hash) 목록의 sha256입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-plugin-host --test package_validation` | 6 passed |
| `cargo test -p stellar-plugin-host` | 전체 통과 |
| `cargo clippy -p stellar-plugin-host --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| `../`, 절대 경로, 중첩 `..`, `\` entry와 `../../` WASM 경로 거부 | 통과 |
| 밖을 가리키는 파일·디렉터리 symlink, 깨진 symlink, 안쪽 symlink, symlink package root 거부 | 통과 |
| `host_api: 2`, `manifest_version: 9` 거부 | 통과 |
| 같은 id 두 package가 모두 `duplicate-id`, 다른 id는 정상 | 통과 |

## 차이(작업 지시와 다른 점)

- package 안을 가리키는 symlink도 거부합니다(ADR-0097 결정 3: 확인·사용 시점 사이 교체 위험 제거).

## 남은 위험 / 한계

- 훑기와 실제 파일 사용(WebView 연결 뒤) 사이에 package가 바뀔 수 있습니다. `resolve`는 검증 당시 목록만 허용하지만 내용 재확인은
  WebView scheme handler 연결 때 digest로 해야 합니다.
- Windows 경로 규칙(예약 이름)은 Windows 지원이 보류라 검사하지 않습니다.
