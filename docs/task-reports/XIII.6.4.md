# 작업 보고서: XIII.6.4 External Font 패키지 참조 인터페이스

- WBS: XIII.6.4
- 작업명: External Font는 관리형 패키지의 고정 hash/face를 참조하는 인터페이스만 구현하고 다운로드 서비스는 제공하지 않음
- 상태: 완료 (`[x]`)
- 선행: X.8.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/fonts/external.rs`(신규) — `ExternalFontPackageName`, `ExternalFontPackageVersion`,
  `ExternalFontPackage`, `ExternalFontRef`, `ExternalFontRefError`, `InstalledFontPackage`,
  `InstalledPackageError`, `ExternalFontPackageProvider`, `ExternalFontDiagnostic(Kind)`,
  `ExternalFontResolution`, `resolve_external_fonts`.
- `crates/project/src/fonts/mod.rs` — `external` 모듈 추가.
- `docs/task-reports/XIII.6.4.md`(신규), 계획서(XIII.6.4 `[ ]`→`[x]`).

## 구현 요약

- **참조**: `ExternalFontRef`는 패키지(이름 + 정확한 버전)와 `FontFaceRef`(출처 External, 파일 hash,
  face index, variation)를 기록합니다. 버전은 `[0-9A-Za-z.+-]`만 허용해 범위 연산자를 표현할 수
  없고, 이름은 소문자 segment 규칙(`.`·`..`·빈 segment 금지)과 길이 상한을 둡니다.
- **인터페이스만**: `ExternalFontPackageProvider::installed_package`는 로컬에 이미 설치된 패키지의
  (파일 hash → face 수)만 돌려줍니다. 다운로드·네트워크 메서드는 없고, 문서로 구현에 금지합니다.
- **해석**: `resolve_external_fonts`는 패키지마다 provider를 한 번만 조회하고, 설치된 패키지에
  기록한 hash+index가 있으면 `FontSource::External`로 registry에 등록합니다. variation은 face
  정체성에 영향을 주지 않습니다(X.8.2).
- **진단**: 설치되지 않은 패키지는 `missing-external-font-package`, 패키지에 기록한 face가 없으면
  `missing-external-font-face` 진단 하나로 묶고 영향 face를 정렬·중복 제거해 나열합니다. 다른
  폰트로 대체하지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 283 passed, 1 ignored, 0 failed; 통합 3·5·5·4 passed) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |

| fixture | 결과 |
|---|---|
| **missing external package diagnostic**: 두 패키지 중 미설치 패키지 → 진단 1개(code, 패키지, 영향 face 2개 중복 제거), 미설치 face handle 없음, 설치 face만 External로 등록, provider 패키지당 1회 조회(`missing_external_package_is_diagnosed_without_substitution`) | 통과 |
| 설치 패키지에 없는 hash·범위 밖 face index → `missing-external-font-face`(`installed_package_without_recorded_face_is_diagnosed`) | 통과 |
| variation이 달라도 같은 face handle(`variations_do_not_change_face_identity`) | 통과 |
| registry 상한 오류 전달(`registry_capacity_error_is_returned`) | 통과 |
| 이름·정확한 버전 규칙, External 아닌 출처 거부(`package_identity_rules`) | 통과 |
| 설치 내용 face 수 범위·충돌 검사(`installed_package_contents_are_validated`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 실제 관리형 패키지 cache provider(설치 위치, 설치 시 파일 hash 검증)는 구현하지 않았습니다. 이
  모듈은 provider가 hash를 검증했다고 가정합니다.
- 패키지 이름·버전을 `stellar.yaml`·`stellar.lock.yaml` 의존성 모델(Phase XV)과 연결하는 작업은
  이 행 범위 밖입니다.
