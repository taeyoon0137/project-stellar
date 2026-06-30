# 저장 primitive canonical 인코딩 규칙 (V.1.1)

결정론적 청크 저장의 기준이 되는 primitive 인코딩 규칙입니다. 구현(V.1.2 encoder, V.1.3
decoder)은 본 규칙과 `docs/fixtures/storage/canonical-primitives.yaml`의 golden bytes를 1차
기준으로 따릅니다. 본 문서는 **구현 이전 명세**입니다.

## 규칙

### R1. Little-endian

모든 다중 바이트 정수·float은 **little-endian**으로 기록합니다.

### R2. 고정 정수 폭(fixed integer width)

정수는 타입별 **고정 폭**으로 기록합니다(가변 길이 varint 아님):

| 타입 | 폭(byte) | 표현 |
|---|---|---|
| `u8`/`i8` | 1 | LE |
| `u16`/`i16` | 2 | LE |
| `u32`/`i32` | 4 | LE(부호는 2의 보수) |
| `u64`/`i64` | 8 | LE(부호는 2의 보수) |

`bool`은 1바이트(`0x00`=false, `0x01`=true)로 기록합니다.

### R3. 길이 프리픽스 UTF-8(length-prefixed UTF-8)

문자열·바이트열은 **`u32` LE 길이 프리픽스 + payload**로 기록합니다. 문자열 payload는 UTF-8
입니다(정규화는 상위 계층 책임, III.3.4).

### R4. 정렬된 map key(sorted map key)

map은 **key의 canonical byte 순서**로 정렬해 기록합니다. 입력 삽입 순서와 무관하게 동일
bytes를 만듭니다(V.1.2).

### R5. 유한 float bits(finite float bits)

float은 IEEE-754 비트 패턴을 LE로 기록하되 **유한 값만** 허용합니다(III.3.1). NaN·±Infinity는
거부하고 `-0.0`은 `+0.0`으로 정규화합니다(V.1.4).

### R6. unknown field 정책

- **mandatory(필수)로 표시된 unknown tag**는 디코드 시 거부합니다(V.1.3).
- optional unknown field는 정책에 따라 보존 또는 무시하되, canonical 재기록 시 결정론을 깨지
  않아야 합니다.

## golden bytes fixture

`docs/fixtures/storage/canonical-primitives.yaml`에 각 primitive의 (type, value, expected hex
bytes)를 정의합니다. encoder(V.1.2)·float 정책(V.1.4)은 이 golden bytes와 일치해야 합니다.
fixture 스키마는 `cargo xtask validate-storage-fixtures`로 검사합니다.

스키마(각 항목):

- `name`: 식별자(문자열).
- `type`: `u8`/`u16`/`u32`/`u64`/`i8`/`i16`/`i32`/`i64`/`bool`/`f32`/`f64`/`string` 중 하나.
- `value`: 사람이 읽는 값(문자열로 표기).
- `bytes`: 기대 바이트의 소문자 16진수(공백 없음, 짝수 길이).
