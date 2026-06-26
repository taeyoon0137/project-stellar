# 작업 보고서: III.2.1 PropertyKey와 이름 registry

- WBS: III.2.1
- 작업명: `PropertyKey`를 고정 numeric ID로 정의, 사람이 읽는 이름은 별도 registry
- 상태: 완료 (`[x]`)
- 선행: III.1.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/core-types/src/property.rs` (`PropertyKey`, `PropertyNameRegistry`, 단위테스트)
- `crates/core-types/src/lib.rs` (`pub mod property;`)
- `docs/task-reports/III.2.1.md`, 계획서(III.2.1 `[ ]`→`[x]`)

## 구현 요약

- `PropertyKey(u32)` — 문자열이 아닌 고정 numeric 식별자.
- `PropertyNameRegistry`가 `name_by_key`/`key_by_name` 두 맵으로 키↔이름을 관리. `register`(키 유지·이름 갱신), `rename`(키 유지, 미등록 키는 false), `name_of`, `key_of`(역조회). 이름 갱신 시 옛 이름의 역조회 엔트리를 제거해 일관성 유지.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 이름 변경 후 numeric key 유지(key 5 불변, 새 이름 역조회, 옛 이름 None) | 통과 |
| 미등록 키 rename 실패 | 통과 |
| register 재등록 시 이름 갱신·키 유지 | 통과 |
| `cargo test -p stellar-core-types`(25 unit + 3 doctest) / fmt / clippy `-D warnings` | 통과 |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 없음.
