# 작업 보고서: IV.1.3 중복 mapping key 거부

- WBS: IV.1.3
- 작업명: 중복 mapping key를 위치와 함께 거부(마지막 값 덮어쓰기 금지)
- 상태: 완료 (`[x]`)
- 선행: IV.1.2(제한 loader)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/yaml/loader.rs` (`DuplicateKey` 종류·코드·검사·테스트, 모듈 doc 갱신)
- `docs/task-reports/IV.1.3.md`(신규), 계획서(IV.1.3 `[ ]`→`[x]`)

## 구현 요약

- 매핑 키를 받는 시점(`pending_key` 설정 직전)에 같은 매핑의 `pairs`에 동일 스칼라 키가
  이미 있으면 `DuplicateKey`(code `duplicate-key`)로 거부합니다. 검사를 키 이벤트에서 하므로
  오류 위치(line/column)가 **두 번째 키**를 가리킵니다.
- 마지막 값으로 덮어쓰지 않고 거부합니다.
- 중복 판정은 **같은 매핑 안**에서만 합니다. 서로 다른 매핑(스코프)의 동일 키는 허용합니다.
- 따옴표 여부와 무관하게 같은 스칼라 값이면 동일 키로 봅니다(`a`와 `"a"`는 중복).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-project` (28 unit) | 0 |
| `cargo fmt -p stellar-project -- --check` | 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| top-level 중복(`a: 1\na: 2`) → `duplicate-key`, line=2 | 통과 |
| nested 중복(`m:\n  x: 1\n  x: 2`) → `duplicate-key`, line=3 | 통과 |
| flow 중복(`{a: 1, a: 2}`) → `duplicate-key` | 통과 |
| 따옴표/평문 동일 키(`a`/`"a"`) 중복 | 통과 |
| 다른 매핑·시퀀스 내 매핑의 동일 키는 허용 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 지시대로 위치 포함 거부·덮어쓰기 금지를 구현했고, top-level·nested·flow·스코프 분리
  사례를 모두 검증했습니다.

## 남은 위험 / 한계

- 중복 검사는 매핑당 선형 탐색(`pairs.iter().any`)입니다. 매니페스트 매핑 크기는 작아 실무상
  문제 없으며, 큰 매핑이 필요해지면 해시 기반으로 최적화할 수 있습니다.
