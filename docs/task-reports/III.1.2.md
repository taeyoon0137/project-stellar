# 작업 보고서: III.1.2 결정적 StableIdGenerator

- WBS: III.1.2
- 작업명: 테스트용 결정적 `StableIdGenerator`(seed + 수동 시각 동일 → 동일 ID 시퀀스)
- 상태: 완료 (`[x]`)
- 선행: III.1.1, II.3.3 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/core-types/src/ids.rs` (`StableIdGenerator`, 내부 `SplitMix64` PRNG, golden/결정성 테스트)
- `docs/task-reports/III.1.2.md`, 계획서(III.1.2 `[ ]`→`[x]`)

## 구현 요약

- `StableIdGenerator::new(seed)` + `next_id(&clock)`. timestamp는 주입된 `MonotonicClock`(테스트는 `ManualClock`)의 `now_nanos()/1_000_000`(ms)에서, random 필드(rand_a 12비트, rand_b 62비트)는 seed 기반 std 전용 PRNG(SplitMix64)에서 가져와 `StableId::new_v7`로 조립.
- 외부 의존성 없이 결정적: 같은 seed와 같은 수동 시각 수열이면 항상 동일한 ID 시퀀스.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| golden sequence(seed 42, 5개, 고정 문자열 일치) | 통과 |
| 같은 seed+시각 → 동일 시퀀스 | 통과 |
| 다른 seed → 다른 시퀀스 | 통과 |
| 생성 ID가 v7·variant 0b10·시간 순(first < second) | 통과 |
| `cargo test -p stellar-core-types`(17 unit + 2 doctest) / fmt / clippy `-D warnings` | 통과 |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 없음. random 소스는 승인 의존성 정책(core-types std 전용)에 맞춰 내부 SplitMix64로 구현했습니다.
