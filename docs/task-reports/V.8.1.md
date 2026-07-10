# 작업 보고서: V.8.1 버전 타입과 호환성 판정

- WBS: V.8.1
- 작업명: format_version, schema_version, minimum_reader_version, minimum_writer_version 타입과 호환성 판정
- 상태: 완료 (`[x]`)
- 선행: V.4.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/migration/mod.rs`(신규), `crates/storage/src/migration/compat.rs`
  (신규) — `FormatVersions`, `Compatibility`, `RejectReason`,
  `assess_compatibility`, `VersionDeclarationError`.
- `crates/storage/src/lib.rs` — `pub mod migration;` 등록.
- `docs/task-reports/V.8.1.md`(신규), 계획서(V.8.1 `[ ]`→`[x]`).

## 구현 요약

- **`FormatVersions`**: 문서(세대)가 선언하는 4개 버전. 선언 불변식
  `minimum_reader ≤ minimum_writer ≤ schema`를 검증하고 위반은 거부합니다.
- **판정 규칙**: format version은 정확 일치만 허용(불일치 거부). reader 스키마
  버전 `A`에 대해 `A ≥ min_writer` → 편집 가능, `min_reader ≤ A < min_writer`
  → 읽기 전용, `A < min_reader` → 거부(사유 포함).
- 판정 결과의 소비(마이그레이션 제안, 읽기 전용 UI)는 후속 계층 소관입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 182 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 읽기 전용·편집 가능·거부 matrix 6조합(`compatibility_matrix`) | 통과 |
| 경계값(쓰기 최소 정확 충족), 신버전 reader, 잘못된 선언 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- manifest(V.4.1)에는 현재 schema_version만 기록됩니다. minimum reader/writer
  version의 영속화는 migration step registry(V.8.2)와 함께 필요 시 format version
  상승으로 다룹니다.
