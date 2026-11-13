# 작업 보고서: VI.6.2 subpath·winding rule

- WBS: VI.6.2
- 작업명: subpath와 winding rule 정의, 시작 MoveTo 없는 command 거부
- 상태: 완료 (`[x]`)
- 선행: VI.6.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/path.rs` — `WindingRule`(NonZero/EvenOdd), `PathData`
  (`new`/`subpaths`/`encode`/`decode`), `Subpath`, `validate_commands`,
  `PathError::{MissingMoveTo, UnknownWinding}`.
- `docs/task-reports/VI.6.2.md`(신규), 계획서(VI.6.2 `[ ]`→`[x]`).

## 구현 요약

- **canonical subpath 규칙**: 비어 있지 않은 path의 첫 command와 Close 다음 command는
  반드시 MoveTo입니다(암묵적 subpath 재시작 불허). 위반은 문제 command index와 함께
  `MissingMoveTo`로 거부합니다. 빈 path는 유효합니다.
- **winding**: NonZero(기본, tag 0)·EvenOdd(tag 1). `PathData` 인코딩은 winding tag +
  VI.6.1 command 인코딩이며, 디코딩도 같은 규칙으로 검증합니다.
- **subpath 분할**: `subpaths()`가 MoveTo 단위로 나누고 닫힘 여부를 보존합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 100 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| malformed path fixture 5종 실패(정확한 index)(`malformed_path_fixtures_fail`) | 통과 |
| subpath 분할·닫힘, winding round-trip, 빈 path, 알 수 없는 winding | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- SVG import(로드맵)는 Close 뒤 암묵적 재시작을 명시 MoveTo로 정규화해야 합니다.
