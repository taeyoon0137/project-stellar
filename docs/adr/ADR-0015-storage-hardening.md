# ADR-0015: 저장 decoder 경화(fuzz 발견 수정)

## Status

Accepted (2026-11-14, 구현 기록)

## Context

기술 아키텍처 §37.2(파서)는 신뢰되지 않은 입력에 대해 "청크 길이·참조·해시 검증", "깊이·개수
제한", "Fuzzing 필수"를 요구하고, §38.2는 "YAML·binary parser fuzzing"을 필수 테스트 층으로
둡니다. 구현 계획 XVI.6.2는 chunk envelope/canonical decoder fuzz target을 추가하고 "malformed
input panic 0"을 검증 기준으로 둡니다.

`crates/storage/`는 ADR 보호 경로(I.3.5)입니다. fuzz가 찾은 결함을 고치는 변경도 저장 포맷
parser의 동작을 바꾸므로 이 ADR에 기록합니다.

발견 기록:

| 날짜 | WBS | 발견 | 위치 | 수정 |
|---|---|---|---|---|
| 2026-11-14 | XVI.6.2 | `ContentHash` 문자열 parse가 다중 byte UTF-8 문자에서 panic | `crates/storage/src/hash/mod.rs` | slice 전 문자 검사 |

XVI.6.2 발견의 원인: parse는 hex 부분의 **byte 길이**(64)만 확인한 뒤 2 byte 단위로 slice합니다.
hex 자리에 2·3·4 byte 문자가 있으면 byte 길이는 맞아도 slice 경계가 문자 안쪽에 떨어져 panic합니다
(예: `sha256:` + `€` + `a`×61, 총 71 byte). 이 parse는 신뢰되지 않은 입력에서 호출됩니다.

- chunk envelope의 payload hash(V.2.2, chunk 읽기 V.2.6)
- chunk pointer·generation manifest의 content hash(V.3.1, V.4.1)
- `design/current` 파일 내용(V.4.3)
- fallback scanner가 해석하는 `design/generations/` 파일 이름(V.4.5)

release profile은 `panic = "abort"`(II.1.3)이므로 손상·조작된 chunk 하나, 또는 파일 이름 하나로
프로젝트 열기 경로가 프로세스를 끝낼 수 있었습니다.

## Decision

1. `ContentHash::from_str`은 slice 전에 hex 부분의 모든 문자를 앞에서부터 검사하고, 소문자 ASCII
   16진수가 아닌 첫 문자를 `ParseContentHashError::InvalidHexDigit { found }`로 보고합니다. 검사를
   통과한 hex는 ASCII뿐이므로 2 byte slice는 항상 문자 경계입니다.
2. ADR-0004의 문자열 형식(`sha256:<64자리 소문자 16진수>`), 오류 종류, 수용 집합은 바꾸지 않습니다.
3. 회귀 방지는 결정론적 fuzz harness `crates/storage/tests/chunk_canonical_fuzz.rs`(XVI.6.1과 같은
   방식: 고정 seed, 새 의존성·nightly 없음)와 seed corpus `crates/storage/tests/fuzz_corpus/
   chunk_canonical/`로 고정합니다. 발견 입력은 corpus에 회귀 seed로 남깁니다.
4. 이후 저장 decoder의 fuzz 발견 수정도 위 발견 기록 표에 추가합니다.

## Alternatives

- **byte 단위 직접 변환**(`as_bytes()` + nibble 계산): 결과는 같지만 오류에 담을 `char`를 따로
  복원해야 해 변경이 커집니다 — 미선택.
- **오류를 삼키는 `catch_unwind` 감싸기**: release가 `panic = "abort"`라 동작하지 않고, 결함을
  숨깁니다 — 미선택.
- **cargo-fuzz/libFuzzer**: 새 의존성과 nightly toolchain이 필요합니다(XVI.6.1과 같은 이유) — 미선택.

## Consequences

- 신뢰되지 않은 hash 문자열은 panic 대신 오류가 됩니다: envelope·pointer·manifest는 `InvalidHash`,
  current pointer는 `InvalidFormat`, fallback scanner는 그 파일을 후보에서 제외합니다.
- 이전에 수용하던 입력 중 거부로 바뀐 것은 없습니다. 이전에 panic하던 입력만 오류로 바뀝니다.
  ASCII 입력은 이전과 같은 첫 위반 문자를 보고합니다(이전에도 문자열 순서로 검사).
- hex 64자를 한 번 더 순회합니다.

## Compatibility

- 파일 포맷·문자열 형식·공개 API·FFI 변경 없음.

## Performance

- 성능 예산 영향 없음(hash parse당 64자 추가 순회 1회).

## Security

- §37.2 파서 경계: 신뢰되지 않은 프로젝트의 chunk·manifest·current pointer·파일 이름이 앱을
  종료시키는 경로(서비스 거부)를 제거합니다.

## Migration

- 데이터 마이그레이션 없음.
