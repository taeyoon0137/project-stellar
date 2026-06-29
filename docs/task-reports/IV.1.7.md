# 작업 보고서: IV.1.7 lock writer는 resolver 결과만 생성

- WBS: IV.1.7
- 작업명: 잠금 파일 writer가 resolver 결과에서만 생성, unknown field 거부·원본 미수정
- 상태: 완료 (`[x]`)
- 선행: IV.1.6(잠금 파일 모델)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/lock.rs` (`Lockfile::write` 추가, 강제·검증 테스트)
- `docs/task-reports/IV.1.7.md`(신규), 계획서(IV.1.7 `[ ]`→`[x]`)

## 구현 요약

- `Lockfile::write(&self) -> String`을 추가했습니다. 입력은 resolver 결과를 담은 `Lockfile`뿐이고
  사용자 편집 필드를 받는 경로가 없습니다. 고정 필드 구조체만 직렬화하므로 **알려진 필드 외에는
  구조적으로 기록될 수 없습니다**(타입 시스템이 강제).
- 읽기 경로(`from_yaml`)는 순수 함수로, 알 수 없는 키가 있으면 거부할 뿐 입력을 수정하거나
  조용히 정리·재작성하지 않습니다(IV.1.6의 unknown-key 거부 재사용).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-project` (52 unit) | 0 |
| `cargo fmt -p stellar-project -- --check` | 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| writer가 모델 고정 필드(top 2개·entry 6개)만 emit | 통과 |
| resolver 결과 → 텍스트 → 모델 → 텍스트 byte-identical(canonical 재생성) | 통과 |
| **임의 unknown field lockfile 거부 + 원본 트리 미수정** | 통과 |
| entry unknown key 거부(조용한 drop 없음, IV.1.6) | 통과 |

## 차이(작업 지시와 다른 점)

- "사용자 편집 필드를 받지 않음"을 별도 런타임 검사 대신 **타입 시스템**으로 강제했습니다:
  writer 입력이 고정 필드 `Lockfile`이라 임의 필드를 주입할 표현 자체가 없습니다. 보강으로
  "emit된 키가 정확히 알려진 집합"임을 테스트로 고정했습니다.

## 남은 위험 / 한계

- 실제 파일 시스템 쓰기(원자적 저장, 권한)는 저장 계층(후속)에서 다룹니다. 본 작업은 모델 →
  canonical 텍스트 생성과 unknown-field 거부 계약에 한정합니다.
- IV.1 섹션(제한 YAML 파서) 전체가 완료되었습니다.
