# 작업 보고서: VII.4.3 텍스트 입력 grouping

- WBS: VII.4.3
- 작업명: 동일 Text node, 인접 caret, 입력 종류, 짧은 manual-clock interval 조건으로 텍스트 입력 병합, interval 값은 config
- 상태: 완료 (`[x]`)
- 선행: VI.7.5, VII.4.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/history/src/grouping.rs`(신규) — `GroupingConfig`(텍스트 병합 간격, 기본 1초), 공통 `HistoryGroupingPolicy::should_merge`, 테스트.
- `crates/history/src/history.rs` — 정책·열린 묶음 상태, `with_policy`, `record_grouped`(병합 시 정방향 뒤 이어 붙임·역방향 앞에 붙임),
  일반 기록·preview 확정 시 묶음 닫기, 텍스트 fixture 테스트 3건.
- `crates/history/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VII.4.3.md`(신규), 계획서(VII.4.3 `[ ]`→`[x]`).

## 구현 요약

- **정책**: 도구별 예외는 공통 `HistoryGroupingPolicy`로만 정의합니다(§23.3). 텍스트 입력 병합 조건은 같은 Text node, 같은 입력 종류
  (`Insert`·`DeleteBackward`·`DeleteForward`), 인접 caret(새 입력의 편집 전 caret = 직전 입력의 편집 후 caret), 직전 입력 뒤 간격 이내
  (경계 포함). 시간은 주입한 `MonotonicClock` nanoseconds이며 역행하면 병합하지 않습니다.
- **config**: 간격은 `GroupingConfig::text_merge_interval_nanos`(기본 1초, 사용성 벤치마크 전 잠정값)이며 `History::with_policy`로 바꿉니다.
- **병합**: `record_grouped`는 실행 후 정책이 허락하면 마지막 항목에 합칩니다 — 정방향은 뒤에 이어 붙이고 역방향은 앞에 붙여 항목 하나의
  undo가 묶음 전체를 되돌리며 grouping key는 마지막 입력으로 갱신됩니다. 일반 기록·preview 확정이 끼면 묶음이 닫힙니다.
- 텍스트 편집 transaction은 VI.7.5 `edit_text`로 만든 payload를 레코드 복원 command(VII.2.7)로 적용합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-history` | 완료 |
| `cargo test -p stellar-history` | unit 10 passed, 0 failed |
| `cargo clippy -p stellar-history --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-history -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 정책 단위: 병합·간격 경계(=/+1ns)·다른 node·다른 종류·비인접·시간 역행·비텍스트 key·설정 간격(`text_merge_conditions`) | 통과 |
| "가b😀" 200ms 간격 연속 입력 → 항목 1건(정방향 3, key는 마지막 caret), 역 적용 시 빈 텍스트(`consecutive_typing_merges_into_one_entry_and_undoes_together`) | 통과 |
| 간격 초과·다른 node·비인접 caret·입력 종류 변경 → 분리, 연속 Backspace → 병합, 일반 기록 후 묶음 닫힘(`split_conditions`) | 통과 |
| config 50ms: 40ms 병합·60ms 분리(`interval_comes_from_config`) | 통과 |

## 차이(작업 지시와 다른 점)

- "config"는 저장소 `config/` 파일이 아니라 history crate의 `GroupingConfig` 값으로 두었습니다(허용 경로 밖 파일을 만들지 않음).
  앱 설정 파일 연결은 창 세션 통합(XII) 단계에서 합니다.
- 전용 텍스트 편집 command가 계획에 없어, fixture는 `edit_text` 결과를 `RestoreNodeRecords`로 적용합니다.

## 남은 위험 / 한계

- 기본 간격 1초는 잠정값입니다(벤치마크로 확정 필요).
