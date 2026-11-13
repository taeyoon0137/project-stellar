# 작업 보고서: XI.1.3 DocumentRevision→SceneRevision 매핑과 snapshot 세대

- WBS: XI.1.3
- 작업명: DocumentRevision→SceneRevision mapping, snapshot generation, scene semantic hash
- 상태: 완료 (`[x]`)
- 선행: II.3.4 (`[x]`, b703259), XI.1.1 (이 lane에서 완료: `3615863`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/scene/src/revision.rs`(신규) — `SceneStamp`, `SceneRevisionMap`(`peek`/`next`/`generate`),
  `SceneRevisionError`, `SceneGenerateError`.
- `crates/scene/src/snapshot.rs` — `SceneSnapshot::semantic_hash`, `SceneSemanticHash`, 내부
  `Fnv1a128` sink.
- `crates/scene/src/lib.rs` — `revision` 모듈 등록.
- `docs/task-reports/XI.1.3.md`(신규), 계획서(XI.1.3 `[ ]`→`[x]`).

## 구현 요약

- **세대 번호**: `SceneRevision`은 snapshot마다 정확히 1씩 증가(첫 값 1)하는 세대 번호입니다.
  `DocumentRevision`은 감소할 수 없고(`DocumentRevisionRegressed`), 같은 document revision에서 여러
  snapshot을 만들 수 있습니다(비동기 결과 반영 등). `u64` 소진은 `Exhausted` 오류입니다.
- **mapping**: 각 snapshot이 자기 `(document, scene)` stamp를 들고 있으므로 이력 표를 두지 않습니다.
  `SceneRevisionMap::generate`는 tree 검증이 끝난 뒤에만 revision을 소비합니다.
- **semantic hash**: revision 두 개를 뺀 canonical 본문 bytes(XI.1.1 codec)를 FNV-1a 128로 hash합니다.
  bytes를 만들지 않고 sink로 바로 흘려 넣습니다. 값만 반영하므로 `Arc` 공유 구조·생성 순서·scene
  revision과 무관합니다. 변경 감지용이며 보안 hash가 아닙니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-scene` | 31 passed, 0 failed |
| `cargo clippy -p stellar-scene --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-scene -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 같은 document revision(5)에서 독립 생성한 두 snapshot: scene revision 1·2, semantic hash 동일. 공유가 끊긴 decode 복제본도 동일, 내용 변경 시 다름(`same_document_revision_yields_same_semantic_hash`) | 통과 |
| scene revision 단조 증가, document revision 감소 거부·상태 불변, `u64` 소진 오류(`scene_revisions_increase_and_document_revisions_never_regress`) | 통과 |
| 잘못된 tree는 revision 미소비(`invalid_tree_does_not_consume_a_revision`) | 통과 |
| FNV-1a 128 공개 test vector(`""`, `"a"`, `"foobar"`)·분할 입력 동일(`fnv1a_128_matches_reference_vectors`) | 통과 |
| revision만 다른 snapshot hash 동일(`semantic_hash_ignores_revisions_only`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- FNV-1a 128은 암호학적 hash가 아니라 악의적 충돌을 막지 못합니다. 변경 감지·fixture 비교 용도로만
  쓰고, 보안 판단이나 저장 주소화에는 쓰지 않아야 합니다.
- semantic hash는 snapshot 전체를 순회합니다(O(node 수)). 매 frame 계산 용도가 아닙니다.
