# 작업 보고서: VI.7.1 Text payload와 text index 정책

- WBS: VI.7.1
- 작업명: Text payload(UTF-8 content, paragraph list, style runs) — run 경계는 byte index가 아닌 정책 명시 text index
- 상태: 완료 (`[x]`)
- 선행: VI.2.2, III.3.4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/text.rs`(신규) — `TextIndex`, `TextRange`, `StyleRun`,
  `Paragraph`, `TextPayload`(`new`/`slice`/`encode`/`decode`), `scalar_len`,
  `byte_offset`, `TextError`.
- `crates/document/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VI.7.1.md`(신규), 계획서(VI.7.1 `[ ]`→`[x]`).

## 구현 요약

- **text index 정책**: run·paragraph 경계는 **Unicode scalar value 단위 offset**
  (`TextIndex`)입니다. 경계가 multi-byte 문자 중간에 놓일 수 없고, surrogate는 Rust
  `char`가 표현할 수 없어 존재가 불가능합니다. grapheme cluster 단위 커서·선택
  이동은 text layout(Phase X) 정책으로 두고 저장 경계는 scalar로 고정했습니다
  (std 전용 — segmentation 테이블 의존성 없음).
- **내용 보존**: 내용은 III.3.4 `UserText`로 정규화 없이 보존합니다(조합형 자모가
  완성형으로 바뀌지 않음).
- **paragraph**: `'\n'` 구분, 목록 길이 = `'\n'` 개수 + 1을 강제합니다. 속성은 VI.7.3,
  style 정의는 VI.7.2에서 채웁니다(현재는 표 index).
- **인코딩**: 길이 프리픽스 UTF-8 + paragraph style index + run(start, end, style).
  디코딩은 UTF-8(인코딩된 surrogate 포함 거부)·범위·paragraph 수·trailing을 검증합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 113 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 한글 완성형 + 조합형 자모 round-trip, 정규화 없음(`hangul_composition_round_trips_without_normalization`) | 통과 |
| emoji ZWJ 시퀀스(5 scalar) round-trip·slice(`emoji_zwj_sequence_round_trips`) | 통과 |
| 인코딩된 surrogate(ED A0 80) 거부(`encoded_surrogate_is_impossible`) | 통과 |
| scalar↔byte offset, 범위·paragraph 수·trailing 검증 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- run이 grapheme cluster 중간을 자를 수 있습니다(예: ZWJ 시퀀스 일부만 다른 style).
  저장은 허용하되, 편집 helper(VI.7.5)와 layout(Phase X)에서 grapheme 경계 정책을
  적용합니다.
