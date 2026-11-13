# 작업 보고서: VII.4.2 pointer gesture begin/update/end

- WBS: VII.4.2
- 작업명: pointer gesture begin/update/end API — update는 같은 preview transaction을 교체, end에서 한 entry 확정
- 상태: 완료 (`[x]`)
- 선행: VII.4.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/history/src/history.rs` — preview 상태, `begin_preview`/`begin_pointer_gesture`, `update_preview`, `end_preview`,
  `preview_active`, 테스트 2건.
- `crates/history/src/entry.rs` — `HistoryError::{PreviewActive, NoPreview}`.
- `docs/task-reports/VII.4.2.md`(신규), 계획서(VII.4.2 `[ ]`→`[x]`).

## 구현 요약

- **begin**(pointer down): preview를 엽니다. 문서·history는 그대로입니다.
- **update**(pointer move): 직전 preview의 역(현재 revision으로 rebase)과 새 transaction을 **한 원자 transaction**으로 실행해 preview를
  교체합니다. 문서에는 항상 마지막 preview만 반영되고 history에는 쌓이지 않습니다. 결합 역 transaction의 앞쪽 n개(새 transaction 길이)가
  gesture 시작 전 상태로의 역입니다. 실패하면 문서와 이전 preview가 그대로 남습니다.
- **end**(pointer up): 마지막 preview를 grouping key(`PointerGesture`)와 함께 **한 항목**으로 확정합니다. update가 없었으면 `None`.
- preview 중에는 일반 기록·다른 preview 시작을 `PreviewActive`로 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-history` | 완료 |
| `cargo test -p stellar-history` | unit 6 passed, 0 failed |
| `cargo clippy -p stellar-history --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-history -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| transform drag 500 update → update 중 항목 0, end 후 **항목 1건**·sequence 1·grouping key, 문서에 마지막 값, 항목 역 적용 시 gesture 전 snapshot(`drag_with_500_updates_records_one_entry`) | 통과 |
| 매 update가 다른 node를 넣어도 문서에는 마지막 하나만(누적 없음), 실패 update는 이전 preview 유지, preview 중 기록·새 gesture 거부, update 없는 gesture 미기록, 비활성 end 거부(`update_replaces_previous_preview_instead_of_accumulating`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- update마다 문서 revision이 하나 오릅니다. drag 중 revision 증가 정책(정의된 preview revision)은 XII.3.7에서 정합니다.
