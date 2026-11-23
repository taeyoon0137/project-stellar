# 작업 보고서: IX.11.29 CLI `stellar style-defaults list|show|set`

- WBS: IX.11.29
- 작업명: CLI `stellar style-defaults`의 `list`(내장·사용자 세트 목록), `show`(평탄화한 세트 표), `set`(활성 세트 변경)
- 상태: 부분 완료 (`[~]`) — 2026-11-24 통합 판정: 이 행의 구현은 끝났지만 선행 IX.11.26, IX.11.27, XXIV.4.5이 `[x]`가 아닙니다. 그 조건이 풀리면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]`) — 완료 기준 세 가지를 확인했습니다. `set`이 `ops::apply`를 거치지 않는 차이와 그 이유는 아래
  "차이"에 있습니다. 선행 IX.11.26·IX.11.27·XXIV.4.5의 표기는 통합 때 선행 판정을 따릅니다.
- 결정: ADR-0115 결정 4(신규), ADR-0087 결정 2, ADR-0086 결정 11
- 선행: IX.11.26, IX.11.27, XXIV.4.5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `tools/stellar-cli/src/style_defaults.rs`
  - `set <경로> <세트> [--base <revision>] [--dry-run]`: 교차 프로세스 저장 잠금 → 다시 열기 → base revision 검사 →
    `prepare_selection(root, 연 시점 이름, 세트)` → `commit`. 결과에 `mode`·`base_revision`·`revision`을 더했습니다.
  - `show`: layout 표(`values`)에 더해 payload longhand 표(`payload`, registry 항목)를 냅니다.
  - `resolve`: `ops::query::node_layout` 결과를 그대로 냅니다(CLI 자체 해석 제거).
- `tools/stellar-cli/src/cli.rs`, `tools/stellar-cli/tests/fixtures/help.txt`: 도움말 한 줄.
- `tools/stellar-cli/tests/style_defaults.rs`: 통합 test 2개 추가, 사용법 오류 test 보강.
- `docs/spec/style-defaults.md` 3장·5장: `set` 옵션과 `resolve`·`query node`가 같은 결과라는 설명.

## 구현 요약

- `list`·`show`는 기존 경로이고, `show`에 payload 표가 더해졌습니다.
- `set`은 ops 쓰기 경로와 같은 순서(잠금을 기다리지 않고 잡음 → 다시 열기 → base revision 비교)를 지킵니다. base가
  다르면 `base-revision-mismatch`(exit 65)이고 아무 파일도 쓰지 않습니다. `--base`가 없으면 지금 revision을 씁니다.
- 쓰기는 ADR-0087 결정 2의 두 단계 쓰기입니다. 준비가 세트 평탄화(없는 세트 `unknown-style-defaults`, 순환
  `style-defaults-cycle` 등)와 연 뒤 선택 변경(`style-defaults-stale`)을 거부하고, commit은 읽은 bytes hash가 그대로일
  때만 씁니다. 바꿀 것이 없으면 쓰지 않습니다.
- 문서 레코드는 바뀌지 않으므로 generation과 revision은 그대로입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-cli --test style_defaults` | 통과(5). 목록·표(payload 포함)·`set` 뒤 다시 열기, base 불일치 거부와 파일 불변, dry-run 불변, 바뀐 파일이 `stellar.yaml` 하나·한 줄, 되돌리면 원래 bytes, `resolve`와 `query node`의 `layout.values` 동일 |
| `cargo test -p stellar-cli --test headless_help` | 통과(6). 도움말 golden |
| `cargo test -p stellar-cli --test layout_provenance` | 통과(1) |

## 완료 기준 대비

| 기준 | 상태 |
|---|---|
| CLI 통합 test(목록·표·`set` 뒤 재열기) | `list_and_show_include_user_sets_and_reject_cycles`, `show_lists_payload_longhands_from_the_registry_table`, `set_goes_through_the_ops_write_path_and_changes_one_manifest_line` |
| base revision 불일치 거부 | 같은 test: `--base g0`이면 exit 65 `base-revision-mismatch`, 프로젝트 파일 전부 그대로 |
| 도움말 golden | `headless_help` |

## 차이(작업 지시와 다른 점)

- 행 설명은 "`set`(`SetStyleDefaults` 적용)은 ops 쓰기 경로를 거친다"입니다. `set`은 `ops::apply`에 canonical
  `SetStyleDefaults`를 넣지 않고, 그 command의 영속 결과(`stellar.yaml` 한 줄)를 같은 두 단계 쓰기 API로 씁니다.
  - 이유: `ops::apply`는 문서 레코드가 그대로인 transaction에도 generation을 새로 씁니다. 직접 확인한 결과
    `design/current`와 새 `design/generations/*.starmanifest`가 바뀌어 Git diff가 세 파일이 됩니다. ADR-0082 결정 4의
    "Git diff에서 한 줄"에 어긋납니다.
  - `ops::apply`는 이 작업 소유 밖이라 고치지 않았습니다. mirror만 바뀐 transaction의 generation 저장을 건너뛰게 고치면
    `set`을 `ops::apply` 하나로 바꿀 수 있고 결과 bytes는 같습니다(ADR-0115 결정 4).
- base revision은 generation revision입니다. `stellar.yaml`만 바뀌는 `set` 뒤에도 revision은 그대로이고, 동시에 다른
  writer가 선택을 바꾼 경우는 준비 단계의 연 시점 이름 확인과 commit의 hash 확인이 막습니다.

## 남은 위험 / 한계

- 앱·`stellar apply`로 `SetStyleDefaults`를 적용하면 여전히 generation 파일 두 개가 함께 바뀝니다(`ops::apply` 소유 작업).
- `show`의 payload 표는 내장 세트 값입니다. 사용자 세트는 payload longhand를 두지 않습니다(ADR-0115 결정 1).
