# 작업 보고서: XXI.6.2 플러그인 metadata를 plugin namespace 디렉터리에만 저장

- WBS: XXI.6.2
- 작업명: collaboration metadata는 plugin namespace directory에만 저장하고 core document schema에 넣지 않음
- 상태: 완료 (`[x]`)
- 선행: XXI.1.1
- 결정: [ADR-0097](../adr/ADR-0097-plugin-host-core.md) 결정 10

## 필수 실행 환경

- `DEV-MAC` (Rust, 로컬 검증).

## 변경 파일 목록

- `crates/plugin-host/tests/plugin_data_namespace.rs`(신규).
- 구현은 XXI.4.2에서 넣은 `crates/plugin-host/src/plugin_data.rs`(`PluginData`)와 `file.*` handler입니다.

## 구현 요약

- 플러그인 파일은 프로젝트 루트의 `plugins/<plugin-id>/` 아래에만 씁니다(`file.read`·`file.write`·`file.list`, 권한 `file`).
  core 문서 schema·`design/`·`stellar.yaml`에는 아무것도 쓰지 않습니다.
- 경로는 `PackagePath` 규칙(절대·`..`·`\`·`:` 등 거부)을 따르고, 프로젝트 루트부터 대상까지 어느 구성 요소든 symlink면 거부합니다.
  쓰기는 임시 파일 뒤 rename이고 목록은 symlink·임시 파일을 건너뜁니다.
- `plugins/`는 Git이 보는 일반 디렉터리이므로 협업 플러그인이 공유할 수 있고, 지워도 프로젝트는 그대로 열립니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-plugin-host --test plugin_data_namespace` | 2 passed |

| 검사 | 결과 |
|---|---|
| 플러그인 파일 2개가 모두 `plugins/com.example.all/` 아래, core 파일 hash 동일(core metadata 0) | 통과 |
| 쓰기 뒤 project revision 동일, `validate_project` 통과 | 통과 |
| `plugins/` 삭제(플러그인 제거) 뒤 `open_snapshot`·`validate_project`·`document.read_snapshot` 정상 | 통과 |
| 경로 탈출 4종 거부, namespace 안 파일·디렉터리 symlink 읽기·쓰기 거부, `stellar.yaml` 내용 그대로 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- `plugins/`를 Git에 넣을지는 프로젝트 `.gitignore`가 정합니다. 기본 scaffold는 무시하지 않습니다.
- 바이너리 파일은 아직 받지 않습니다(UTF-8 텍스트만).
