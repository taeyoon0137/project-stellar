# 작업 보고서: XVI.7.2 신뢰 grant/revoke API와 OS 사용자 데이터 저장

- WBS: XVI.7.2
- 작업명: trust grant/revoke API를 구현하고 OS user data store에 기록(project file/Git config 미변경)
- 상태: 완료 (`[x]`)
- 선행: IV.5.2(신뢰 상태 인터페이스, `trust_key` = canonical target)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증; symlink·권한 테스트는 `cfg(unix)`).

## 변경 파일 목록

- `crates/project/src/trust_store.rs` (신규: `FileTrustStore`, `TrustStoreError`, `os_user_data_base`,
  `TRUST_DIR`/`TRUST_FILE`/`TRUST_STORE_SCHEMA_VERSION`, fixture 테스트)
- `crates/project/src/lib.rs` (`pub mod trust_store;`)
- `docs/task-reports/XVI.7.2.md`(신규), 계획서(XVI.7.2 `[ ]`→`[x]`)

## 구현 요약

- `FileTrustStore::open(data_dir)`: `<data_dir>/trust/trusted-projects.yaml`을 읽습니다. 파일이 없으면
  빈 저장소이며 **파일·디렉터리를 만들지 않습니다**. 손상 파일(YAML 문법·스키마·알 수 없는 키·
  중복 target·상대 target)은 오류로 반환하고 덮어쓰지 않습니다.
- `grant(&identity)` / `revoke(&identity)`: 새 목록을 같은 디렉터리의 임시 파일에 쓰고(`create_new`,
  Unix `0600`, `sync_all`) rename으로 교체합니다. 기록에 성공한 뒤에만 메모리 상태를 바꿉니다.
  기록이 없는 revoke는 파일을 건드리지 않습니다.
- `status(&identity)`: IV.5.2 정책대로 `trust_key`(canonical target)로 찾고, 추가로 **project id가
  일치할 때만** `Trusted`입니다(같은 경로에 다른 프로젝트가 놓이면 비신뢰).
- 형식: 제한 YAML canonical writer(IV.1.5)로 `schema_version`·`trusted[{project_id, target}]`를 target
  정렬 순으로 기록합니다(삽입 순서와 무관하게 byte-identical).
- `os_user_data_base()`: macOS `$HOME/Library/Application Support`, Windows `%APPDATA%`, 그 밖의 Unix
  `$XDG_DATA_HOME`(절대 경로) 또는 `$HOME/.local/share`. 환경 변수가 없으면 `None`(추측하지 않음).
- 프로젝트 폴더·`.git/config`·Git 전역 설정은 읽지도 쓰지도 않으며 Git 프로세스를 실행하지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 137 passed, 0 failed; 신규 10개 포함) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |

| fixture | 결과 |
|---|---|
| 저장소 없음 → `NotTrusted`, 열기만으로 파일 생성 없음 | 통과 |
| **grant → Trusted → reopen 후 Trusted → revoke → reopen 후 NotTrusted** | 통과 |
| **grant/revoke 전후 프로젝트 트리(매니페스트·`.git/config`·문서) bytes 동일**, 저장 파일은 프로젝트 밖 | 통과 |
| 삽입 순서가 달라도 저장 파일 byte-identical, 임시 파일 잔존 없음 | 통과 |
| 같은 target·다른 project id → `NotTrusted` | 통과 |
| reopen 후 symlink alias 신뢰 공유, root rename 신뢰 비계승(IV.5.2 정책 유지) | 통과 |
| 저장 파일 권한 `0600` | 통과 |
| 기록 없는 revoke → 파일 미생성 | 통과 |
| 손상 저장 파일 5종 + 중복 target 거부, 원본 미수정 | 통과 |
| OS별 user data base 규칙, 환경 변수 없음 → `None` | 통과 |

## 차이(작업 지시와 다른 점)

- IV.5.2의 `TrustStore` trait(`set`/`forget`가 오류를 반환하지 않음)를 구현하지 않고, 영속 실패를
  보고할 수 있도록 `grant`/`revoke`가 `Result`를 반환하는 별도 API로 만들었습니다. 판정 키는 같은
  `trust_key`입니다.
- 앱 전용 하위 폴더 이름(번들 식별자 등)은 셸 연결 시 정할 값이라 고정하지 않았습니다. API는
  `data_dir`을 인자로 받고, OS 기본 base만 `os_user_data_base`로 제공합니다.

## 남은 위험 / 한계

- 여러 프로세스가 동시에 grant/revoke하면 마지막 기록이 이깁니다(파일 잠금 없음). 다중 인스턴스
  잠금은 후속 과제입니다.
- UTF-8로 표현할 수 없는 경로는 `NonUtf8Path`로 기록을 거부합니다.
- rename 후 부모 디렉터리 fsync는 하지 않습니다(전원 손실 시 직전 목록으로 남을 수 있음).
- Windows `%APPDATA%` 경로 실행과 ACL 권한은 이 환경에서 검증하지 않았습니다(`cfg(unix)` 권한
  테스트만 실행). UI 연결과 capability matrix(XVI.7.1)는 후속입니다.
