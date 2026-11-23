# 작업 보고서: XI.11.2 채널·structural 허용 오차 config와 diff artifact

- WBS: XI.11.2
- 작업명: image comparison에 per-channel/structural 허용 오차를 config로 정의하고 초과 pixel 위치를 diff artifact로 남김
- 상태: 완료 (`[x]`)
- 선행: XI.11.1 (`[x]`, 이 lane)
- 결정 기록: ADR-0102 결정 6

## 필수 실행 환경

- `DEV-MAC` (Apple Silicon GPU, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/effects/compare.rs`(신규) — `Tolerance`, `ToleranceConfig`(`parse`, `builtin`), `compare`, `Comparison`
  (`passed`, `summary`, `diff_image`, `report_text`, `write_artifact`), `PixelDiff`, 오류 타입.
- `crates/renderer-core/src/effects/tolerances.conf`(신규) — 기본 허용 오차 class(`default`, `curve`, `filtered`, `gradient`, `blur`).
- `crates/renderer-core/src/effects/mod.rs` — 모듈 등록.
- `crates/renderer-metal/src/tests/conformance.rs` — harness가 fixture class 허용 오차로 CPU 기준과 비교하고 실패 시 diff artifact를 씀.

## 구현 요약

- config는 줄 단위 `class channel=… radius=… max_failures=… max_shifted=…`입니다. 네 key가 모두 필요하고 모르는 key·중복 key·중복 class·
  `default` 누락은 줄 번호와 함께 오류입니다.
- 판정: pixel 채널 최대 차이가 `channel` 이하면 허용, 넘으면 structural 검사입니다. 반경 안에 실제 값과 맞는 기대 이웃이 있고 **동시에** 기대
  값과 맞는 실제 이웃이 있으면 위치 어긋남(shifted)입니다. 경계 밀림만 인정하고 고립 점·빠진 가는 선은 failure입니다. failure·shifted 수가
  각 상한 이하이면 통과입니다.
- diff artifact(`<dir>/<name>/`): `expected`·`actual`(raw·PNG), `diff.png`(shifted 노랑, failure 빨강, 나머지 어둡게), `failures.txt`(요약과
  pixel 위치·기대·실제·차이).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core --lib effects::compare` | 4 passed |
| `cargo test -p stellar-renderer-metal --lib tests::conformance` | 2 passed |
| `cargo clippy -p stellar-renderer-core -p stellar-renderer-metal --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| known equal: 같은 image, 1 LSB 차이(within), 경계 1 pixel 밀림(반경 1에서 shifted 8개, 위치 x=4)이 통과(`known_equal_fixtures_pass`) | 통과 |
| known different: 고립 점 1개가 (1, 6) 위치·값·차이 40으로 failure, 반경 0의 밀린 경계 failure 8개, shifted 상한 초과 실패, 크기 불일치 오류(`known_different_fixtures_fail_with_exact_locations`) | 통과 |
| diff image 색·`failures.txt` 줄·artifact 파일 생성(`diff_artifact_marks_failures_and_lists_positions`), config 해석·오류 8종(`config_parses_builtin_and_rejects_bad_lines`) | 통과 |
| GPU 출력을 바꾼 fixture 기준과 비교하면 failure 위치가 (6, 6)·(7, 6)이고 `failures.txt`·`diff.png`가 생기며, 원래 기준과는 통과(`gpu_image_against_altered_reference_fails_with_located_diff_artifact`) | 통과 |

## 차이(작업 지시와 다른 점)

- config 파일은 `config/performance-budgets.yaml`(ADR 보호 경로, lane 소유 밖)이 아니라 renderer-core 안 `effects/tolerances.conf`입니다.
  `include_str!`로 내장하고 `ToleranceConfig::parse`가 외부 text도 읽습니다.

## 남은 위험 / 한계

- class별 수치는 Apple Silicon 개발 장비 측정으로 정했습니다. D3D12 비교가 들어오면 class를 다시 맞춥니다.
