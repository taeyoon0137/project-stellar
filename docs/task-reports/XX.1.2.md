# 작업 보고서: XX.1.2 내보내기 입력 고정(SceneSnapshot·font/asset revision)

- WBS: XX.1.2
- 작업명: export가 immutable SceneSnapshot과 고정 font/assets revision을 캡처해 작업 중 document 변화와 분리
- 상태: 완료 (`[x]`)
- 선행: XI.1.5 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/export/src/capture.rs`(신규) — `ExportCapture`, `SceneSource`, `ExportRoot`,
  `ResourcePins`(`FontRevision`, `AssetRevision`), `ExportAssetSource`, `EncodedImage`, `ImageMedia`,
  `CaptureError`.
- `crates/export/src/test_fixtures.rs`(신규, test 전용) — 공개 `SceneBuilder`로 만든 View scene fixture.
- `crates/export/src/lib.rs` — 모듈 선언·재노출.
- `docs/task-reports/XX.1.2.md`(신규), 계획서(XX.1.2 `[ ]`→`[x]`).

## 구현 요약

- **scene 고정**: capture는 `SnapshotHandle`(`Arc<SceneSnapshot>`, XI.1.5)을 붙잡습니다. snapshot은
  불변이고 편집은 새 snapshot을 만들 뿐이므로(XI.1.4), 이후 단계가 capture만 읽으면 편집이 섞이지
  않습니다. capture 시점 semantic hash를 함께 기록합니다.
- **resource revision 고정**: `ResourcePins { fonts, assets }`를 capture에 고정합니다. asset 내용은
  `ExportCapture::encoded_image`가 `ExportAssetSource::encoded_image(asset, 고정 revision)`으로만
  조회하며, 구현은 그 revision이 없으면 현재 revision으로 대체하지 않고 `None`을 돌려주는 계약입니다.
  font revision은 텍스트 outline 변환(XX.3.2)이 같은 방식으로 사용합니다.
- **출처 확인**: host가 넘기는 `SceneSource`에 snapshot을 만든 View·target profile을 밝히고,
  요청과 다르면 거부합니다(`ViewMismatch`, `ProfileMismatch`).
- **범위 해석**: View는 snapshot 최상위 node 전체, selection은 선택 node subtree(paint 순서)입니다.
  선택 node의 자손이 함께 선택되면 최상위 하나만 남깁니다. 선택 node의 조상이 건 clip은 범위
  밖이라 `ExportRoot::base_clip_depth`로 표시합니다. scene에 없는 선택 id는 거부합니다.
- **영역**: 보이는 node의 world 그리기 영역(scene bounds XI.3.1: stroke·shadow·blur 포함)을 범위
  안 clip으로 자른 합집합입니다. 숨긴 node와 subtree는 제외합니다. 넓이가 0이면 `NothingVisible`.
- 순회는 모두 명시적 stack이며 재귀하지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-export` | 19 passed, 0 failed (이 작업 신규 10) |
| `cargo fmt -p stellar-export -- --check` | 통과 |
| `cargo clippy -p stellar-export --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| capture 뒤 편집(다시 칠하기·이동·asset 교체) snapshot을 발행해도 capture의 hash·최상위 node·영역·resource가 그대로이고, 새 capture는 편집을 봄(`edits_after_capture_do_not_reach_the_export`) | 통과 |
| export worker가 capture를 읽는 동안 document thread가 200세대 편집 발행, 모든 읽기가 capture 시점 값(`concurrent_edits_never_mix_into_a_running_export`) | 통과 |
| capture 뒤 asset revision 2가 생겨도 고정 revision 1 내용을 읽음, 없는 asset은 대체 없음(`assets_are_read_at_the_pinned_revision`) | 통과 |
| View·profile 불일치 거부(`scene_must_match_requested_view_and_profile`) | 통과 |
| selection 순서·중첩 제거·조상 clip 제외·없는 id 거부(`selection_keeps_topmost_nodes_and_drops_ancestor_clips`) | 통과 |
| clip 밖 overflow는 영역에서 제외, clip이 없으면 포함(`unclipped_overflow_extends_bounds_but_clipped_does_not`) | 통과 |
| 숨김·빈 scene은 `NothingVisible`(`hidden_content_is_not_exported`) | 통과 |
| 깊이 1000 chain을 64 KiB stack thread에서 capture(`deep_trees_are_captured_without_recursion`) | 통과 |

## 차이(작업 지시와 다른 점)

- font catalog·asset store의 revision 타입은 export가 그 crate(text·assets)에 의존할 수 없어(ADR-0019)
  이 crate의 `FontRevision`·`AssetRevision`(host가 발급하는 단조 번호)으로 정의했습니다. host가 자기
  revision을 이 값으로 옮겨 넘깁니다.

## 남은 위험 / 한계

- `ExportAssetSource` 구현이 고정 revision 내용을 실제로 보존하는지는 host(asset store) 책임입니다.
  asset GC가 내보내기 중인 revision을 회수하지 않도록 하는 연결은 export command(XX.5.1)에서
  확인해야 합니다.
- 영역은 scene bounds의 보수적 값(path control point 볼록 껍질)을 쓰므로 곡선 path가 가장자리에
  있으면 실제 그림보다 조금 넓을 수 있습니다.
- missing resource·unsupported capability 사전 진단은 XX.1.3(선행 IX.10.8·XIII.6.7 미완료)의 범위입니다.
