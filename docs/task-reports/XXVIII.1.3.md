# 작업 보고서: XXVIII.1.3 의미 모델과 Git diff·merge 규칙 명세

- WBS: XXVIII.1.3
- 작업명: semantic model과 Git diff/merge 규칙을 별도 normative section으로 작성
- 상태: 완료 (`[x]`)
- 선행: Phase XIV(phase 단위 선행 — 완화 규칙 적용. 명세 대상 코드인 XIV.3.1\~XIV.3.10, XIV.10.1\~XIV.10.6,
  XIV.2.3\~XIV.2.7, XIV.11.1·XIV.11.2·XIV.11.4가 기준 commit에서 `[x]`이고 `semantic-diff`·`semantic-merge`·
  `git-porcelain` crate가 있습니다. 미구현 행은 명세에 "미구현"으로 적었습니다)

## 필수 실행 환경

- `DEV-MAC` (문서 작업, 로컬 검증).

## 변경 파일 목록

- `docs/spec/stellar-semantic-model.md`(신규) — 의미 모델·diff·merge·Git 연결 규범 명세.
- `docs/spec/stellar-format.md` — §1 "다루지 않는 것"의 XXVIII.1.3 참조를 새 명세 링크로 바꿈.
- `docs/spec/stellar-format-compatibility.md` — §2 registry 표의 "XXVIII.1.3 범위"를 새 명세 링크로 바꿈.
- `docs/task-reports/XXVIII.1.3.md`(신규), 계획서(XXVIII.1.3 `[ ]`→`[x]`).
- 코드 변경 없음. 문서만 변경했습니다.

## 구현 요약

- **근거**: 기준 commit `24728e3`의 `semantic-diff`(snapshot, value, keys, build, change, diff, codec, apply),
  `semantic-merge`(model, merge), `document`(records, record_codec, definitions, modifier, node_kind, validate),
  `core-types`(property, order_key, order_key_merge_fixtures, color), `git-porcelain`(snapshot, classify,
  conflict, conflict_snapshot), `project`(conflict_open, scaffold)를 읽고 옮겨 적었습니다.
- **§2 문서 의미 레코드**: 형식 명세 §1이 미룬 node 레코드·`SubtreeRecords`·View 분할·`DocumentDefinition`·
  `ModifierStack`(12종 payload 길이·검증)과 NodeKind·ModifierKind·BooleanOp·BlendMode registry, 문서 불변식,
  저장되지 않는 의미 객체 목록.
- **§3 Semantic Snapshot**: 객체·ObjectKind code·Origin·PropertyPath 순서·SemanticValue 8종·key 표(재사용 key와
  전용 대역 `0xE0xx`~`0xEBxx`), 입력별 객체·속성 펼치기 표.
- **§4 diff**: StableId 대응, 교체, Move/Reorder, 속성 merge-join, 변경 종류 10종 분류 규칙, 출처 4종,
  canonical 정렬 key, apply 목록 검증·단계·stale 오류, inverse.
- **§5 교환 인코딩**: `SSNP`·`SDIF`·`SMIN`·`SMRS` 형식, 최소 항목 크기, decoder 오류.
- **§6 merge**: 대상 단위 그룹, 객체별 판정 순서, conflict 6종 분류 표, 기록 규칙, 후처리 3단계(고아 방지,
  cycle, 독립 삽입 OrderKey 재배정), 정렬, 알려진 위험 M1~M7.
- **§7 Git**: snapshot 원천 5종, 경로 분류, unmerged stage 수집·`XY` 표, conflict side 구성, 저장 형식과 Git
  병합의 관계, 미구현 목록.
- **부록 A·B**: merge corpus(semantic-merge 테스트 20개, `conflict_matrix` 14행, OrderKey fixture 2개와 그 테스트
  5개)와 diff corpus(semantic-diff 단위 테스트 45개, fuzz 테스트 4개)를 전부 명세 절에 연결했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `git merge --ff-only 24728e3` | `Already up to date.`, HEAD `24728e3` |
| `cargo test -p stellar-semantic-diff -p stellar-semantic-merge` | semantic-diff unit 45 passed, `semantic_diff_fuzz` 4 passed, semantic-merge unit 20 passed, 0 failed |
| `cargo test -p stellar-core-types order_key` | 21 passed, 0 failed |
| `cargo test -p stellar-git-porcelain conflict` | 12 passed, 0 failed |
| `cargo test -p stellar-git-porcelain classify` | 3 passed, 0 failed |
| `cargo test -p stellar-project --test conflict_project_open` | 2 passed, 0 failed |
| `cargo test -p stellar-document --lib` | 316 passed, 0 failed |
| 명세·코드 일치 검사 스크립트(scratchpad `check_semantic_spec.py`) | 337개 검사 통과, 0 실패, backtick 식별자 389개 존재 확인, 종료 코드 0 |
| 일치 검사 음성 테스트(key 값·ConflictKind code·부록 테스트 이름·함수 이름을 일부러 틀린 사본) | 주입한 4개 오류를 모두 실패로 보고, 종료 코드 1 |
| scratch 실행 파일 `semprobe`(작업 트리 밖, path 의존) | 아래 "검사" 표의 재현 항목 P1~P14 |
| `git diff --check` | 출력 없음 |
| `cargo xtask validate-task-report docs/task-reports/XXVIII.1.3.md` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| 전용 key 45개·재사용 key 표 값이 `keys.rs`·`frame.rs`·`instance.rs`·`modifier_metadata.rs`·`image.rs` 상수와 같음 | 통과 |
| NodeKind 7·ModifierKind 12·BooleanOp 4·BlendMode 6·SemanticChangeKind 10·Provenance 4·ConflictKind 6·ObjectKind·SemanticValue tag 표가 코드와 같음 | 통과 |
| magic `SSNP`·`SDIF`·`SMIN`·`SMRS`, version 1, 최소 항목 크기(4·16·23·25·2·22) | 통과 |
| `unmerged_xy` 8가지 조합 표 | 통과 |
| 부록 A: merge 테스트 20개, matrix 14행, fixture 2개·fixture 테스트 5개가 모두 표에 있음 | 통과 |
| 부록 B: diff 단위 테스트 45개(module 이름 포함), fuzz 테스트 4개가 모두 표에 있음 | 통과 |
| P1 cycle 되돌림 뒤 고아(§6.10 M1): conflict는 102·103 conflicting-reorder 둘, candidate에 P101 없음, 103이 고아 | 재현됨 |
| P2 이동하지 않은 cycle 구성원 203·204도 conflicting-reorder, 세 위치 동일(§6.7) | 재현됨 |
| P3 고아 방지는 parent 11 삭제만 되돌리고 13 삭제는 유지(§6.6) | 재현됨 |
| P4 delete vs replace·replace vs delete → DeleteVsModify, 다른 replace·다른 add → InconsistentDefinition, 같은 replace·add → conflict 없음(§6.4) | 재현됨 |
| P5 Text→Text 대 Text→Uint → TypeChangeVsValueChange, 출처 Document(ours)(§6.4, §6.5) | 재현됨 |
| P6 `FramePayload::decode`·`ModifierStack::decode`가 `-0.0` bit를 받고 재인코딩 bytes가 다름, semantic snapshot codec은 `InvalidValue { field: "value.float" }`(§2.3) | 재현됨 |
| P7 `DocumentDefinition::decode`가 페이지 id 중복을 받음(§2.5) | 재현됨 |
| P8 kind 7 → `UnsupportedNodeKind`, kind 99 → `UnknownNodeKind`, NIL root 빈 subtree 통과(§2.2~§2.4) | 재현됨 |
| P9 같은 변경 두 번: `SemanticDiff` decoder는 `NonCanonicalOrder`, MergeResult decoder는 수용(§5.3, §5.4) | 재현됨 |
| P10 conflict 순서를 뒤집은 MergeResult도 decode됨(§5.4) | 재현됨 |
| P11 kind 같고 origin만 다른 token → remove-token + add-token(§4.1) | 재현됨 |
| P12 token TOKEN_TYPE 변경 대 TOKEN_VALUE 변경 → conflict 없음(§6.10 M3) | 재현됨 |
| P13 Component Property 삭제 대 instance override 추가 → conflict 없음(§6.10 M4) | 재현됨 |
| P14 subtree decoder가 root 뒤 node 순서를 검사하지 않고 다시 합치면 같은 레코드(§2.4) | 재현됨 |

## 차이(작업 지시와 다른 점)

- "merge corpus"라는 별도 fixture 디렉터리는 저장소에 없습니다. 검증 기준 "merge corpus와 spec mapping
  완전"은 semantic-merge의 테스트·`conflict_matrix` 행·OrderKey merge fixture를 merge corpus로 보고 부록 A에
  전부 연결하는 것으로 해석했습니다. diff 쪽 테스트도 부록 B에 연결했습니다.
- node payload 종류별 내부 bytes(Frame·Shape·Path·Text·Image)는 이 명세에 넣지 않고 "다루지 않는 것"으로
  남겼습니다. 이 작업의 대상은 의미 모델(snapshot)과 diff·merge 규칙이며, 형식 명세가 미룬 레코드
  컨테이너(node 레코드·SubtreeRecords·DocumentDefinition·ModifierStack)까지 포함했습니다.
- 발견한 위험(M1 등)은 코드 수정 없이 기록만 했습니다. 이 작업의 허용 경로는 `docs/spec/`뿐입니다.

## 남은 위험 / 한계

- M1: merge 후처리 순서(고아 방지 → cycle) 때문에 cycle 되돌림 뒤 candidate에 고아가 남을 수 있고 그 parent
  삭제 conflict가 없습니다. XIV.10.8(merged snapshot full validation)이 없어 이 결과가 그대로 나옵니다.
  `semantic-merge` 수정이 필요합니다(후처리 반복 또는 validator 연결).
- M3·M4·M5: 아키텍처 §27.1의 token 타입 대 값, Component Property 삭제 대 사용처 추가, 공급자 삭제 대 참조
  추가 conflict가 없습니다(XIV.10.7 `[ ]`).
- M7: MergeResult decoder가 변경 반복·conflict 순서·merged 일치를 검사하지 않습니다.
- 후속 추적: M3·M4·M5는 계획 XIV.10.7 `[ ]`, M2는 XIV.10.8 `[ ]`이 다룹니다. M1의 수정(cycle 되돌림 뒤
  고아 방지 재실행 또는 빠진 parent 삭제 conflict 생성)과 M7의 decoder 검사를 다루는 행은 계획서에 없습니다.
  XIV.12.3·XXIII.2.4(candidate full validation)가 구현되면 M1의 고아 candidate는 저장 전에 거부되지만, 빠진
  parent 삭제 conflict는 여전히 만들어지지 않습니다. M1·M7의 후속 WBS 행 추가가 필요합니다. 계획 행 추가는 이
  작업 범위 밖이라 하지 않았습니다.
- 명세는 기준 commit 동작을 기록합니다. 코드와 명세의 일치를 강제하는 CI 검사는 없습니다(일치 검사
  스크립트는 scratchpad에 있고 저장소에 넣지 않았습니다).
- Git 쪽 semantic status(XIV.4), staging(XIV.5~XIV.7), conflict→객체 mapping(XIV.11.3), 해결(XIV.12)이 구현되면
  §7을 갱신해야 합니다.
