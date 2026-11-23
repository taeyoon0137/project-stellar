# 작업 보고서: IX.11.3 Shape·Path·Text·Image registry와 CSS 매핑

- WBS: IX.11.3
- 작업명: Shape·Path·Text·Image property metadata registry를 Frame registry와 같은 방식으로 두고 CSS 매핑을 채움, snapshot 전용 key를 문서 key로 교체
- 상태: 완료 (`[x]`) — 2026-11-23 근거 커밋 기록(ledger-refresh): `[~]`의 이유였던 남은 범위(값 여러 개를 한 경로에 담던 snapshot 투영)를 integration-followups lane이 끝냈고(아래 "후속 보완" 절), 선행 IX.11.1·IX.11.22도 `[x]`입니다. 작업 커밋 제목이 `Task(<ID>)` 형식이 아니어서 원장이 근거 커밋을 찾지 못했습니다. 아래 "근거 커밋" 절에 실제 작업 커밋을 적었습니다. 이전 기록: 완료 (`[x]`) — registry·CSS 매핑·key 할당, 값 하나인 snapshot 경로의 문서 key 교체, 값 여러 개를 한 경로에 담던 snapshot 투영의 longhand 경로 분할(2026-11-23 integration-followups 보완)까지 끝났습니다.
- 결정: ADR-0075 결정 4

## 필수 실행 환경

- `DEV-MAC`(Rust 단위 테스트).

## 변경 파일 목록

- `crates/document/src/payload_metadata.rs` — `PayloadPropertyMetadata.css`(모든 항목), `stellar_only` bool을
  매핑 상태로 대체(`is_stellar_only`), `PayloadPropertyType::{Bool, Text, Structured}`, 공통 node 필드 registry
  (`node_keys` 0x0001\~0x0003, `NODE_PROPERTIES`), Path registry(`path-data` 0x0401 payload 단위, `fill-rule`
  0x0402), Text registry(`text-content` 0x0501 payload 단위), Image `image-asset`(0x0601)·`image-crop`(0x0606)·
  `image-color-space`(0x0607), 필드 대응 함수와 test.
- `crates/document/src/css/registry.rs` — 범위 `Node`·`Shape`·`Path`·`Text`·`Image` 추가, 속성 모델 3·6·7장 표
  비교. `registry_snapshot.txt` golden 갱신.
- `crates/commands/src/property.rs` — 구조 값(`Structured`)은 `SetProperty` 대상이 아님, Image 편집 key 목록에서
  구조 값 제외.
- `crates/commands/src/inspector.rs` — `InspectorProperty::all()`(test용 공개 목록).
- `crates/commands/src/property_tests.rs` — command `node_keys`와 문서 registry 일치, 속성 패널 행의 이름·문법이
  registry와 같음.
- `crates/semantic-diff/src/keys.rs` — `LINE_CAP`·`POLYGON_SIDES`·`POLYGON_ROTATION`·`POLYGON_CORNER_RADIUS`·
  `PATH_DATA`·`TEXT_PAYLOAD`·`IMAGE_FIT`·`IMAGE_CROP`·`IMAGE_SAMPLING`·`IMAGE_COLOR_SPACE`를 문서 key로 교체, 전용
  대역 test 재작성, 남은 전용 key 목록 test.
- `docs/design/property_model.md` — 3·6·7장 표에 key 번호와 registry 이름, 6장 도입 문단 현행화.

## 구현 요약

- key 상위 byte 규칙(Shape `0x03xx`, Path `0x04xx`, Text `0x05xx`, Image `0x06xx`)을 지키고 기존 key는
  재사용하지 않았습니다. Shape·Image `width`·`height` key(0x0301·0x0302, 0x0602·0x0603)는 그대로이며 IX.11.6
  item record가 같은 key를 씁니다.
- Path command·point 목록과 Text content는 payload 단위 key 하나입니다(속성 모델 15장 O12). Text style longhand
  key는 IX.11.11이 할당합니다.
- 구조 값(asset·crop·색공간·path·text 단위)은 key와 CSS 매핑은 갖지만 `SetProperty`로 바꾸지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document --lib` | 380 passed |
| `cargo test -p stellar-commands --lib` | 통과(registry_consistency 2 passed 포함) |
| `cargo test -p stellar-semantic-diff` | 85 + 4(fuzz) passed |
| `cargo test -p stellar-semantic-merge -p stellar-ops -p stellar-cli` | 실패 0 |
| `cargo clippy -p stellar-document -p stellar-commands -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| payload 필드와 registry 1:1 누락 검사(Shape·Image·Path·Text) | 통과 |
| key 중복 0(모든 registry 합산), 상위 byte 규칙 | 통과 |
| CSS 매핑 누락 0, 속성 모델 3·6·7장 표와 상태 일치 | 통과 |
| 문서 key가 생긴 필드의 snapshot 전용 key 사용 0(값 하나인 경로) | 통과. 남은 전용 key는 값 여러 개 경로 7개 + 도형 종류 tag |
| 갱신한 diff golden | 없음. CLI·ops fixture의 속성 경로는 공통 필드(0xE0xx)뿐이라 바뀌지 않았습니다(`grep`으로 확인) |

## 차이(작업 지시와 다른 점)

- 값 여러 개를 한 snapshot 경로에 담는 투영의 분할은 소유 파일 밖이라 하지 않았습니다(상태 참조).
- 공통 node 필드 registry(`node_keys` 0x0001\~0x0003)를 문서 crate에 더했습니다. 숫자는 command `node_keys`와 같고 test로 고정합니다.

## 남은 위험 / 한계

- 후속(semantic-merge lane): `build.rs`·`records.rs`에서 `RECT_SIZE`→`shape_keys::WIDTH`·`HEIGHT`,
  `RECT_CORNER_RADIUS`→`shape_keys::BORDER_RADIUS_LONGHANDS`, `LINE_START`·`LINE_END`→`X1`·`Y1`·`X2`·`Y2`,
  `IMAGE_SIZE`→`image_keys::WIDTH`·`HEIGHT`로 경로를 나누고 옛 snapshot 경로 읽기를 유지합니다. Ellipse·Polygon
  bounds 원점은 위치 longhand가 아니라 기하 offset이므로(속성 모델 12장 Position 행) 크기만 longhand로 나눕니다.

## 후속 보완(2026-11-23, integration-followups)

- `crates/semantic-diff/src/build.rs`·`records.rs`: Rectangle 크기 `RECT_SIZE`→`shape_keys::WIDTH`·`HEIGHT`, 모서리
  `RECT_CORNER_RADIUS`→`BORDER_RADIUS_LONGHANDS`, Line `LINE_START`·`LINE_END`→`X1`·`Y1`·`X2`·`Y2`, Image
  `IMAGE_SIZE`→`image_keys::WIDTH`·`HEIGHT`, Ellipse·Polygon bounds 크기→`shape_keys::WIDTH`·`HEIGHT`로 나눴습니다.
  bounds 원점은 registry key가 없어 `ELLIPSE_BOUNDS`·`POLYGON_BOUNDS` 번호의 `(x, y)` 두 값 경로로 남습니다.
- 옛 snapshot 전용 경로는 읽지 않습니다. 위 "남은 위험"의 "옛 snapshot 경로 읽기 유지"와 다릅니다. 이 key들은 문서
  key가 아니고 snapshot은 저장하지 않으므로, 옛 경로가 들어오는 입력이 없습니다. 문서 registry의 묶음 key(Frame
  크기·모서리, Shadow offset)만 옛 경로를 longhand가 없을 때 읽습니다(ADR-0075 개정).
- 검증: `build::tests::multi_value_fields_project_to_document_longhand_paths`(random 문서 200개에서 옛 경로 0, longhand
  값 하나, 원점 두 값), 기존 `projection_round_trips_records`(build→records→build 동일), status·stage test의
  기대 경로를 `shape_keys::WIDTH`로 갱신. `cargo test -p stellar-semantic-diff` 88 + fuzz 4 passed.

## 근거 커밋 (2026-11-23 기록)

이 행의 작업은 아래 커밋에 있습니다. 이 기록 커밋(`Task(IX.11.3)`)은 원장이 근거 커밋을 찾도록 남긴 것이고 코드 변경은 없습니다.

- `f8b002ab` Feat: Shape·Path·Text·Image registry와 CSS 매핑 추가
- `27ac2535` Feat: semantic snapshot의 다값 경로를 longhand 문서 key로 나누고 묶음 key 검사를 validator에 연결
- `67aea861` Test: Git snapshot 통합 테스트가 Line 끝점 longhand 경로를 기대하게 함
