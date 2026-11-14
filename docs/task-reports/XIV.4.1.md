# 작업 보고서: XIV.4.1 Staged·Unstaged semantic status 분류

- WBS: XIV.4.1
- 작업명: HEAD→Index diff를 Staged, Index→WorkingTree diff를 Unstaged로 계산
- 상태: 완료 (`[x]`)
- 선행: XIV.3.10(`[x]`), XIV.2.5(`[x]`), XIV.2.7(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-diff/src/status.rs`(신규) — `StatusArea`, `SnapshotSide`, `SemanticStatus::classify`.
- `crates/semantic-diff/src/lib.rs` — `pub mod status;`.
- `docs/task-reports/XIV.4.1.md`(신규), 계획서(XIV.4.1 `[ ]`→`[x]`).

## 구현 요약

- `SemanticStatus::classify(head, index, working)`은 `diff(HEAD, Index)`를 staged, `diff(Index,
  WorkingTree)`를 unstaged로 둡니다. 두 영역 모두 XIV.3.8 canonical 순서의 `SemanticDiff`이며
  `apply(staged, HEAD) == Index`, `apply(unstaged, Index) == WorkingTree`가 성립합니다.
- 기준은 Git index입니다(§26.3: index가 staged의 유일한 원본). WorkingTree는 디스크에 저장된 파일
  기준(XIV.2.7)이라 저장 대기 메모리 상태는 입력이 아닙니다. unborn HEAD는 빈 snapshot으로 넘기면
  Index 전체가 staged 추가가 됩니다.
- `StatusArea`(staged=1, unstaged=2)는 안정 code를 가집니다. `SnapshotSide`(Head·Index·WorkingTree)는
  이후 실패 보고(XIV.4.5)와 값 층(XIV.4.2)에서 씁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-diff` | unit 48 passed, fuzz 통합 4 passed, 0 failed |
| `cargo clippy -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-semantic-diff -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **부분 staged fixture**: 같은 chunk의 rect 11에서 크기는 staged, 표시 여부는 unstaged; ellipse 14 추가는 staged, fragment 13 삭제는 unstaged. 세 snapshot은 View subtree chunk bytes 경유 재구성과 동일. staged = [SetProperty(11.RECT_SIZE), AddObject(14)], unstaged = [SetProperty(11.VISIBLE), RemoveObject(13)] 정확, 각 영역 적용이 다음 snapshot 재현, staged 값이 Index 값(`partial_staged_fixture_is_classified_exactly`) | 통과 |
| unborn HEAD → Index 전체 staged AddObject, 변경 없는 트리 clean(`unborn_head_makes_every_index_object_staged_and_clean_tree_is_clean`) | 통과 |
| `StatusArea` code round-trip(`area_codes_round_trip`) | 통과 |

## 차이(작업 지시와 다른 점)

- semantic-diff는 crate 그래프(II.2.8)상 git-porcelain·project에 의존할 수 없어 Git에서 세 snapshot을
  직접 읽지 않습니다. fixture는 XIV.3.1과 같은 방식(View subtree chunk bytes 직렬화·복원으로
  "materialize된 blob → loader → snapshot" 모사)으로 HEAD·Index·WorkingTree를 만듭니다.

## 남은 위험 / 한계

- 실제 Git(materialize → `SavedProjectLoader` → `SnapshotBuilder`) 연결은 project crate와
  semantic-diff를 함께 쓰는 crate가 필요합니다. 현재 허용 의존 표(`tools/xtask/src/crate_graph.rs`)에는
  project → semantic-diff edge가 없어 이 연결 위치는 crate 그래프 결정(ADR)이 먼저 필요합니다.

## 후속 수정 (리뷰 반영)

- **Git 연결 없음(리뷰 minor, 사실 기록)**: XIV.4.1·XIV.4.2·XIV.4.3(semantic 쪽)의 부분 staged fixture는 메모리의
  `DocumentRecords`를 View subtree chunk bytes로 직렬화·복원해 만든 것이고, 실제 Git index(XIV.2.5)·작업
  트리(XIV.2.7)에서 읽은 snapshot이 아닙니다. 실제 연결(materialize → `SavedProjectLoader` → `SnapshotBuilder` →
  `classify`)을 둘 crate가 crate 그래프에 없고, 그 결정은 ADR이 필요해 이 lane에서 하지 않았습니다.
- 상태 `[x]` 유지 근거: 이 행의 산출물은 분류 계산(HEAD→Index=staged, Index→WorkingTree=unstaged)이고 검증 열은
  "부분 staged fixture 분류 정확"입니다. 분류 계산은 실제 코드이며 fixture도 저장 형식(chunk bytes)을 거칩니다.
  XIV.5.5는 산출물 자체(serializer)가 없어 `[~]`이고, 이 행은 산출물이 있고 통합 연결만 없다는 점이 다릅니다.
- 후속 필요: 통합 crate 위치를 ADR로 정한 뒤 실제 Git 저장소에서 HEAD·Index·WorkingTree를 읽어 같은 부분
  staged 분류를 확인하는 종단 테스트를 추가해야 합니다.
