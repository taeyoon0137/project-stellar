# 작업 보고서: IV.4.3 기본 프로젝트 scaffold

- WBS: IV.4.3
- 작업명: staging 디렉터리에 기본 scaffold(파일·폴더) 생성, golden byte-identical
- 상태: 완료 (`[x]`)
- 선행: IV.1.5(canonical writer), IV.1.7(lock writer), IV.4.2(트랜잭션)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/scaffold.rs` (신규: `write_default_scaffold`, 기본 내용 상수, golden·결정론·round-trip 테스트)
- `crates/project/src/lib.rs` (`pub mod scaffold;`)
- `docs/task-reports/IV.4.3.md`(신규), 계획서(IV.4.3 `[ ]`→`[x]`)

## 구현 요약

- `write_default_scaffold(staging, name, id)`가 staging 디렉터리에 생성:
  - `stellar.yaml`: 기본 매니페스트를 IV.1.5 canonical writer로 기록.
  - `stellar.lock.yaml`: 빈 잠금 파일을 IV.1.7 lock writer로 기록.
  - `.gitignore`/`.gitattributes`: 고정 기본 내용(텍스트 LF 정규화 + 흔한 에셋 binary 표시).
  - `design/`, `assets/`: 빈 디렉터리.
- 같은 입력이면 결정론적으로 동일 바이트를 만듭니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-project` (전체) | 0 |
| `cargo fmt -p stellar-project -- --check` | 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| **scaffold 각 파일이 golden(canonical 계산값·고정 상수)과 byte-identical** | 통과 |
| 두 번 생성 시 byte 결정론 | 통과 |
| 기록한 stellar.yaml/lock이 loader로 동일 모델 복원 | 통과 |
| design/·assets/ 디렉터리 생성 | 통과 |

## 차이(작업 지시와 다른 점)

- golden은 별도 체크인 파일 대신 **canonical 계산값**(manifest/lock writer 출력)과 고정 상수로
  정의하고 byte-identical을 단언합니다(self-contained). 추가로 round-trip으로 의미 보존을
  확인합니다.
- `.gitignore`/`.gitattributes` 기본 내용은 사양에 명시가 없어 최소·합리적 기본값으로 고정
  했습니다(OS cruft 무시, 텍스트 LF, 흔한 이미지 binary).

## 남은 위험 / 한계

- 본 작업은 staging 생성에 한정합니다. `git init -b main`은 IV.4.4, add/commit은 IV.4.5~IV.4.6,
  단계별 rollback 종합은 IV.4.7에서 다룹니다.
- 프로젝트 id(UUID)는 호출자가 제공합니다(결정론적 테스트를 위해 고정 id 사용).
