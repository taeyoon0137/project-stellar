# 작업 보고서: XXI.4.5 플러그인별 권한 승인·저장

- WBS: XXI.4.5
- 작업명: 파일, network, Git, terminal, clipboard, document apply 권한을 plugin별로 승인·저장
- 상태: 완료 (`[x]`) — 엔진 범위. 승인 대화상자는 UI 재설계 때 이 API에 연결합니다.
- 선행: XXI.4.2
- 결정: [ADR-0097](../adr/ADR-0097-plugin-host-core.md) 결정 5

## 필수 실행 환경

- `DEV-MAC` (Rust, 로컬 검증).

## 변경 파일 목록

- `crates/plugin-host/src/grants.rs`(신규) — `GrantStore`(`open`, `grant`, `revoke`, `forget`, `effective`), `GrantScope`,
  `default_grants_path`.
- `crates/plugin-host/src/lib.rs` — module 등록.
- `crates/plugin-host/tests/permission_matrix.rs`(신규).

## 구현 요약

- 권한 7종(`document.read`, `document.apply`, `git.read`, `file`, `network`, `terminal`, `clipboard`)을 plugin id별로 승인합니다.
- 범위: `Session`(메모리, §37.3 세션 단위 승인), `Persistent`(파일). 저장 파일은 `<OS 사용자 데이터>/xyz.taeyoon.stellar/plugins/grants.json`
  이고 승인 당시 package digest를 같이 둡니다. digest가 바뀐 package는 저장 승인을 쓰지 않아 다시 승인받아야 합니다.
- 유효 권한 = manifest 선언 ∩ (session ∪ 같은 digest 저장 승인). 선언하지 않은 권한 승인은 `permission-not-declared`입니다.
- 쓰기는 임시 파일 뒤 rename, 형식이 틀린 파일(모르는 권한·형식 버전·key, sha256 아닌 digest)은 열기에서 거부합니다.
- 실행 중 session은 `HostSession::set_permissions`로 승인·철회를 반영합니다(XXI.4.3).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-plugin-host --test permission_matrix` | 4 passed |
| `cargo clippy -p stellar-plugin-host --all-targets -- -D warnings` | 경고 0 |

| 검사(permission matrix) | 결과 |
|---|---|
| 승인 없음·권한 하나씩 7가지 × method 12개: 필요 권한이 승인과 다를 때만 `permission-denied` | 통과 |
| 저장 승인은 다시 열어도 남고 session 승인은 남지 않음, 철회는 파일에서도 지움 | 통과 |
| 선언하지 않은 권한 승인 거부, dispatcher에 넘겨도 유효 권한 아님 | 통과 |
| package 내용 변경 뒤 저장 승인 무효 | 통과 |
| 손상된 저장 파일 5종 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- terminal·clipboard는 승인·저장만 있고 그 권한을 쓰는 Host method는 아직 없습니다.
- 사용자에게 묻는 승인 UI는 없습니다(UI 재설계 때).
