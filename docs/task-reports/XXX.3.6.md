# 작업 보고서: XXX.3.6 프리뷰 상태의 세션 메모리 보관

- WBS: XXX.3.6
- 작업명: 프리뷰 상태·진행 중 motion·입력 로그를 세션 메모리에만, 저장·recovery snapshot 제외.
- 상태: 부분 완료 (`[~]`) — 프리뷰 상태·motion·입력 로그·trace가 `PreviewSession` 안에만 있고 문서를 공유 참조로만 읽어 save bytes가 바뀌지 않음은 확인했습니다. recovery snapshot(XIII.8.2) 필드 검사는 그 형식이 생긴 뒤입니다.
- 결정: ADR-0094 결정 6.6
- 선행: XXX.3.3, XIII.8.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/prototype/tests/preview_memory.rs`(신규). 구조는 XXX.3.3 commit의 `session.rs`.

## 구현 요약

- 세션은 `&ComponentLibrary`·`&BindingSet`·`&MotionSet`만 받습니다. 문서를 바꾸는 경로가 타입으로 없습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-prototype` | 통과(lib 13, 통합 test 11개 파일 24개). |

시험:

- `preview_memory.rs::preview_never_writes_the_document`: 프리뷰 조작 뒤 library·binding·MotionSpec·hint bytes 동일, 입력 로그는 세션에 있음.

## 차이(작업 지시와 다른 점)

- recovery snapshot 형식(XIII.8.2)이 아직 없어 그 필드 0 검사는 하지 않았습니다.

## 남은 위험 / 한계

- recovery snapshot이 생기면 프리뷰 필드 0 test를 더해야 합니다.
