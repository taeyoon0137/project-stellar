# 작업 보고서: XXII.5.2 임시 생성 후 교체 또는 generated marker 아래만 수정

## WBS

- WBS: XXII.5.2
- 작업명: output directory를 temp에 생성 후 replace하거나 generated marker 아래만 수정, 사용자 source를 임의로 덮어쓰지
  않음
- 상태: 완료 보고(`reported_done`). 선행 XXII.5.1이 부분 완료라 원장 판정은 통합 때 정합니다.
- 결정: ADR-0106 결정 9
- 선행: XXII.5.1

## 변경 파일

- `crates/ops/src/export_web_write.rs`(신규): `replace_directory`, `update_marked_regions`, `WriteError`, `WriteReport`,
  생성 manifest(`.stellar-generated`).
- `crates/ops/src/export_web_write/tests.rs`(신규).
- `crates/ops/src/lib.rs`: module 등록.

## 구현 요약

- 디렉터리 교체: 출력 디렉터리가 없거나 비었거나, 생성 manifest가 있고 그 목록 밖 파일이 없을 때만 씁니다. 같은 부모
  아래 임시 디렉터리에 모든 파일과 manifest를 쓰고 sync한 뒤 rename으로 교체합니다. 기존 디렉터리는 백업 이름으로
  옮겼다가 교체 뒤 지우고, 교체가 실패하면 되돌립니다. 목록 밖 사용자 파일·symlink·디렉터리가 아닌 경로는 아무것도
  바꾸지 않고 거부합니다. 이전 생성에만 있던 파일은 `removed`로 알립니다.
- marker 구간 갱신: `stellar:generated:begin <이름>`·`stellar:generated:end <이름>` 주석 줄 사이만 바꿉니다. 구간 밖은
  줄 끝(CRLF 포함)과 공백까지 byte 그대로입니다. 없는 구간·닫히지 않은 구간·겹친 구간·같은 이름 두 번은 거부하고
  파일을 바꾸지 않습니다. 같은 디렉터리의 임시 파일에 쓴 뒤 rename합니다.
- 교체는 rename 두 번(기존 → 백업, 임시 → 출력)이라 그 사이 짧게 출력 경로가 비어 있을 수 있습니다. 표준 라이브러리에
  디렉터리 교환 rename이 없어 이렇게 했고, 실패 시 백업을 되돌립니다.

## 명령과 결과

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops --lib export_web_write` | 통과(4) |
| `cargo clippy -p stellar-ops --all-targets -- -D warnings` | 경고 0 |

검증 기준 대응(user file preservation fixture):

- `user_files_are_never_overwritten`: 생성 디렉터리에 더한 사용자 파일, manifest 없는 사용자 디렉터리, 파일 경로 모두
  거부하고 모든 byte가 그대로임.
- `only_marked_regions_change_in_user_files`: 요청한 구간만 바뀌고 나머지(다른 구간 포함)는 byte 그대로, marker 오류
  4종은 파일 불변.
- `symlinks_are_not_followed`, `fresh_and_regenerated_directories_are_replaced_with_manifest`(임시·백업 디렉터리 잔여 없음).

## 남은 위험

- 이 쓰기 함수를 부르는 CLI·MCP 명령은 아직 없습니다. 문서에서 IR을 만드는 연결(XXII.1.1 재개)과 함께 붙여야 합니다.
- 생성 코드 수정분을 가져오지 않는다는 UI 경고(XXII.5.3)는 이 범위 밖입니다. 생성 파일 marker 문구에는 같은 경고를
  넣었습니다(XXII.5.1).
