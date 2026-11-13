# 작업 보고서: VI.3.2 containing block capability validator

- WBS: VI.3.2
- 작업명: Frame만 자식의 layout containing block과 좌표계를 만들 수 있게 하는 capability validator
- 상태: 완료 (`[x]`)
- 선행: VI.3.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/capabilities.rs`(신규) — `can_be_containing_block`,
  `validate_containing_block_kind`, `resolve_containing_block`, `CapabilityError`.
- `crates/document/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VI.3.2.md`(신규), 계획서(VI.3.2 `[ ]`→`[x]`).

## 구현 요약

- **capability 규칙**: containing block(좌표계 생성)은 `NodeKind::Frame`만
  가능합니다. Fragment·Shape·Path·Text·Image·ComponentInstance를 containing block
  으로 지정하면 `NotAContainingBlock { kind }`로 거부합니다.
- **해석**: `resolve_containing_block`은 parent chain을 올라가며 **가장 가까운
  Frame 조상**을 반환합니다. Fragment처럼 좌표계를 만들지 않는 조상은 건너뜁니다
  (VI.4.1의 zero-size marker 정책과 일관). Frame 조상이 없으면 `None`이며,
  root가 Frame인지의 강제는 전체 document validator(VI.10.1) 소관입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 57 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| Fragment 등 비Frame 종류를 containing block으로 사용 시 오류(`only_frame_can_be_containing_block`) | 통과 |
| Fragment를 건너뛴 가장 가까운 Frame 해석, 중첩 Frame, stale 핸들 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- layout 엔진(Phase X)이 이 해석 함수를 containing block 결정의 단일 경로로
  사용해야 합니다.
