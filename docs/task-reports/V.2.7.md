# 작업 보고서: V.2.7 chunk kind registry와 unknown 정책

- WBS: V.2.7
- 작업명: chunk kind registry — unknown optional kind는 skip, unknown required kind는 오류
- 상태: 완료 (`[x]`)
- 선행: V.2.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/chunk/kind_registry.rs`(신규) — `ChunkKindRegistry`,
  `KindResolution`, `UnknownRequiredKind`, `KindRegistryError`,
  `ChunkKindCode`의 required bit API.
- `crates/storage/src/chunk/mod.rs` — `kind_registry` 모듈 등록.
- `docs/task-reports/V.2.7.md`(신규), 계획서(V.2.7 `[ ]`→`[x]`).

## 구현 요약

- **version 정책의 핵심**: requiredness를 registry가 아니라 **kind code의 최상위
  bit**(bit 31)에 인코딩합니다(PNG ancillary bit 원리). reader가 모르는 kind라도 code만
  보고 skip/오류를 판정할 수 있어, 신버전 writer가 추가한 kind에 대해 구버전 reader가
  올바르게 동작합니다.
- **`ChunkKindRegistry`**: code→이름 매핑. code·이름 중복 등록 거부. `resolve(code)`는
  등록된 kind → `Known`, 모르는 optional → `SkipUnknownOptional`, 모르는 required →
  `UnknownRequiredKind` 오류를 반환합니다.
- 초기 kind 목록(ProjectMetadata 등) 등록은 계획대로 V.6.1에서 수행합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 70 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| unknown optional fixture → skip(`unknown_optional_kind_is_skipped`) | 통과 |
| unknown required fixture → 오류(`unknown_required_kind_is_error`) | 통과 |
| 중복 code·이름 거부, required/optional bit 구분 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- required bit 규약은 kind code 할당 시점(V.6.1)부터 일관되게 지켜야 합니다. V.6.1
  등록 시 이 모듈의 `required`/`optional` 생성자만 사용해야 합니다.
