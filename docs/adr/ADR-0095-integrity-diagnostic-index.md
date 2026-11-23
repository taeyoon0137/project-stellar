# ADR-0095: 무결성 진단을 생산자 결과 변환과 scope 단위 증분 index로 모은다

## Status

Accepted (2026-11-23).

- 계획서 XXV.2.1~XXV.2.3, XXV.3.1의 코어 API 부분, XXV.3.3을 구현하면서 정한 결정입니다.
- XXV.3.1의 셸 FFI와 XXV.3.2 native panel은 사용자 방향(2026-11-23, 엔진·기능 먼저, UI는 나중에 다시 설계)에 따라
  이 결정에서 다루지 않습니다.

## Context

- 무결성 진단은 이미 여러 crate에 따로 있습니다.
  - 문서 validator의 미해결 참조(`stellar_document::validate::validate_document`, VI.10.1, III.2.4).
  - Image asset 누락(`ImagePayload::resolve_asset`, VI.8.2).
  - OrphanedBinding(`stellar_bindings::orphan::BindingStore`, VIII.8.1).
  - missing font(`stellar_text::missing_font::collect_missing_fonts`, X.8.4, XIII.6.7).
  - 접근성(`stellar_accessibility::diagnostics::diagnose`, IX.8.5).
  - library 부분 열기(`stellar_dependencies::library_load`, XV.5.5).
  - Target Profile capability(`stellar_target_profile::validate`, IX.10.8).
  - 낮은 schema generation의 메모리 migration(`stellar_project::migration::SchemaDecision`, IX.11.21).
- 생산자마다 진단 타입·심각도 단계·위치 표현이 다릅니다. 진단 패널·CLI·MCP가 한 목록으로 필터하고 page로 읽으려면
  공통 형태와 index가 필요합니다.
- 판정을 다시 구현하면 같은 규칙이 두 곳에 생깁니다(이중 경로 금지).
- 진단 생산자 중 일부는 문서 밖 상태를 읽습니다(font registry, asset store, binding 저장소, library cache, 접근성
  tree를 만드는 생성기). 문서 변경만으로 그 생산자를 다시 실행할 수 없습니다.
- crate 그래프에서 `stellar-cli`는 `stellar-ops`·diagnostics에만 직접 의존합니다(ADR-0005 §8).

## Decision

1. **새 crate `stellar-integrity`.**
   - 허용 직접 의존: core-types, document, commands, bindings, expressions, text, accessibility, dependencies,
     target-profile. `stellar-ops`가 이 crate에 의존하고 `stellar_ops::integrity`로 다시 냅니다.
   - 기존 `stellar-diagnostics`(telemetry·성능 trace)와 목적이 달라 합치지 않습니다. diagnostics는 core-types만
     의존해야 하는 하위 crate라 생산자 crate를 읽을 수 없습니다.
2. **공통 진단 형태.**
   - 종류 10개: `orphaned-binding`, `missing-token`, `missing-font`, `missing-asset`, `missing-library`,
     `missing-node`, `missing-component`, `invalid-accessibility`, `target-unsupported`, `migration-warning`.
   - 심각도 3단계(info < warning < error). 생산자 심각도는 그대로 옮깁니다. 미해결 참조·orphan·missing font·
     library·migration은 경고입니다(문서는 열리고 동작). Target Profile Unsupported·NotRegistered는 오류, Deprecated는
     경고이고 Native·Approximated는 진단으로 옮기지 않습니다.
   - 정체성 key: 문서 → 종류 → source StableId → property path → 생산자 code → subject → origin(생산자 이름). 같은
     key는 같은 진단입니다. key 순서가 전역 순서이며, 안정 ID는 key canonical bytes의 FNV-1a 64bit hash입니다.
   - 진단에는 텍스트 내용·파일 경로·repository URL을 넣지 않습니다. 설명은 생산자가 이미 비민감으로 보장한 Display와
     식별자만 씁니다.
3. **저장 schema v1(`SDIX`).** 진단 목록의 canonical binary 형식입니다(cache·process 경계용). 진단은 Git 원본이 아니라
   원본에서 다시 계산하는 파생 데이터이므로 Git 추적 파일에 쓰지 않습니다. decode는 모르는 값·잘린 입력·상한 초과·남은
   byte를 거부합니다.
4. **scope 단위 증분 재검사.**
   - 진단은 (문서, 생산자) scope 단위로 계산·교체합니다. scope는 계산할 때 footprint(직접 읽은 객체, 관심 속성 key,
     node 삽입 반응)를 남깁니다.
   - command 결과 `ChangeSet`(VII.1.4)을 footprint와 대조해 걸린 scope만 다룹니다. 걸린 scope가 없으면 checker 실행
     0이고 문서 레코드도 만들지 않습니다.
   - **내장 checker**(문서 레코드만 읽음: 미해결 참조, Image asset)는 그 자리에서 다시 계산합니다.
   - **외부 생산자**(문서 밖 상태를 읽음)는 걸리면 stale로 표시해 보고하고, 호출자가 생산자를 다시 실행해 결과를
     넣으면 풀립니다. 그 사이 기존 진단은 남습니다.
   - 편집 전용 공통 필드(잠금·표시 이름)는 "모든 key" 관심에서도 빼서 어떤 scope도 다시 계산하지 않습니다.
   - 문서 전체에서 새 진단이 생길 수 있는 외부 생산자(missing font, 접근성, 호출자가 모은 미해결 token 참조, library
     resource 참조, Target Profile)의 footprint는 "모든 key"와 node 삽입입니다. 지금 진단이 있는 node만 보면 진단이 없던
     node에 새 문제가 생기는 편집(Text font 변경, 숨김 subtree 표시, 새 token 참조)을 놓칩니다. binding orphan은 binding
     저장소가 정한 target·source node만 봅니다.
   - 미해결 참조 checker의 footprint는 validator가 읽는 참조 위치(Frame clip path, Boolean operand, Mask·Clip
     source)와 같은 목록을 씁니다. 목록이 어긋나면 무작위 command 열에서 증분 결과와 전체 재검사를 비교하는 test가
     실패합니다.
5. **remediation은 두 형태만.**
   - 명시 navigation(node, 속성, 프로젝트 설정 절)과 일반 CommandTransaction입니다. 계획 타입이 이 두 variant만 가진
     closed enum이라 script·외부 명령·plugin 형태는 만들 수 없습니다.
   - action은 inventory 7개입니다: `reveal-source`, `reveal-property`, `open-dependencies`, `open-fonts`,
     `open-target-profiles`, `open-migration`, `remove-dangling-modifier`. 숫자 값은 schema의 일부입니다.
   - command action은 `remove-dangling-modifier` 하나입니다. source가 없는 Mask·Clip modifier를 `RemoveModifier`
     요청 하나의 transaction(출처 사용자)으로 지웁니다. 실행은 다른 편집과 같은 command 경로라 검증·잠금·undo가
     그대로 적용됩니다.
   - Boolean operand 하나가 없을 때 modifier 전체를 지우면 남은 operand 결과까지 잃으므로 navigation만 줍니다.
   - OrphanedBinding 재연결·삭제(VIII.8.4)는 아직 CommandTransaction이 아닌 binding 전용 command라 inventory에
     넣지 않습니다. binding command가 CommandTransaction으로 들어오면 action을 더합니다.
6. **노출: 코어 API, CLI, MCP. 셸 FFI는 두지 않음.**
   - `DiagnosticIndex::page(filter, cursor, limit)`: 종류·최소 심각도·문서(또는 프로젝트 단위만)·source 필터,
     key 순 cursor pagination. page 크기 상한은 10,000입니다.
   - 문서·종류 필터는 map 범위 탐색이라 page 비용이 page 크기에 비례합니다. 종류×심각도 개수와 문서별 개수는
     삽입·삭제 때 갱신합니다.
   - cursor는 마지막 key와 index revision이고, index가 바뀌면 거부합니다. CLI cursor는 프로젝트 revision token도
     함께 담아 저장 프로젝트가 바뀌면 거부합니다.
   - `stellar diagnose`와 MCP `document.diagnostics`(읽기 권한, `document.validate`와 같은 제공 범위)가 같은 수집 함수
     `stellar_ops::integrity::diagnose_snapshot`과 같은 index 필터를 씁니다.
   - FFI는 사용자 방향에 따라 UI 재설계 때 이 코어 API 위에 만듭니다.
7. **headless 수집 범위.**
   - 저장 snapshot만으로 계산할 수 있는 것은 모든 문서의 미해결 참조와 메모리 migration 상태입니다.
   - asset store 조회·font registry·접근성 tree·Target Profile 검사·library cache·binding 저장소는 headless snapshot에
     입력이 없어 실행하지 않습니다. 그 생산자를 snapshot에서 만들 수 있게 되면 같은 `diagnose_snapshot`에 연결합니다.

## Alternatives

- **생산자별 진단을 그대로 두고 UI·CLI가 각각 모음**: 필터·pagination·remediation 규칙이 소비자마다 생깁니다.
- **index가 생산자 판정을 직접 구현**: 판정이 두 곳에 생겨 규칙이 어긋납니다(이중 경로 금지).
- **문서 변경마다 전체 재검사**: XXV.2.3(무관한 편집의 재계산 0)을 만족하지 못하고 대형 문서에서 편집 지연을 늘립니다.
- **node 단위 dependency graph**: 접근성(조상·형제 문맥)과 Target Profile(모든 속성)은 node 단위로 나눌 수 없습니다.
  scope 단위 footprint가 정확도와 비용의 균형입니다.
- **remediation에 binding 전용 command 허용**: XXV.3.3이 일반 CommandTransaction 또는 navigation만 허용합니다.
- **stellar-diagnostics에 합침**: crate 그래프에서 diagnostics는 core-types만 의존할 수 있습니다.

## Consequences

- 새 진단 생산자는 `sources`에 변환 함수 하나와 footprint 규칙을 더하면 index·필터·page·CLI·MCP를 그대로 씁니다.
- 외부 생산자의 stale 표시는 호출자가 다시 실행해야 풀립니다. 앱 runtime 연결(문서 thread가 command 뒤
  `apply_changes`를 부르고 stale 생산자를 다시 실행)은 UI 재설계 때 합니다.
- headless `stellar diagnose`는 미해결 참조와 migration 상태만 보고합니다(결정 7).
- `remove-dangling-modifier` 계획은 `stellar apply` 입력 형식(`stellar-apply` v1)에 `remove-modifier` 항목이 없어 CLI에서
  바로 실행할 수 없습니다. 코어 API와 앱 command 경로에서는 실행됩니다.

## Compatibility

- 파일 포맷: Git 원본·storage chunk를 바꾸지 않습니다. 진단 schema v1은 새 파생 데이터 형식입니다.
- 공개 API: 새 crate `stellar-integrity`, `stellar_ops::integrity`, CLI 명령 `diagnose`, MCP tool
  `document.diagnostics`가 생깁니다. 기존 명령·tool은 바뀌지 않습니다.
- FFI: 바뀌지 않습니다(`stellar_ffi.h` drift 없음).

## Performance

- 무관한 편집: checker 실행 0, 레코드 변환 0(대조 비용은 scope 수 × change set 크기).
- 관련 편집: 걸린 내장 checker 하나당 문서 레코드 변환 한 번과 validator 한 번(문서 크기에 선형).
- index: 삽입·삭제 O(log n), page O(log n + page 크기)(문서·종류 필터). 100만 진단 page 순회 test가 있습니다.
- `config/performance-budgets.yaml`은 바꾸지 않습니다. 진단 재검사는 편집 응답 경로 밖(문서 thread 뒤)에서 돌린다는
  전제이며, 앱 연결 때 예산 항목을 정합니다.

## Security

- 진단에는 텍스트 내용·파일 경로·repository URL이 없습니다(II.3.2, XV.1.4).
- remediation은 closed enum command와 navigation만 실행 계획으로 만들 수 있어 임의 코드 실행 경로가 없습니다.
- CLI·MCP 진단 읽기는 파일을 쓰지 않습니다(test가 프로젝트 hash 불변을 확인).

## Migration

- 기존 데이터 migration은 없습니다. 진단은 저장하지 않고 다시 계산합니다.
