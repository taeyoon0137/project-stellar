# 작업 보고서: XII.10.19 눈금자와 가이드

- WBS: XII.10.19
- 작업명: 눈금자와 가이드(만들기·옮기기·지우기, 눈금 원점, 눈금자 표현 C1)와 가이드의 문서 저장 연결
- 상태: 부분 완료 (`[~]`) — 코어 값·계산·제스처와 문서 모델·command·저장·텍스트 원본·Semantic Diff·Merge·CLI·MCP 연결 완료.
  ⌘R 메뉴와 눈금자·가이드 그리기(셸)는 UI 재설계 때 연결합니다.
- 결정: ADR-0117 결정 2·3, ADR-0126
- 선행: 계획서 행의 선행 열

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, Apple Silicon). 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 변경 파일 목록

1차(코어 값·제스처, 커밋 e0ba6a0b):

- `crates/document/src/guides.rs`, `crates/runtime/src/canvas/ruler.rs`

2차(문서 저장 연결, ADR-0126):

- `crates/core-types/src/reserved.rs`: `reserved::guides`(subtree 확장 절 tag 0x01·0x02, snapshot key 0xED01·0xED02·0xED10·0xED11)
- `crates/document/src/guides.rs`(줄 단위 formatter·parser `to_line`·`GuideLine::parse`·`lines`·`from_lines`, `GuideEdit` bytes),
  `crates/document/src/guides/persist.rs`(신규: validator 검사, subtree 확장 절 codec), `model.rs`·`records.rs`·`record_codec.rs`·
  `validate.rs`(필드·확장 절 API·validator 연결)
- `crates/commands/src/guides.rs`(신규: `SetNodeGuides`·`EditViewVariants`), `guides_tests.rs`·`guides_test_support.rs`(신규),
  `kind.rs`·`command.rs`·`codec.rs`(종류 0x00B0·0x00B1, codec version 7), `edit.rs`(primitive·journal),
  `snapshot_apply.rs`·`agent_external.rs`(외부 반영 tag 9·10), `convert.rs`(복원이 가이드·layout record와 맞지 않는 종류 변경 거부),
  `property_tests.rs`·`test_support.rs`·`fuzz.rs`·`lib.rs`(등록)
- `crates/project/src/document_io/serialize.rs`·`load.rs`(확장 절 경로), `text_codec.rs`(옛 원본 레코드 필드)
- `crates/project/src/document_io/source_v4/view_data.rs`(신규: `guides` 목록·View 변수 표기), `node.rs`·`view.rs`·`project.rs`·`mod.rs`
- `crates/semantic-diff/src/guides.rs`(신규: 투영·역투영·범주), `build.rs`·`records.rs`·`keys.rs`·`snapshot.rs`·`lib.rs`
- `crates/semantic-merge/tests/guides_merge.rs`(신규)
- `crates/ops/src/guides.rs`(신규), `tools/stellar-cli/src/guides.rs`(신규)·`cli.rs`·`lib.rs`·`tests/fixtures/help.txt`·`tests/guides.rs`(신규),
  `crates/agent-protocol/src/catalog/tools.rs`·`crates/mcp-core/src/headless.rs`·`headless/guides.rs`(신규),
  `tools/stellar-mcp/tests/*`(tool 목록·fixture)
- `docs/adr/ADR-0126-guides-grids-view-variables-persistence.md`(신규), `docs/adr/ADR-0117-…`(개정 절)

## 구현 요약

- **C1 결정과 근거**(1차와 같음): 눈금자는 셸이 화면 고정 띠로 그리고, 눈금 값과 화면 위치는 코어 `ruler_ticks`가 scene camera와 같은
  `CanvasViewport`에서 계산합니다. 같은 viewport 값에서 계산하므로 camera와 어긋날 수 없고 fixture가 world→view 변환 결과와 비교합니다.
- **문서 데이터**: `DocumentRecords::node_guides`와 `DocumentModel::node_guides`·`set_node_guides`. node를 지워도 기록은 StableId로 남아
  삭제 undo가 되살리고, 영속 레코드에는 살아 있는 Frame의 기록만 나옵니다. validator는 Frame이 아닌 소유자·중복·빈 기록을 거부합니다.
- **command**: `set-node-guides`(0x00B0) = owner + `GuideEdit` 하나. 역은 `NodeGuides::apply`가 돌려준 역 편집이라 제스처 하나가 history
  항목 하나이고 undo가 정확히 되돌립니다. 무효화는 비어 있습니다(layout 입력 아님). 외부 원본의 가이드 변경은 외부 반영 command의
  tag 9로 들어옵니다.
- **chunk**: 소유 Frame이 있는 View subtree chunk 끝의 확장 절(tag 0x01, `NodeGuides::encode`). 가이드가 없는 문서의 chunk bytes는 그대로이고,
  확장 절을 모르는 `decode`·`view_subtrees`는 버리지 않고 거부합니다.
- **텍스트 원본**: Frame node 항목의 `guides` 목록. 줄 문법은 문서 crate `NodeGuides::lines` 그대로이고 정규형만 받습니다.
- **Semantic Diff·Merge**: 가이드 하나 = 소유 Frame 객체의 `[0xED01] + 가이드 id` 속성(값 = 원본 한 줄). 다른 가이드 동시 편집은 충돌 없음,
  같은 가이드 양쪽 이동·이동과 삭제는 그 경로 충돌(semantic-merge 시험). 범주는 `guides`.
- **CLI·MCP 조회**: `stellar guides <경로> <문서 id> [--node <Frame id>]`와 MCP `document.guides`가 같은 `ops::guides::read_snapshot_guides`를 씁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document --lib guides` | 통과(9): 편집 역 연쇄, bytes·줄 정규형 왕복, 확장 절 왕복·거부, 모델→레코드→subtree bytes→모델 왕복, validator 거부 |
| `cargo test -p stellar-commands` | 통과(244, ignored 6): 가이드 command 생성·이동·삭제·그리드 교체의 역 연쇄와 bytes 왕복, rollback, 외부 반영 왕복, 새 종류 property round trip·변조 fuzz·sequence fuzz |
| `cargo test -p stellar-project --lib view_data` | 통과(4): `guides` 목록 정규형, 단일 View 원본·다중 파일 원본·chunk cache 왕복 |
| `cargo test -p stellar-semantic-diff --lib` | 통과(94): 가이드별 경로 투영·역투영, 한 가이드 이동이 그 경로만 바꿈, 정규형 아닌 값 거부 |
| `cargo test -p stellar-semantic-merge --test guides_merge` | 통과(3): 다른 가이드 병합, 같은 가이드 이동·삭제 충돌, 그리드 목록 값 충돌 |
| `cargo test -p stellar-cli --test guides` | 통과(1): 저장된 가이드·그리드 조회, `--node`, Frame 아닌 id 실패, 사용법 오류 64 |
| `cargo test -p stellar-mcp` | 통과: `document.guides` 목록·실행, catalog·trust-boundary fixture 갱신(`STELLAR_MCP_BLESS=1`) |
| `cargo xtask validate-adr --base ef01e66d` | 종료 코드 0 |
| `cargo xtask generate-ffi-header --check` | 종료 코드 0(FFI 헤더 drift 없음) |

전체 workspace 시험과 `check-core` 결과는 작업 결과 보고(lane 결과)에 적습니다.

## 차이(작업 지시와 다른 점)

- 저장 위치는 ADR-0117 결정 2.4가 예상한 별도 chunk kind가 아니라 subtree chunk 확장 절입니다(ADR-0126 결정 2: 국소 데이터라 그
  subtree chunk만 새 hash가 되고 manifest·storage kind 표를 바꾸지 않음). core-types에는 확장 절 tag를 예약했습니다.
- ⌘R 표시 토글 메뉴 연결과 눈금자·가이드 그리기는 셸 재설계 때 합니다(사용자 방향 2026-11-23).
- CLI·MCP의 가이드 **편집** 입력(`ops::mcp_command_input`)은 이번 범위 밖입니다. 같은 `set-node-guides` command를 쓰면 됩니다.

## 남은 위험 / 한계

- Frame 복제·붙여넣기는 가이드·그리드를 옮기지 않습니다(layout record와 같은 남은 범위).
- command codec version을 7로 올렸습니다. 동시에 다른 작업이 version을 올리면 통합 때 번호를 맞춰야 합니다.
- 회전·확대된 최상위 Frame에는 Frame 기준 눈금·가이드를 두지 않습니다(View root 기준으로 대체).

## 후속 수정 (2026-11-24 instance-integrity: Frame 복제·붙여넣기·삭제 undo에서 가이드·그리드가 따라감, ADR-0133 결정 4)

위 "남은 위험"의 "Frame 복제·붙여넣기는 가이드·그리드를 옮기지 않습니다"를 해소했습니다. 레이아웃 그리드(XII.10.20)도
같은 기록이라 함께 따라갑니다.

### 변경 파일

- `crates/commands/src/node_ops.rs` — `InsertNode.node_guides`(삽입하는 Frame의 기록, 문서 validator와 같은 검사),
  `delete-subtree` 역이 subtree의 기록을 담음, 적용 때 기록을 담지 않은 삽입 node의 남은 기록을 지움(`apply_guides`),
  끝 절 codec `encode_insert_tail`·`decode_insert_tail`(layout record 절과 하나의 canonical 형식).
- `crates/commands/src/duplicate.rs` — `remap_guides`(소유 Frame만 새 id, 가이드 id는 Frame 안에서만 유일해 그대로)와 복제.
- `crates/commands/src/paste.rs`(소유 밖, `clipboard*` 범위의 붙여넣기 파일) — 붙여넣기 계획이 기록을 새 Frame으로 옮김.
- `crates/commands/src/detach.rs` — 인스턴스 구조 command `insert_guides`(만들 Frame의 기록, 지운 node의 기록을 역에 담음).
- `crates/commands/src/codec.rs`(소유 밖, 등록) — command codec 버전 9.
- 구조체 필드 추가에 따른 literal 한 줄씩(소유 밖, 컴파일 유지): `crates/commands/src/{convert,test_support}.rs`,
  `crates/runtime/src/canvas/{instance.rs,text/edit.rs}`, `crates/scene/src/document/layout/tests.rs`,
  `tests/integration/tests/{chunk_layout,component_library,project_lifecycle}.rs`.

### 실행한 명령

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-commands --lib` | 통과(266 passed, 6 ignored) |
| `cargo test -p stellar-history` | 통과(55 passed) |

### 테스트

- `duplicate_copies_guides_to_the_new_frames_and_undo_removes_them`: Frame 11과 자식 13의 가이드 2개·12열 그리드가 복제 Frame
  둘에 같은 값으로 생기고(소유 Frame만 다름) 원본은 그대로, undo로 복제 기록이 사라집니다.
- `paste_moves_guides_to_the_pasted_frames`: 클립보드 bytes 왕복 뒤 붙여넣기 계획의 기록 소유 node가 새 id이고 적용 결과가
  같은 값입니다.
- `delete_undo_restores_guides_after_the_journal_is_reopened`: 삭제의 역 command를 codec 왕복한 뒤 저장 레코드로 다시 만든
  문서(남은 기록 없음)에 적용하면 기록이 돌아옵니다.
- `slot_content_delete_undo_restores_frame_guides_after_reopen`: 인스턴스 구조 command의 역도 같은 규칙입니다.
- `guide_section_round_trips_and_keeps_older_bytes_without_guides`·`guide_section_must_be_canonical`·
  `insert_rejects_guides_outside_the_insert_or_on_non_frames`: 가이드 기록이 없으면 bytes가 버전 8과 같고, 수 0 layout 절·빈
  가이드 절·내림차순을 거부하며, 삽입하지 않는 node·Frame이 아닌 node·빈 기록을 거부합니다.

### 남은 위험

- command codec 버전 9는 registry 전역 변경입니다. 동시에 다른 작업이 버전을 올리면 통합 때 맞춰야 합니다.
