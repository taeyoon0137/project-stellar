# 작업 보고서: V.2.3 chunk object 경로 생성

- WBS: V.2.3
- 작업명: content hash → `design/objects/<prefix>/<hash>.starchunk` 경로(타입으로 traversal 차단)
- 상태: 완료 (`[x]`)
- 선행: V.1.5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/chunk/path.rs`(신규) — `ChunkObjectPath`.
- `crates/storage/src/chunk/mod.rs` — `path` 모듈 등록.
- `docs/task-reports/V.2.3.md`(신규), 계획서(V.2.3 `[ ]`→`[x]`).

## 구현 요약

- **`ChunkObjectPath`**: [`ContentHash`]에서만 생성 가능한 상대 경로 타입입니다. 임의
  문자열 입력 경로가 없어 path traversal이 타입상 불가능합니다.
- **형식**: `design/objects/<prefix>/<algorithm>-<hex64>.starchunk`. prefix는 digest
  소문자 hex 앞 2자(shard). 파일명의 알고리즘 구분자는 ADR-0004 문자열의 `:` 대신
  `-`를 사용합니다 — `:`는 Windows 파일명 금지 문자입니다.
- **API**: `to_relative_path()`(플랫폼 구분자 `PathBuf`), `Display`(`/` 정규 표기),
  `prefix()`, `file_name()`.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 48 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 1,000개 hash 전부 고정 길이·안전 문자 집합·traversal 없음 | 통과 |
| golden 경로(sha256 "test" vector) | 통과 |
| Windows 금지 문자 부재 | 통과 |

## 차이(작업 지시와 다른 점)

- 계획 표기 `<hash>`를 `<algorithm>-<hex64>`로 구체화했습니다. ADR-0004 문자열 그대로는
  `:` 때문에 Windows에서 사용할 수 없습니다.

## 남은 위험 / 한계

- prefix 2자는 256개 shard입니다. 대규모 프로젝트에서 디렉터리 항목 수가 문제되면
  format version 상승 없이 경로 정책만으로는 바꿀 수 없습니다(기존 경로와의 호환 필요).
