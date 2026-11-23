# 작업 보고서: IX.11.10 Frame border 4변 longhand

## WBS

- WBS: IX.11.10
- 작업명: Frame border를 4변 `border-*-width`·`-style`·`-color` longhand로 바꾸는 migration(IX.11.21 registry step)
- 상태: 부분 완료 (`[~]`) — 열두 longhand key·값 모델·CSS 값 해석과 출력·레이아웃 입력·schema 2 테두리 절 형식·migration step 변환은 들어왔습니다. Frame payload 타입 교체와 step 등록, 레이아웃 엔진의 border 입력은 남았습니다(아래 "차이").
- 결정: [ADR-0075](../adr/ADR-0075-css-property-registry.md) 개정 결정 10.

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일

- `crates/document/src/css/border.rs`(신규): 열두 longhand key(0x0122\~0x012d), `BorderLonghand`·`BorderSides` 값 모델, CSS 값 해석·출력(`css::value` 사용), 묶음 값 0x0105와의 변환(`from_payload`·`to_payload`), computed 두께와 레이아웃 입력 선언, schema 2 테두리 절 canonical bytes(`encode`·`decode`), registry 항목.
- `crates/document/src/css/registry.rs`·`mod.rs`: Frame 범위에 테두리 항목 열두 개를 잇고 모듈을 등록합니다.
- `crates/document/src/css/registry_snapshot.txt`: registry golden에 열두 줄이 더해졌습니다.
- `docs/design/property_model.md`: 5장 표에 0x0122\~0x012d 행(registry 상태 일치 test 대상).
- `crates/project/src/document_io/source_v4/{keys,node}.rs`: 텍스트 원본 v4의 자체 테두리 해석·이름 표를 지우고 `css::border`를 씁니다. 파일 bytes는 같습니다.
- `crates/project/src/migration/steps/{mod,frame_border}.rs`(신규 step): schema 1→2 테두리 변환과 보고서, 0x0105 참조 보고.
- `docs/adr/ADR-0075-css-property-registry.md`: 개정 결정 10.

## 구현 요약

- 열두 longhand의 순서는 변마다 width·style·color이고 registry 순서가 텍스트 원본 줄 순서와 같습니다.
- 옛 `Border{color, thickness}`는 값 변환 없이 4변 같은 두께·`solid`·색으로, 없음은 `medium none currentcolor`로 읽힙니다. 묶음 값으로 되돌릴 수 없는 조합은 `unsupported-value`로 거부합니다(조용히 바꾸지 않음).
- step `split_frame_border(records, set)`(IX.11.9 step 뒤 적용): `display`·`position`·`box-sizing`·`padding-*`·inset은 명시 값이 없으면 활성 기본값 세트 값으로 풉니다(`css` 세트 block 부모, `react-native` 세트 flex 부모 모두 flow 부모). 테두리 있는 부모의 Free 자식(`position: absolute`) 길이 inset에서 그 변 두께를 빼고, 테두리 있는 Frame의 `padding-*`이 두께를 흡수합니다. padding이 모자라는 변은 in-flow 자식이 있을 때 노드·변별로 보고합니다. `content-box`면 흡수하지 못한 두께만큼 payload 크기를 줄여 border box 크기를 지킵니다(`size_adjusted`). 백분율 inset·크기·padding과 위치가 payload에 있는 자식은 보고합니다. 테두리 값 bytes는 바꾸지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document --lib css::` | 48 passed(테두리 key·이름·순서, 옛 묶음 값 변환, 4변 다른 값 텍스트·bytes round-trip, 값 parser 거부 코드, registry snapshot·속성 모델 표 일치) |
| `cargo test -p stellar-project --lib` | 432 passed, 1 ignored(v4 codec 테두리 줄, migration step 4개: Free 자식 화면 위치 보존, 중간 schema right·bottom·백분율 보고, flex 부모 padding 보정과 부족분 보고, 0x0105 참조 보존) |
| `cargo test -p stellar-project --tests` | 통과(실패 0) |
| `cargo test -p stellar-commands -p stellar-semantic-diff -p stellar-semantic-merge` | 통과(실패 0) |
| `cargo clippy -p stellar-document -p stellar-project --all-targets` | 경고 0 |

## 완료 기준 대비

| 기준 | 상태 |
|---|---|
| 4변 다른 값 round-trip | 값 모델·CSS 텍스트·schema 2 테두리 절 bytes에서 확인. Frame payload에 저장하는 연결은 남음 |
| border가 있는 부모의 Free 자식 화면 위치 보존 fixture | step 결과의 `left`·`top` + 부모 border 두께가 schema 1 위치와 같음을 확인 |
| layout record가 있는 중간 schema의 flex 부모 flow 자식 위치·내용 기반 크기 보존 | padding 보정 산술로 확인. 레이아웃 엔진 실행 비교는 엔진에 border 입력이 없어 못 함 |
| padding이 두께보다 작은 변의 보고서 기록 | 확인 |
| 참조 bytes 보존 fixture | 경로를 바꾸지 않고 보고함을 확인 |
| migration golden 외형 동일(1 LayoutUnit 이내), border 있는 flex 자식 위치가 IX.11.8 기대값과 일치 | 미확인. 레이아웃 엔진(`crates/layout`)이 `border: 0` 고정이라 border 입력을 받지 못합니다 |
| 같은 diff의 ADR, `cargo xtask validate-adr` | ADR-0075 개정 결정 10. 보호 경로 변경 없음 |

## 차이

- `FramePayload::border`(`crates/document/src/frame.rs`)를 4변 구조로 바꾸는 일과 chunk 연결, `frame_metadata`의 0x0105 `longhands` 지정은 이 lane 소유 밖입니다. 이 필드를 읽는 crate(semantic-diff·scene·runtime·codegen·ops·commands·ffi)도 함께 바뀌어야 합니다.
- 레이아웃 엔진 border 입력: `stellar_layout`은 `border: 0`을 고정 값으로 둡니다. `BorderSides::layout_declarations`가 변별 computed `border-*-width` 선언을 주지만 엔진·`style_from_declarations`가 받지 않습니다(`crates/layout` 소유 밖).
- 기본값 세트(`css::style_defaults`)에 테두리 longhand 항목이 없습니다. 테두리는 payload 값이 언제나 명시 값이라 지금은 필요 없지만, 희소 저장으로 바꾸면 `css` 세트 initial(`medium none currentcolor`)을 더해야 합니다.
- step은 `PRODUCT_STEPS`에 등록하지 않았습니다(IX.11.9와 같은 시점에 1→2 묶음으로 등록).

## 남은 위험

- step을 같은 입력에 두 번 적용하면 두께를 두 번 뺍니다. schema step으로 한 번만 실행되어야 합니다.
- (검토 보완) 처음 구현은 명시 `display: flex|grid`만 flow 부모로 보고, content-box Frame의 크기 증가와 `position`이 absolute가 아닌 자식의 inset 보정을 다루지 않았습니다. 검토에서 기본값 세트 해석과 content-box 크기 보정으로 고쳤습니다.
