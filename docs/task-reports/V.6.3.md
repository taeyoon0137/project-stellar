# 작업 보고서: V.6.3 chunk 목표 크기 측정 harness

- WBS: V.6.3
- 작업명: 64KiB/256KiB/1MiB 후보의 로드·저장·Git delta 측정 harness(결론 자동 선택 없음)
- 상태: 완료 (`[x]`)
- 선행: V.5.6

## 필수 실행 환경

- `DEV-MAC` (Rust + 시스템 Git, 로컬 실행).

## 변경 파일 목록

- `tools/xtask/src/chunk_bench.rs`(신규) — 측정 harness와 보고서 렌더러.
- `tools/xtask/src/commands.rs` — `bench-chunk-size` 명령 등록(COMMANDS 16개),
  `direct_dependencies`가 내부 workspace crate(path 의존)를 승인 목록 대상에서
  제외하도록 수정.
- `tools/xtask/src/lib.rs`, `tools/xtask/Cargo.toml` — 모듈 등록, 내부 crate 의존
  추가(`stellar-storage`, `stellar-core-types`).
- `Cargo.lock` — workspace 의존 갱신.
- `docs/status/chunk-size-benchmark.md`(신규, 생성물) — raw benchmark + ADR 입력
  보고서.
- `docs/task-reports/V.6.3.md`(신규), 계획서(V.6.3 `[ ]`→`[x]`).

## 구현 요약

- **harness(`cargo xtask bench-chunk-size`)**: 후보별로 임시 Git repo를 만들고
  결정적 pseudo-random 8MiB 데이터셋을 해당 크기로 잘라 실제 저장 파이프라인
  (V.5 `run_save`)으로 generation 1을 저장·commit → 전체 검증 로드 측정 →
  중앙 chunk 16바이트 수정 후 generation 2 저장·commit → `.git/objects` 증가량을
  Git delta로 기록합니다.
- **보고서**: raw 수치 표 + 해석 지침만 담고 **결론을 포함하지 않습니다**(임계치
  확정은 ADR·V.6.4). 파일 상단에 생성물 표기를 넣었습니다.
- **1회 실행 결과(참고)**: 저장 2434.7/1324.1/1103.2ms, 로드 ~335ms(동률), 단일
  수정 Git delta 74,679/265,343/1,050,389 bytes (64KiB/256KiB/1MiB).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo run -p xtask -- bench-chunk-size` | exit 0, 보고서 생성 |
| `cargo test -p xtask` | 112 passed, 0 failed |
| `cargo fmt -p xtask -- --check` | 통과 |
| `cargo clippy -p xtask --all-targets -- -D warnings` | 경고 0 |
| `cargo xtask validate-dependencies` / `validate-versions` / `validate-crate-graph` / `sync-plan --check` | 전부 통과 |

## 차이(작업 지시와 다른 점)

- xtask가 내부 crate(`stellar-storage`)에 의존하게 되면서 I.3.2 dependency
  validator가 내부 path 의존을 unknown으로 오탐해, 검사 범위를 외부 dependency로
  한정했습니다(정책 목적인 외부 공급망 통제와 일치). crate 그래프 정책은 제품
  crate(`crates/`)만 대상이므로 영향 없습니다.

## 남은 위험 / 한계

- 측정은 DEV-MAC 1회 실행이고 데이터셋이 비압축성 합성 데이터입니다. 실제 문서
  분포와 다를 수 있어 임계치 확정(V.6.4) 전 추가 측정이 필요할 수 있습니다.
- Git delta는 loose object 크기 기준입니다(pack 전).
