# Semantic Diff/Merge conformance corpus

XXVIII.2.3의 conformance corpus입니다. 결정은 [ADR-0081](../../../docs/adr/ADR-0081-semantic-merge-resolution-and-git-driver.md)
결정 10에 있습니다.

## 내용 정책

- 모든 사례는 합성 fixture(`tests/common/mod.rs`의 `world`와 그 편집)에서 만듭니다. 실제 프로젝트 내용은 없습니다.

## 파일 형식

사례마다 세 파일이며 모두 16진수 텍스트입니다(한 줄 64자, 공백·줄바꿈은 무시).

| 파일 | 내용 |
|---|---|
| `<이름>.input.hex` | `MergeInput` canonical bytes(`SMIN`, base·ours·theirs snapshot) |
| `<이름>.diff.hex` | 기대 base → ours `SemanticDiff` canonical bytes |
| `<이름>.result.hex` | 기대 `MergeResult` canonical bytes(`SMRS`, 병합 snapshot·변경·conflict) |

## 판정

- 구현은 `stellar_semantic_merge::conformance::Implementation`(diff bytes, merge bytes)을 제공합니다.
- 사례마다 diff bytes와 merge 결과 bytes가 기대와 정확히 같아야 통과입니다. merge 오류는 실패입니다.
- 공식 구현(`conformance::official`)은 모든 사례를 통과해야 합니다(`tests/conformance_corpus.rs`).

## 사례

| 이름 | 규칙 |
|---|---|
| `clean-different-objects` | 서로 다른 객체 변경 자동 병합(XIV.10.2) |
| `clean-identical-edit` | 같은 속성을 같은 값으로(XIV.10.3) |
| `property-value` | 같은 속성 다른 값(XIV.10.4) |
| `delete-vs-modify` | 삭제 대 수정(XIV.10.5) |
| `conflicting-reorder` | 같은 child 상충 재정렬(XIV.10.6) |
| `independent-insertions` | 같은 간격 독립 삽입과 key 재배정(XIV.10.6) |
| `token-type-vs-value` | token 타입 대 값(XIV.10.7) |
| `component-property-removed-vs-override` | Property 삭제 대 scene override(XIV.10.7) |
| `drilling-chain-break` | Property 삭제 대 drilling binding(XIV.10.7) |
| `invalid-result-dangling-reference` | 결과 검증 되돌리기(XIV.10.8) |

## 다시 만들기

형식이나 엔진 규칙이 바뀌었을 때만 `STELLAR_CONFORMANCE_BLESS=1 cargo test -p stellar-semantic-merge --test conformance_corpus`로
다시 씁니다. 바뀐 bytes는 리뷰 대상입니다.
