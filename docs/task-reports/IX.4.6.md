# 작업 보고서: IX.4.6 chain 일부 삭제·타입 변경 시 상위 chain 전체 orphan 표시

- WBS: IX.4.6
- 작업명: chain 일부 삭제·타입 변경 시 상위 chain 전체를 orphan 상태로 표시하되 저장에서 삭제하지 않음
- 상태: 완료 (`[x]`)
- 선행: IX.4.5(이 lane에서 `[x]`), VIII.8.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/drilling.rs` — `OrphanCause::PropertyRetyped`(tag 0x02)·`UpstreamOfBreak`(tag 0x03)와
  codec, `DrillingSet::orphan_chain`(직접 끊긴 binding + 상위 chain 전파), `DrillingSet::affected_chain`.
- `crates/document/src/component/property_commands.rs` — `PropertyCommand::Retype`, inverse 전용 `RestoreRecord`,
  Delete·Retype의 상위 chain 전파, 전파 orphan까지 정확히 해제하는 revive 검사, IX.1.3 테스트 기대값 갱신(전파 반영),
  IX.4.6 fixture 테스트.
- `docs/task-reports/IX.4.6.md`(신규), 계획서(IX.4.6 `[ ]`→`[x]`).

## 구현 요약

- **break point**: 삭제는 그 property를 직접 쓰는 모든 live binding, 타입 변경은 그중 바뀐 타입으로 다시 검증해 실패하는
  binding이 직접 끊긴 binding입니다(`PropertyDeleted`·`PropertyRetyped { previous, current }` + `Source`/`Boundary(k)`).
- **상위 chain 전파**: 끊긴 binding `X`(owner `O`, source `s`)로 값을 넘기는 binding(경계 chain에 (`O`, `s`)가 있는 binding)을
  `UpstreamOfBreak { origin }` + 연결 경계 `Boundary(k)`로 표시하고 반복(worklist, 방문 집합)으로 위로 올라갑니다. 이미
  orphan인 binding은 덮어쓰지 않습니다. 아래쪽(안쪽) binding은 live로 남습니다.
- **마지막 유효 타입**: 끊기기 전 target 타입입니다. 타입 변경에서 target이 바뀐 property면 이전 타입을 씁니다.
- **저장**: binding은 지우지 않고 orphan 정보(원인·break point·마지막 유효 타입)를 `DrillingSet` bytes에 저장합니다.
- **undo**: Delete → `Restore`, Retype → `RestoreRecord`가 그 command가 표시한 orphan(직접 + 전파)만 해제합니다. 전파
  orphan은 origin이 같은 revive 목록에 있을 때만 해제를 허용합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 338 passed, 통합 7+1+1, doc 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| IX.4.5로 만든 3단계 chain(L0·L1·L2)에서 중간 property 삭제 → L1 `Boundary(0)`·L2 `Source` 직접, L0 전파(origin L1), `affected_chain(L1)` = [L0, L1], binding 3개·orphan 3건 저장 round-trip, undo 후 bytes 동일; 가장 안쪽 삭제 → L2 break point, L1·L0 전파(origin L2)(`partial_chain_delete_orphans_the_whole_upstream_chain`) | 통과 |
| C1 property String→Number → L1 `Source`·L0 `Boundary(0)` 직접(`PropertyRetyped` 이전·새 타입 정확, L0 마지막 유효 타입 String), 아래쪽 L2 live, 저장 round-trip·library 저장 검증 통과, undo 정확; C2 property String→Optional → L2만 직접, L1·L0 전파, redo 시 같은 orphan 재현; 타입 안 맞는 default·상태 머신 참조 Retype 거부(`type_change_orphans_broken_links_and_upstream_but_keeps_downstream_live`) | 통과 |
| IX.1.3 orphan 테스트(전파 반영 갱신) 회귀 | 통과 |

## 차이(작업 지시와 다른 점)

- 타입 변경 command가 계획서 IX.1.3 목록(생성·rename·default·삭제)에 없어 이 작업에서 `Retype`을 추가했습니다(타입과
  새 default를 함께 받음).
- IX.1.3 테스트의 기대값을 상위 chain 전파에 맞게 갱신했습니다(IX.1.3 때는 직접 사용처만 orphan).

## 남은 위험 / 한계

- 전파 탐색은 끊긴 binding마다 모음 전체를 훑습니다(binding 수 B, 깊이 d일 때 최악 O(B²·d)). 대규모 모음에서는 경계 값
  자리 index가 필요할 수 있습니다. (후속 수정에서 해소했습니다. 아래 "전파 탐색 비용" 참고.)
- orphan binding 재연결 command(VIII.8.4 대응)는 아직 없습니다. 현재 orphan 해제는 undo 경로뿐입니다.

## 후속 수정 (리뷰 반영)

### orphan 정보 디코딩 검사

`Fix: binding 구조와 맞지 않는 drilling orphan 정보 디코딩 거부` 커밋에서 고쳤습니다.

- **문제**: `DrillingSet::decode`가 orphan 정보를 binding과 대조하지 않았습니다. `BreakPoint::Boundary(k)`의 `k`가
  binding 경계 수를 넘거나, 상위 chain orphan(`UpstreamOfBreak`)이 자기 자신을 origin으로 두거나 `Source` 위치에 있거나,
  직접 원인(삭제·타입 변경)의 property가 break point 위치의 property와 달라도 받아들이고 canonical bytes로 그대로
  다시 썼습니다.
- **변경**: 디코딩할 때 각 orphan 정보를 그 binding과 대조합니다. `Boundary(k)`는 `k < 경계 수`, 상위 chain orphan은
  `Boundary` 위치이고 origin ≠ 자기 자신, 직접 원인 property는 `Source`면 binding owner·source, `Boundary(k)`면
  `chain[k]`와 같아야 합니다. 어기면 `DrillingError::Malformed`입니다. command가 만드는 orphan 정보(삭제·타입
  변경·전파)는 모두 이 규칙을 지키므로 기존 round-trip 테스트가 그대로 통과합니다.
- **검사하지 않는 것**: 상위 chain orphan의 origin binding이 모음에 있는지·orphan인지는 디코딩에서 검사하지 않습니다.
  origin binding은 `DrillingCommand::Remove`로 따로 지울 수 있어 정상 편집으로도 없는 origin을 가리키는 상태가
  생기고, 이를 거부하면 저장 → 로드가 깨집니다. 이 상태를 막으려면 origin 삭제 정책(전파 orphan 함께 정리 또는 삭제
  거부)을 먼저 정해야 합니다.
- **검증**: `decode_rejects_orphan_information_inconsistent_with_its_binding`을 먼저 추가해 수정 전 범위를 넘는 break
  point가 디코딩되는 것(실패)을 확인했습니다. 수정 후 정상 orphan 2종은 round-trip하고 잘못된 orphan 7종은 모두
  `Malformed`입니다.

| 명령(후속 수정) | 결과 |
|---|---|
| `cargo test -p stellar-document --lib decode_rejects_orphan_information`(수정 전) | 1 failed(경우 0이 디코딩됨) |
| `cargo test -p stellar-document` | unit 353 passed, 통합 7+1+1, doc 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

### 전파 탐색 비용

`Fix: drilling orphan 전파 탐색을 경계 자리 색인으로 선형화` 커밋에서 고쳤습니다.

- **문제**: `orphan_chain`이 작업 항목을 꺼낼 때마다 모음 전체 binding의 경로를 다시 해석했습니다(instance payload
  디코딩 포함). 넓은 chain을 끊는 삭제·타입 변경 한 번이 O(B²·d)였습니다.
- **변경**: 시작할 때 binding마다 경로를 한 번 해석해 (경계 컴포넌트, 경계 chain property) → (binding, 처음 지나는 경계,
  이전 타입) 색인을 만들고, 전파는 색인 조회로 합니다. 목록은 id 오름차순이고 binding별로 처음 지나는 경계만 넣어 이전
  `position` 결과와 같습니다. 탐색 중 library와 binding 목록은 바뀌지 않으므로 결과(표시 대상·원인·break point·마지막
  유효 타입)는 같습니다. 경로 해석은 최대 2B번(색인 B + 표시한 binding의 target 타입 조회)이고 전체 O(B·d)입니다.
- **검증**: 테스트 build에만 있는 `resolve_path` 호출 수 계수기(`RESOLVE_CALLS`, `cfg(test)`)를 두고
  `upstream_propagation_resolves_paths_linearly_in_binding_count`(32단계 직선 chain의 가장 안쪽 property 삭제)를 먼저
  추가했습니다. 수정 전에는 경로 해석이 527회라 상한 2B = 64를 넘어 실패했습니다. 수정 후에는 64회 이하로 통과합니다.
  전파 결과(32개 전부 orphan, origin·break point·마지막 유효 타입)와 기존 IX.1.3·IX.4.6 테스트도 그대로 통과합니다.

| 명령(후속 수정) | 결과 |
|---|---|
| `cargo test -p stellar-document --lib upstream_propagation_resolves`(수정 전) | 1 failed("경로 해석 527회(binding 32개)") |
| `cargo test -p stellar-document` | unit 354 passed, 통합 7+1+1, doc 4 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo clippy -p stellar-document --features test-support -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |
