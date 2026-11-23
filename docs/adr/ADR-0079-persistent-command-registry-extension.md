# ADR-0079: 외부 snapshot 반영·Page·View·Path point·Text 범위·인스턴스 교체 command 등록과 journal codec 버전

## Status

Accepted (2026-11-23). engine lane `persistent-commands`의 결정입니다. UX.4.1·UX.12.1의 차단(blocked)을 선택지 1(코어
command를 먼저 만들고 runtime·셸 연결은 뒤로)로 풉니다.

## Context

- `CommandKind`는 closed numeric registry이고 번호는 command 직렬화와 history journal의 일부입니다(VII.1.2, XXV.1).
  registry를 바꾸는 작업은 전역 mutex 작업이라 한 lane에 모았습니다.
- 멈춘 행이 두 개 있었습니다. UX.4.1(앱을 연 채 외부에서 바뀐 원본을 Undo 한 번으로 반영)과 UX.12.1(Page·View 추가·
  이름 변경을 Undo 한 번으로). 둘 다 해당 command가 코어에 없어서 막혀 있었습니다.
- XII.10.28(Path point 편집), XII.10.32(Text 범위 편집), XII.10.35(인스턴스 교체)도 새 command 종류가 필요합니다.
- history journal(XXV.1)은 command canonical bytes를 그대로 담습니다. 새 종류가 생기면 옛 앱이 새 journal을, 새 앱이 옛
  journal을 만나는 경우를 정해야 합니다.
- 사용자 방향(2026-11-23): 지금은 엔진·기능만 만듭니다. 셸 FFI와 Swift UI는 만들지 않고, runtime 창 연결(watcher가
  command를 발행하고 제스처가 끝난 뒤 반영)은 `window-session-stale` lane이 맡습니다.
- crate 의존 방향은 `history → commands → document → core-types`이고 commands는 semantic-diff·expressions에 의존할
  수 없습니다(`tools/xtask/src/crate_graph.rs`).

## Decision

### 1. 새 command 종류와 번호

번호는 기존 대역 끝에 이어 주고 재사용하지 않습니다. 모두 command codec 2에서 처음 들어갑니다(`CommandKind::codec_version`).

| 번호 | 이름 | 의미 | 역 |
|---|---|---|---|
| `0x0051` | `set-text-range-style` | Text scalar 범위의 문자 style 설정·지우기 | `restore-node-records` |
| `0x0060` | `add-page` | 페이지(와 View·root subtree)를 표시 순 위치에 추가 | `remove-page` |
| `0x0061` | `remove-page` | 페이지와 그 안의 View·node 삭제 | `add-page`(내용 전체) |
| `0x0062` | `rename-page` | 페이지 표시 이름 변경 | `rename-page` |
| `0x0063` | `add-view` | 페이지에 View(와 root subtree) 추가 | `remove-view` |
| `0x0064` | `remove-view` | View와 그 안의 node 삭제 | `add-view`(내용 전체) |
| `0x0065` | `rename-view` | View 표시 이름 변경 | `rename-view` |
| `0x0070` | `apply-external-snapshot` | 외부 원본의 레코드 변경 집합 반영 | 이전·새 값을 바꾼 같은 종류 |
| `0x0080` | `edit-path-point` | Path point 편집 연산 하나(closed enum) | `restore-node-records` |
| `0x0090` | `swap-instance` | 인스턴스의 `ComponentRef` 교체 | `swap-instance`(원래 payload) |

- lane 결정의 "ReplaceTextRange"는 기존 `replace-text`(`0x0050`)입니다. 이미 Unicode scalar 범위 교체를 VI.7.5 helper로
  하므로 같은 의미의 종류를 따로 만들지 않습니다.
- `remove-page`·`remove-view`는 lane 결정 목록에 없지만 추가·삭제의 역이 command여야 해서 더했습니다.
- Undo·Redo 메뉴의 동작 이름은 코어가 정합니다(`CommandKind::action_name`, `HistoryEntry::action_name`). 셸은 앞뒤
  문구("실행 취소" 등)만 붙입니다. 외부 반영 항목의 이름은 "외부 변경 반영"입니다.

### 2. 외부 snapshot 반영(`apply-external-snapshot`)

- 전체 snapshot을 담지 않습니다. 현재 문서와 새 원본 사이의 **레코드 변경 집합**을 담습니다. 각 변경은 이전 값과 새
  값을 함께 가지며(`RecordChange`), 역은 두 값을 바꾼 같은 command입니다. 역집합이 Undo입니다.
- 레코드 단위는 문서 모델(`DocumentModel`)이 소유하는 것입니다: 문서 이름, 페이지(위치·이름), View(소속 페이지·위치·
  이름·root), node 레코드, modifier stack, 문서 Text style resource 표. 레코드는 StableId로만 짝짓습니다(semantic diff
  XIV.3.3과 같은 식별 규칙).
- 변경 집합 계산(`ApplyExternalSnapshot::between`)은 commands 안에서 현재 모델과 새 원본 레코드(`DocumentRecords`)를
  비교합니다. commands는 semantic-diff에 의존할 수 없으므로, 호출자(runtime)는 디스크에서 읽은 원본 레코드(또는
  semantic-diff의 `document_records` 투영)를 넘깁니다.
- 컴포넌트 정의와 token은 문서 모델 밖(project library chunk·`design/components`)이라 이 command의 대상이 아닙니다.
  library 반영은 library를 소유한 경로가 하고, library가 command 모델에 들어오면 변경 tag를 더하고 codec 버전을
  올립니다.
- 검증: 각 변경의 이전 값이 현재 문서와 같아야 합니다(낡은 변경 집합 거부). 적용 결과는 전체 validator(VI.10.1)에서 새
  위반이 없어야 합니다. 외부 반영은 드문 동작이라 O(N) 결과 검사를 허용합니다.
- 출처: transaction 출처는 `CommandOrigin::System`입니다(`ApplyExternalSnapshot::ORIGIN`). lane 결정은 `External`
  출처를 말했지만, `CommandOrigin`(commands `id.rs`)은 이 lane 소유 파일이 아니고 `System`은 이미 "외부 변경 반영용
  synthetic command" 출처로 정의돼 있습니다. 외부 출처는 command 종류 자체가 나타냅니다.

### 3. history journal 호환

- journal schema를 2로 올리고 header에 **command codec version**(`u32`, minimum writer version 바로 뒤)을 기록합니다.
  현재 codec은 `COMMAND_CODEC_VERSION` = 2입니다.
- 새 앱은 옛 journal(schema 1)을 그대로 decode합니다(codec 1로 간주). 기준 commit `ce60bb92`의 writer가 쓴 bytes를
  fixture(`crates/history/fixtures/journal-schema1-codec1.stlhjnl`)로 둡니다.
- schema 2 header는 옛 reader가 해석할 수 없으므로 minimum reader·writer를 2로 기록합니다. 옛 앱은 schema 판정
  (`ReaderTooOld`)에서 journal 복원만 포기하고 문서 원본은 건드리지 않습니다.
- reader의 codec보다 새 codec(`CommandCodecTooNew`), header가 선언한 codec에 없는 종류(`KindOutsideCodec`), registry에
  없는 종류(`UnknownKind`)를 만나면 journal을 봉인 사유 `unknown-command`로 봉인하고 빈 history로 엽니다. 봉인 파일은
  bytes 그대로 보존합니다. 이 앱이 모르는 codec의 journal은 덮어쓰지 않습니다.
- grouping key 텍스트 입력 종류에 `Replace`(tag 4)를 더했습니다(결정 5).

### 4. Path point 편집(`edit-path-point`)

- 연산은 closed enum `PathEdit`입니다: `InsertPoint`(segment를 t에서 분할, 곡선은 de Casteljau), `MovePoint`(두 handle
  도 이동), `DeletePoint`(앞뒤 segment 병합), `MoveHandle`, `SetHandleMode`, `CloseSubpath`, `OpenSubpath`. 의미는 문서
  모델 `IdentifiedPath::edited`가 정합니다.
- point·handle은 StableId로 가리키고, 편집하지 않은 point·handle의 id와 mode는 유지합니다. 분할로 생기는 id는
  command가 담습니다.
- handle 대칭 모드(`HandleMode`: Independent·Aligned·Mirrored) 전환도 command입니다. mode는 `IdentifiedPath`에 기본값이
  아닌 point만 저장하고, 저장 bytes 끝에 mode가 있을 때만 `u32 개수 + (point id 16 + tag 1)*` 절을 붙입니다.

### 5. Text 범위 편집

- 범위 교체는 `replace-text`, 범위 style은 `set-text-range-style`입니다. style 설정은 범위와 겹치는 run을 범위 밖 조각만
  남기고 새 run을 더한 뒤 VI.7.2 정규화합니다.
- `GroupingKey::text_edit`가 입력 종류와 caret 전·후를 grouping key(VII.4.3)로 만듭니다. 선택을 다른 문자열로 바꾸는
  입력(`Replace`)은 앞뒤 입력과 합치지 않습니다. IME 조합 중 문자열은 command가 아니고 확정 문자열만 command입니다.

### 6. 인스턴스 교체(`swap-instance`)

- command는 교체 뒤 payload를 그대로 담아 library 없이 재실행됩니다. 역은 원래 payload입니다.
- `plan_instance_swap`은 적용 전 preview입니다. 직접 Property override는 옛 Property와 이름과 선언 타입이 같은 새 정의의
  공개 Property로 옮기고, 나머지(nested 경로, 맞는 Property 없음, 타입 다름, 비공개, 새 정의에 없는 Slot)는 이유와 함께
  탈락 목록으로 돌려줍니다. 결과는 key 순입니다.
- 크기: 인스턴스 payload에는 자기 크기가 없어 새 정의 root의 크기를 씁니다. "현재 크기 유지"(`SwapSizing::KeepCurrent`)
  는 인스턴스 크기를 CSS 값으로 담을 override 모델(ADR-0074)이 정해질 때까지 계획 단계에서 거부합니다.

### 7. Page·View command와 문서 정의 primitive

- 문서 정의 변경은 새 primitive `DocumentModel::replace_definition`과 `EditContext::replace_definition`을 지납니다(정의가
  점유하는 영속 id 소유 수 갱신, transaction guard 되돌리기 journal 포함). 두 파일(`crates/document/src/model.rs`,
  `crates/commands/src/edit.rs`)은 이 lane 소유가 아니지만 정의를 바꿀 다른 경로가 없어 메서드 추가만 했습니다.
- 문서 모델은 View 없는 페이지와 페이지 없는 문서를 허용하므로 command도 "마지막 View·페이지 삭제"를 막지 않습니다. 모든
  command의 역이 항상 적용 가능해야 하기 때문입니다. 최소 개수 같은 편집기 규칙은 UI가 정합니다.
- 추가 command는 결과 문서를 전체 validator로 검사하고, 삭제 command는 남는 node가 지울 node를 참조하면 거부합니다
  (VI.2.5).

### 8. 범위

- 이 ADR은 command·history·validator·CLI apply까지입니다. runtime 창 연결, 셸 FFI, 메뉴·`ShortcutPolicy`는 하지 않습니다.
- `stellar apply`는 등록된 모든 종류를 `{"kind": "<이름>", "canonical": "<16진수>"}`로 이미 받습니다. 새 종류를 위한
  필드 형식 JSON은 만들지 않았습니다. CLI 통합 테스트가 canonical bytes를 만들 수 있도록 `stellar-ops`가 test-support
  feature에서만 commands crate를 다시 내보냅니다.

## Alternatives

- **외부 반영에 전체 snapshot 담기**: 구현은 단순하지만 history 메모리와 journal 크기가 문서 크기에 비례합니다. 변경 집합은
  바뀐 레코드에만 비례합니다.
- **semantic-diff `SemanticChange`를 command에 담기**: commands가 semantic-diff에 의존해야 해 crate 방향을 어깁니다.
- **journal format version 올리기**: format version은 reader가 정확히 같아야 하는 컨테이너 형식이라, 올리면 새 reader도
  옛 journal을 읽을 수 없습니다. schema 2 + codec 필드로 옛 journal decode를 지킵니다.
- **handle mode를 geometry에서 유도**: 두 handle이 우연히 대칭이면 독립 mode를 표현할 수 없습니다. 저장합니다.
- **Path 편집 연산마다 종류 번호**: registry가 커지고 역 계산이 연산마다 갈립니다. 종류 하나 + closed enum이 직렬화
  계약을 한곳에 둡니다.
- **`ReplaceTextRange` 새 종류**: `replace-text`와 의미가 같아 같은 편집에 두 표현이 생깁니다.
- **인스턴스 크기 유지를 Transform modifier scale로 표현**: CSS 값이 아니라 ADR-0074와 어긋납니다.

## Consequences

- UX.4.1·UX.12.1의 코어 command가 생겨 runtime lane이 watcher 반영과 Page·View 메뉴를 연결할 수 있습니다.
- 외부 반영·정의 추가는 O(N) 결과 검사를 합니다. 일반 편집 command 경로의 비용은 바뀌지 않습니다.
- history journal은 이 변경 이후 옛 앱에서 복원되지 않습니다(문서는 무손상).
- handle mode가 있는 Path를 저장한 문서는 옛 앱이 읽지 못합니다.

## Compatibility

- command canonical 형식: 기존 종류의 bytes는 그대로입니다. 새 종류는 codec 2입니다.
- history journal: schema 1 journal은 새 앱이 읽습니다. schema 2 journal은 옛 앱이 `ReaderTooOld`로 봉인합니다. 봉인
  사유 token `unknown-command`가 추가됐습니다.
- 문서 저장 형식: handle mode가 없는 Path payload는 bytes가 같습니다. mode가 있으면 끝에 절이 붙고 옛 reader는 trailing
  bytes로 거부합니다. 페이지·View 수가 여럿인 문서는 chunk 저장(`stellar apply`)에서 그대로 저장됩니다. 텍스트 원본
  (ADR-0059 View v2)은 페이지·View 하나만 지원하므로, 추가한 문서를 그 형식으로 내보내면 기존처럼 unsupported입니다.
- 공개 API: commands에 `page_view`·`snapshot_apply`·`path_edit` 모듈과 `COMMAND_CODEC_VERSION`이 생겼습니다.
  document `DefinitionError::IndexOutOfRange`, `PathError::{UnknownElement, InvalidEdit}`가 추가됐습니다. FFI는 바뀌지
  않습니다.

## Performance

- 새 command는 성능 예산 항목을 바꾸지 않습니다(`config/performance-budgets.yaml` 변경 없음).
- `add-page`·`add-view`·`apply-external-snapshot`의 검증은 문서 레코드 전체를 한 번 만들고 validator를 돌립니다(O(N)).
  Path point 편집·Text 범위 style·인스턴스 교체는 대상 node 하나만 다룹니다.

## Security

- 새 종류도 closed enum이며 registry 밖 종류는 decode에서 거부합니다. 외부 반영은 이전 값 일치와 결과 validator를
  통과해야 적용됩니다. journal의 모르는 command는 실행하지 않고 봉인합니다.

## Migration

- 저장된 문서의 migration은 필요 없습니다. history journal은 schema 1을 그대로 읽고 다음 기록 때 schema 2로 씁니다.
