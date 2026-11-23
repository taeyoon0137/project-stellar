# 작업 보고서: XXX.1.1 `MotionSpec` record와 저장 형식

- WBS: XXX.1.1
- 작업명: MotionSpec record(StableId, owner, 대상 PropertyPath, 시작 조건, 지속 시간, 지연, timing function, reduced motion)와 검증·저장 형식.
- 상태: 완료 (`[x]`)
- 결정: ADR-0094 결정 1·5, ADR-0025 결정 1.2·2.3
- 선행: IX.5.1, VIII.6.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/motion/{mod,spec,set,world}.rs`(신규): record·검증, canonical binary 모음, 대상 해석·끝값 검사·진단.
- `crates/document/src/lib.rs`: `motion` module 등록.

## 구현 요약

- CSS `transition` 어휘(`transition-property`·`-duration`·`-delay`·`-timing-function`)로 저장합니다. 시작 조건은 상태 변수의 `becomes <값>`·`changes`입니다.
- timing function은 linear, cubic-bezier(4), steps(개수·jump 위치), spring(질량·강성·감쇠·초기 속도) 닫힌 집합이고 범위 밖 parameter를 거부합니다(ADR-0094 결정 1.5).
- 보간 가능 타입은 Number·Length·Percentage·Angle·Duration·Color입니다. 대상 포트 타입 집합에 보간 가능한 타입이 없으면 생성을 거부합니다(`check_targets`).
- `check_endpoints`가 끝값 타입 차이와 Color 색공간 차이를 거부합니다(premultiplied encoded 보간 자체는 XXX.3.2 evaluator).
- binary 형식(빈 모음 0 bytes, version 1)과 텍스트 원본(XXX.1.5 commit의 `prototype/source.rs`)이 round-trip합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 통과(lib 460개 포함 전체). |
| `cargo xtask validate-adr --base edfea9db` | 통과. |
| `cargo clippy -p stellar-document -p stellar-prototype --all-targets -- -D warnings` | 통과. |

시험:

- `motion::spec::tests::parameter_ranges_are_enforced`: duration·delay·bezier x·steps·spring 범위 위반 13개 fixture 거부.
- `motion::world::tests::color_endpoints_in_different_spaces_are_rejected`, `non_interpolable_and_mixed_endpoints_are_rejected`.
- `motion::set::tests::binary_round_trip_is_canonical`, `malformed_bytes_are_rejected`, `empty_set_is_zero_bytes`.
- `motion::command::tests::invalid_commands_leave_set_unchanged`: 보간할 수 없는 대상(`visible`) 생성 거부.

## 차이(작업 지시와 다른 점)

- 저장 형식은 별도 spec 문서 대신 ADR-0094 결정 5와 module 문서·golden으로 고정했습니다.
- View 소유자 MotionSpec의 시작 조건 변수는 View에 상태 머신이 없어 지금은 진단(`motion-missing-variable`)됩니다.

## 남은 위험 / 한계

- MotionSpec 모음을 프로젝트 열기·저장(document_io)에 연결하는 일은 텍스트 원본 codec 작업 몫입니다.
