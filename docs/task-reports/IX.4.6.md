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
  자리 index가 필요할 수 있습니다.
- orphan binding 재연결 command(VIII.8.4 대응)는 아직 없습니다. 현재 orphan 해제는 undo 경로뿐입니다.
