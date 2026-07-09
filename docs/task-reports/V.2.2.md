# 작업 보고서: V.2.2 chunk envelope

- WBS: V.2.2
- 작업명: chunk envelope(magic, format version, chunk kind, uncompressed length, payload hash) 정의 — 압축 미사용
- 상태: 완료 (`[x]`)
- 선행: V.1.5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/chunk/envelope.rs`(신규) — `ChunkEnvelope`, `ChunkKindCode`,
  `EnvelopeError`, `CHUNK_MAGIC`, `CHUNK_FORMAT_VERSION`.
- `crates/storage/src/canonical/encode.rs` — 고정 폭 raw 기록 `put_raw_magic` 추가.
- `crates/storage/src/canonical/decode.rs` — 고정 폭 raw 읽기 `take_raw` 추가.
- `crates/storage/src/chunk/mod.rs` — `envelope` 모듈 등록.
- `docs/task-reports/V.2.2.md`(신규), 계획서(V.2.2 `[ ]`→`[x]`).

## 구현 요약

- **header 형식(format version 1)**: magic `STARCHNK`(8바이트 raw) + format version
  `u32` LE(=1) + chunk kind `u32` LE + uncompressed length `u64` LE + payload hash
  (ADR-0004 문자열 `sha256:<hex64>`, 길이 프리픽스 UTF-8). 압축 필드는 없으며 압축
  도입 시 format version을 올립니다.
- **`ChunkKindCode(u32)`**: kind 숫자 code newtype. 이름·optional/required 정책 매핑은
  kind registry(V.2.7) 소관입니다.
- **검증**: magic 불일치(`BadMagic`), 미지원 version(`UnsupportedVersion`), hash 형식
  위반(`InvalidHash`), canonical 절단·overflow(`Decode`)를 각각 구분된 오류로 거부합니다.
  payload 길이·hash 일치 검증은 chunk read(V.2.6) 소관입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 44 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| golden header bytes 고정(`golden_header_bytes_are_fixed`, python 검산 값과 일치) | 통과 |
| round-trip, bad magic·미지원 version·hash 손상·절단 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- payload hash를 binary tag가 아닌 ADR-0004 문자열로 저장해 header가 75바이트 커집니다.
  자기 기술성과 단일 표현을 우선했습니다. 크기가 문제되면 format version 상승과 함께
  binary tag로 교체할 수 있습니다.
