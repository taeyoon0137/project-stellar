# 작업 보고서: VI.7.5 결정론적 텍스트 편집 helper

- WBS: VI.7.5
- 작업명: 텍스트 내용 변경 시 style run range를 보존·축소·확장하는 deterministic edit helper
- 상태: 완료 (`[x]`)
- 선행: VI.7.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/text.rs` — `edit_text`.
- `docs/task-reports/VI.7.5.md`(신규), 계획서(VI.7.5 `[ ]`→`[x]`).

## 구현 요약

- **단일 연산**: `edit_text(payload, range, replacement, styles)`가 삽입(빈 범위)·삭제
  (빈 replacement)·replace를 모두 처리하고 새 payload를 반환합니다(입력 불변).
- **run 규칙**: 삭제 구간과 겹치는 run은 축소, 비면 제거, 뒤 run은 당김. 삽입 텍스트는
  replace면 첫 교체 문자의 run style, 순수 삽입이면 바로 왼쪽 문자의 run style(위치 0은
  첫 문자 run)을 상속해 그 run이 확장됩니다. 상속할 run이 없으면 기본 style. 결과는
  VI.7.2 canonical 형식으로 정규화합니다.
- **paragraph 규칙**: 삭제된 `'\n'`마다 다음 paragraph가 앞 paragraph에 합쳐지고(앞
  style 유지), 삽입된 `'\n'`마다 삽입 위치 paragraph style을 복제합니다.
- index는 VI.7.1 scalar 단위이며, 결과는 `TextPayload::new` 검증을 다시 통과합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 124 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 삽입 fixture 4종(run 중간·경계·위치 0·run 없는 문자 뒤)(`insert_fixtures`) | 통과 |
| 삭제 fixture 3종(걸친 축소·run 제거·전체 삭제→빈 node)(`delete_fixtures`) | 통과 |
| replace fixture 2종(한글 교체·경계 가로지름)(`replace_fixtures`) | 통과 |
| 줄바꿈 삽입·삭제의 paragraph 갱신, 결정론·범위 검증 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- grapheme 경계(예: ZWJ 시퀀스 중간) 편집 방지는 커서 이동 정책(Phase X)이 담당합니다.
  이 helper는 주어진 scalar 범위를 그대로 따릅니다.
