# 작업 보고서: VI.1.1 문서·페이지·View 정의

- WBS: VI.1.1
- 작업명: `ProjectDocumentIndex`, `DocumentDefinition`, `PageDefinition`, `ViewDefinition` 타입
- 상태: 완료 (`[x]`)
- 선행: Phase V, III.1.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/definitions.rs`(신규) — 4개 정의 타입, 생성·삭제 API,
  binary encode/decode, `DefinitionError`.
- `crates/document/src/lib.rs` — `definitions` 모듈 등록.
- `docs/task-reports/VI.1.1.md`(신규), 계획서(VI.1.1 `[ ]`→`[x]`).

## 구현 요약

- **소유 계층**: `ProjectDocumentIndex` → `DocumentDefinition` →
  `PageDefinition` → `ViewDefinition`. 각 타입은 고유 `StableId`와 표시 이름을
  갖고, 표시 이름은 비어 있을 수 있으며 식별에 사용하지 않습니다(VI.1.3 예고).
- **생성·삭제**: 각 레벨의 add/remove가 중복 id를 거부하고 미존재 삭제를
  오류로 반환합니다.
- **binary 인코딩**: canonical 규칙(V.1 R1~R3: LE 고정 폭, `u32` 길이 프리픽스
  UTF-8)을 따르는 encode/decode. crate 그래프상 document는 storage에 의존할 수
  없어 이 모듈이 직접 구현하며, 절단·trailing bytes·비UTF-8을 panic 없이
  거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 16 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 생성·삭제(중복 거부·미존재 오류 포함)(`create_and_remove_at_each_level`) | 통과 |
| binary round-trip + 재인코딩 byte-identical(`binary_round_trip`) | 통과 |
| 빈 index, 한글·emoji 이름, malformed 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- Phase V 완료 조건 중 V.6.4·V.6.5(chunk 크기 임계치)는 ADR 승인 대기로 남아
  있으나, 문서 정의 타입과는 독립적이라 착수했습니다.

## 남은 위험 / 한계

- Document→Page→View root Frame 제약(VI.1.2)과 이름/저장 경로 분리 API(VI.1.3)가
  후속입니다.
