# 작업 보고서: V.1.4 저장 float 정책(core 규칙 일치)

- WBS: V.1.4
- 작업명: 저장 float의 NaN·Infinity 거부, negative zero 정규화를 core 규칙과 일치
- 상태: 완료 (`[x]`)
- 선행: V.1.2, III.3.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/canonical/encode.rs` — `put_f32`/`put_f64`를 `Result`로 변경,
  `EncodeError::NonFiniteFloat` 추가, `put_finite_f32(FiniteF32)` 추가.
- `crates/storage/src/canonical/decode.rs` — `take_f32`/`take_f64`가 비유한·`-0.0` bits를
  `DecodeError::InvalidFloat`로 거부.
- `docs/fixtures/storage/canonical-primitives.yaml` — `-0.0` 정규화 golden 항목 2개 추가
  (`f32_neg_zero_normalized`, `f64_neg_zero_normalized`).
- `docs/task-reports/V.1.4.md`(신규), 계획서(V.1.4 `[ ]`→`[x]`).

## 구현 요약

- **encoder(R5)**: `put_f32`는 core의 `FiniteF32::new`(III.3.1)를 그대로 사용해 NaN·±Infinity를
  `NonFiniteFloat`로 거부하고 `-0.0`을 `+0.0`으로 정규화합니다. `put_f64`는 core에 f64 대응
  타입이 없어 같은 규칙을 storage에서 적용합니다. 거부 시 buffer에 bytes를 남기지 않습니다.
- **decoder**: canonical 입력은 유한 값만 담으므로 `take_f32`/`take_f64`가 NaN·±Infinity와
  `-0.0` bit 패턴을 `InvalidFloat`로 거부합니다(비canonical bytes 자동 수정 없음).
- **golden fixture**: `-0.0` 입력이 `+0.0` bytes로 기록되는 항목을 추가했고 encoder golden
  테스트가 이를 포함해 24개 primitive 전부와 byte-identical함을 확인합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 21 passed, 0 failed |
| `cargo test -p xtask` | 109 passed, 0 failed |
| `cargo run -p xtask -- validate-storage-fixtures` | exit 0 |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| primitive golden bytes 24개 일치(`golden_primitive_fixtures_match`) | 통과 |
| f32/f64 NaN·±Infinity encoder 거부(`non_finite_floats_are_rejected`) | 통과 |
| `-0.0` → `+0.0` 정규화(`negative_zero_is_normalized_to_positive_zero`) | 통과 |
| decoder가 NaN·±Infinity·`-0.0` bits 거부(malformed corpus 4항목 추가) | 통과 |

## 차이(작업 지시와 다른 점)

- 작업 지시는 encoder만 언급하지만, R5(유한 float bits)에 맞춰 decoder도 비유한·`-0.0`
  bits를 거부하게 했습니다. canonical 재기록 결정론(R6)과 일관됩니다.

## 남은 위험 / 한계

- core-types에 `FiniteF64`가 없어 f64 규칙은 storage에 지역 구현되어 있습니다. core에 f64
  타입이 생기면 그쪽으로 일원화해야 합니다.
