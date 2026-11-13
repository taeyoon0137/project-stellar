# 작업 보고서: VI.3.4 Frame clip 형식

- WBS: VI.3.4
- 작업명: clip path가 없을 때 frame bounds clip, 별도 PathRef clip 두 형식
- 상태: 완료 (`[x]`)
- 선행: VI.3.1, III.2.4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/frame.rs` — `Clipping::Path(NodeRef)` 추가(tag 0/1/2),
  `ClipResolution`, `resolve_clip`, Frame 속성 key 모듈(`frame::keys`), binary
  encode/decode 갱신.
- `docs/task-reports/VI.3.4.md`(신규), 계획서(VI.3.4 `[ ]`→`[x]`).

## 구현 요약

- **두 clip 형식**: clip path가 없을 때의 기본은 `FrameBounds`이고, 별도 Path
  node로 clip하는 `Path(NodeRef)`를 추가했습니다(`Disabled`는 VI.3.1부터 유지).
  저장 tag는 0 = 없음, 1 = bounds, 2 = PathRef(+16바이트 StableId).
- **해석**: `resolve_clip(frame_id, clipping, lookup)`이 PathRef 대상을 찾지 못하면
  frame을 invalid로 만들지 않고 `PropertyPath(frame, CLIPPING)` 위치의
  `UnresolvedRef`(기대 타입 Node, III.2.4)로 보고합니다. 대상이 Path가 아닌
  종류면 `NotAPath` 진단입니다.
- **속성 key**: Frame 속성 key 7종(`0x0101`~`0x0107`)을 `frame::keys`에 고정해
  진단 위치와 VI.3.5 metadata registry가 같은 key를 씁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 63 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| missing clip ref → unresolved diagnostic(id·기대 타입·위치 보존)(`missing_clip_ref_is_unresolved_diagnostic`) | 통과 |
| 기본 bounds clip, Disabled, Path 해석, 비Path 대상 진단(`clip_resolution_forms`) | 통과 |
| 3개 clip 형식 binary round-trip(`clip_forms_round_trip`) | 통과 |

## 차이(작업 지시와 다른 점)

- 참조 대상이 Path가 아닌 경우의 `NotAPath` 진단을 추가했습니다(지시에는 없음).
  clip geometry를 얻을 수 없는 경우를 미해결과 구분해 보고하기 위함입니다.

## 남은 위험 / 한계

- Shape(VI.5)를 clip 대상으로 허용할지는 Shape→Path 평가(VI.5.6) 이후 재검토할 수
  있습니다. 현재는 Path kind만 허용합니다.
