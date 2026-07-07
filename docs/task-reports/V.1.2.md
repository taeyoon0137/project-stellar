# 작업 보고서: V.1.2 canonical encoder

- WBS: V.1.2
- 작업명: `storage` canonical encoder — map 입력 순서와 무관하게 key order로 기록
- 상태: 완료 (`[x]`)
- 선행: V.1.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/canonical/mod.rs`(신규) — canonical 인코딩 모듈 선언.
- `crates/storage/src/canonical/encode.rs`(신규) — `CanonicalEncoder`, `EncodeError`.
- `crates/storage/src/lib.rs` — `pub mod canonical;` 등록.
- `docs/task-reports/V.1.2.md`(신규), 계획서(V.1.2 `[ ]`→`[x]`).

## 구현 요약

- **`CanonicalEncoder`**: R1(LE)·R2(고정 정수 폭, bool 1바이트)·R3(u32 LE 길이 프리픽스
  UTF-8/바이트열)·R4(정렬된 map key)를 구현했습니다. 외부 의존성 없이 `Vec<u8>` buffer에
  기록합니다.
- **map 인코딩 형식**: `u32` LE entry 수 + key byte 순서로 정렬된 entry(길이 프리픽스 key +
  길이 프리픽스 value bytes). 입력 삽입 순서는 결과 bytes에 영향을 주지 않고, 중복 key는
  `EncodeError::DuplicateKey`로 거부합니다.
- **`EncodeError`**: `LengthOverflow`(길이 프리픽스 u32 초과), `DuplicateKey`. 길이 검사는
  `check_len`으로 분리해 4GiB 할당 없이 단위 테스트합니다.
- **float**: V.1.2 시점에는 IEEE-754 비트 패턴을 LE로 그대로 기록합니다. R5(NaN·±Infinity
  거부, `-0.0` 정규화)는 계획대로 V.1.4에서 적용합니다.
- **golden fixture 연동**: 테스트가 `docs/fixtures/storage/canonical-primitives.yaml`(V.1.1)을
  직접 읽어 22개 primitive 전부를 encoder 출력과 비교합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 9 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| golden primitive fixture 22개 byte-identical(`golden_primitive_fixtures_match`) | 통과 |
| 삽입 순서를 바꾼 1,000 순열이 동일 bytes(`thousand_shuffled_insert_orders_encode_identical_bytes`) | 통과 |
| 중복 key 거부, 길이 overflow 거부, map 정렬·빈 map | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 완료 기준(삽입 순서를 바꾼 1,000 fixture가 동일 bytes)을 충족합니다.

## 남은 위험 / 한계

- `put_f32`/`put_f64`는 아직 비유한 값을 거부하지 않습니다. V.1.4에서 core 규칙(III.3.1)과
  일치시킵니다.
- map value는 호출자가 미리 canonical 인코딩한 bytes를 받습니다. 상위 스키마(레코드 tag,
  unknown field 정책)는 V.1.3 decoder와 함께 확정합니다.
