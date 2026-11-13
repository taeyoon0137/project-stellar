# 작업 보고서: XX.3.3 StableId 기반 결정론적 SVG id와 충돌 검사

- WBS: XX.3.3
- 작업명: SVG id를 StableId에서 deterministic safe id로 생성하고 collision 검사 — 같은 snapshot은 byte 단위로 같은 SVG
- 상태: 완료 (`[x]`)
- 선행: III.1.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/export/src/svg/id.rs`(신규) — `SvgId`, `SvgIdAllocator`, `SvgIdError`.
- `crates/export/src/svg/writer.rs`(신규) — `write_svg`, `SvgDocument`, `SvgWriteOptions`,
  `SvgExportError`.
- `crates/export/src/svg/writer/tests.rs`(신규) — writer 테스트와 출력 구조 검사기.
- `crates/export/tests/fixtures/basic_view_v1.svg`(신규) — View fixture golden SVG.
- `crates/export/src/svg/mod.rs` — 모듈 선언·재노출.
- `docs/task-reports/XX.3.3.md`(신규), 계획서(XX.3.3 `[ ]`→`[x]`).

## 구현 요약

- **id 규칙**: 순회 번호가 아니라 StableId에서 만듭니다. node 그룹 `n-<uuid>`, 같은 StableId의
  k번째(k ≥ 2, pre-order) scene node `n-<uuid>-d<k>`, scene clip 정의 `<처음 쓴 node id>-c<깊이>`,
  Inside stroke 자기 기하 clip `<node id>-g`. 모든 id는 XML `ID`·CSS 식별자 공통 부분 집합
  (`[A-Za-z][A-Za-z0-9_-]*`, ≤128 byte)이고, 발급기가 발급한 id를 모두 기억해 중복 발급을
  `Collision`으로, 규칙 위반을 `Unsafe`로 거부합니다.
- **문서 writer**: `ExportCapture`만 읽습니다(XX.1.2). 최상위 node를 paint 순서로, subtree를 명시적
  stack pre-order로 걷고(재귀 없음) 숨긴 subtree는 생략합니다. node마다 `<g id>` 하나에 image 픽셀 →
  칠하기 layer → 자식을 씁니다(XX.3.1 mapping·표기).
- **clip**: node clip stack은 부모 그룹이 건 clip으로 시작해야 하며(아니면 `ClipNotNested`), 그룹에는
  새로 늘어난 clip만 겁니다(조상 clip은 SVG 상속으로 적용, 이중 적용 없음). 한 node에서 둘 이상
  늘면 clipPath chain을 만들고, 같은 (chain 안 바깥 정의, clip 값) 조합은 문서 안에서 한 정의를
  공유합니다. selection 내보내기는 범위 밖 조상 clip을 적용하지 않습니다.
- **결정론**: 출력은 입력 값에만 의존합니다. `HashMap`은 조회에만 쓰고 출력 순서는 순회·최초 사용
  순서로 고정합니다. scene revision·`Arc` 공유 구조·생성 순서·실행 thread는 출력에 들어가지 않습니다.
- **거부·상한**: vector가 아닌 기능(Text, shadow, blur, blend, Outside stroke, image 색공간 override)은
  그 node에서 `Unsupported`로 멈춥니다. 고정 asset revision에 없는 이미지는 `MissingImage`, 요청 format이
  PNG면 `FormatMismatch`. 출력은 기본 256 MiB 상한이며 이미지 base64는 쓰기 전에 길이를 검사합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-export` | 55 passed, 0 failed (이 작업 신규 18) |
| `cargo fmt -p stellar-export -- --check` | 통과 |
| `cargo clippy -p stellar-export --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 같은 capture 2회, 따로 만든 같은 내용 snapshot(다른 revision·allocation), codec round-trip snapshot, 다른 thread 출력이 모두 byte 동일(`same_snapshot_is_byte_identical`) | 통과 |
| View fixture 출력이 golden SVG와 byte 동일, 그룹 4·clip 정의 2·id 6(`basic_view_matches_golden`) | 통과 |
| 같은 StableId가 여러 scene node를 만들면 `-d2`, `-d3`로 구분, 모든 id 안전 규칙 통과(`repeated_sources_get_distinct_safe_ids`) | 통과 |
| id 형식 비중첩·중복 발급·unsafe id 거부(`id.rs` 테스트 4건) | 통과 |
| capture 뒤 편집·asset 교체가 출력에 섞이지 않음(`edits_during_export_do_not_change_the_document`) | 통과 |
| 중첩 clip은 새 clip만 걸고 정의 공유, 한 node에서 두 clip이 늘면 chain(`clip_definitions_cover_only_new_clips_and_are_shared`, `multiple_new_clips_on_one_node_form_a_chain`) | 통과 |
| selection은 조상 clip 미적용, 부모 clip을 벗어난 자식은 `ClipNotNested` | 통과 |
| 미지원 기능 typed error, 숨긴 node는 영향 없음, 고정 revision 밖 asset·PNG 요청 거부 | 통과 |
| 출력 상한(정확히 같은 길이 허용, 1 byte 작으면 거부, 큰 이미지 사전 거부) | 통과 |
| 배경·scale·group opacity 전체 문서 golden(`background_scale_and_opacity_are_recorded`) | 통과 |
| 깊이 1000 tree를 64 KiB stack thread에서 기록, 출력 구조 검사(`deep_trees_are_written_without_recursion`) | 통과 |

모든 출력은 테스트 검사기(`validate`)로 태그 균형, id 유일성, 모든 `url(#…)` 참조의 정의 존재,
`<text` 부재를 확인했습니다.

## 차이(작업 지시와 다른 점)

- 검증 기준 "same snapshot byte-identical"을 확인하려면 문서 전체를 쓰는 writer가 필요해, XX.3.1이
  정의한 mapping·요소 표기를 조립하는 문서 writer를 이 작업에서 함께 구현했습니다.
- 실제 browser 렌더링은 확인하지 않았습니다. 이 환경의 내장 browser는 로컬 파일을 열 수 없고, browser
  render fixture는 XX.3.4(선행 XI.10.4 미완료)의 검증 기준입니다. 구조 검사는 테스트 검사기로만 했습니다.

## 남은 위험 / 한계

- clip 정의 id는 그 clip을 처음 쓴 node에서 이름을 얻으므로, 그 node가 삭제되면 같은 clip의 정의 id가
  다음 사용 node 기준으로 바뀝니다(같은 snapshot의 결정론에는 영향 없음).
- Text·shadow·blur·blend가 있는 scene은 아직 SVG로 내보낼 수 없습니다(XX.3.2, XX.4.x 선행 미완료).
- SVG 파일 저장(temp + atomic rename)과 진행·취소는 export command(XX.5.1)에서 연결해야 합니다.
