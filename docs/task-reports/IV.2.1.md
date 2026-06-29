# 작업 보고서: IV.2.1 플랫폼 공통 폴더 이름 검증기

- WBS: IV.2.1
- 작업명: macOS·Windows 공통 폴더 이름 검증기(예약 문자·이름, trailing, 대소문자 충돌, 길이)
- 상태: 완료 (`[x]`)
- 선행: III.3.4(문자열·정규화 경계)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/folder_name.rs` (신규: `validate_folder_name`, `collision_key`, `has_case_collision`, `FolderNameError`, 테스트)
- `crates/project/src/lib.rs` (`pub mod folder_name;`)
- `docs/task-reports/IV.2.1.md`(신규), 계획서(IV.2.1 `[ ]`→`[x]`)

## 구현 요약

- Windows가 가장 제약이 크므로 그 규칙을 플랫폼 공통 하한으로 적용합니다.
- `validate_folder_name`: 빈 이름, `.`/`..`, 길이 상한(255자) 초과, 예약 문자
  (`< > : " / \ | ? *`), 제어 문자, trailing space/dot, 예약 장치 이름
  (CON/PRN/AUX/NUL/COM1-9/LPT1-9, 확장자 포함 `CON.txt`도 거부)을 각각 고유 오류로 거부합니다.
- `collision_key`/`has_case_collision`: 대소문자 무시 충돌 판정. 이름은 상위 계층에서 Unicode
  NFC로 정규화된 UTF-8이라고 가정하고(정규화 경계 III.3.4) case-fold를 적용합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-project` (61 unit) | 0 |
| `cargo fmt -p stellar-project -- --check` | 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| 예약 문자 9종 각각 거부 | 통과 |
| 예약 장치 이름(대소문자·확장자 포함) 거부, `COM0`/`COM10`/`CONfig`는 허용 | 통과 |
| 제어 문자·trailing space/dot·`.`/`..`·빈 이름·길이 초과 거부 | 통과 |
| **Unicode NFC 이름**(café·日本語·Москва·그래픽-자산 등) 허용 | 통과 |
| 대소문자 충돌 탐지(Docs/docs, Café/café) | 통과 |

## 차이(작업 지시와 다른 점)

- Unicode NFC 정규화 알고리즘은 구현하지 않았습니다(core-types와 동일하게 std 전용, NFC 테이블
  없음, III.3.4 원칙). 검증기는 NFC로 정규화된 입력을 가정하고, NFC 합성형 Unicode 이름이 정상
  허용됨을 fixture로 검증합니다. 실제 정규화는 정규화 경계(상위 계층)에서 수행합니다.
- 길이 상한은 문자 수 255로 두었습니다(Windows 컴포넌트 255 UTF-16에 맞춘 보수적 값).

## 남은 위험 / 한계

- 디렉터리 내 실제 충돌 검사(`has_case_collision`)는 호출자가 NFC 정규화한 이름 집합을 넘긴다고
  가정합니다. NFC 정규화 자체는 상위 계층 책임입니다.
- macOS의 `:`(Finder 표시) 등 플랫폼별 추가 제약은 Windows 규칙이 이미 더 엄격하므로 공통
  하한으로 충분합니다.
