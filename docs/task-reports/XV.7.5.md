# 작업 보고서: XV.7.5 update 실패 시 이전 lock·cache snapshot 유지

## WBS

- WBS: XV.7.5
- 작업명: update 실패 시 이전 lock/cache snapshot을 유지
- 상태: 완료 (`[x]`)
- 선행: XV.7.4(`[x]`)
- 결정: [ADR-0137](../adr/ADR-0137-library-stored-refs-and-materialize-commands.md) 결정 5

## 변경 파일

- `crates/ops/src/library_materialize_tests.rs` — fault 시험 `failed_update_keeps_previous_lock_manifest_and_cache_snapshot`.
- `docs/task-reports/XV.7.5.md`(신규).

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops --lib library_materialize::tests::failed_update_keeps_previous_lock_manifest_and_cache_snapshot` | 0 — 1 passed |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: 없음(DEV-MAC 로컬 시험, 로컬 `file://` 저장소, unix 권한·symlink)

## 테스트

각 실패 뒤 `stellar.yaml`·`stellar.lock.yaml` bytes가 이전과 같고, 같은 cache로 다시 연 library가 완전하며 snapshot commit이 이전 lock
commit(v1)인지 확인합니다.

| fault | 기대 오류 | 결과 |
|---|---|---|
| network: 저장소에 닿을 수 없음(후보 v2는 cache에 없음) | `Resolve` | 통과 |
| cache: entries 디렉터리 쓰기 금지로 entry 공개 실패 | `Materialize` | 통과 |
| schema: `stellar.yaml` 없는 release(일반 Git 저장소) | `Resolve`(매니페스트 `NotAStellarProject`) | 통과 |
| schema: 텍스트 원본이 손상된 release | `Rejected`(entry 검증) | 통과 |
| 사용 프로젝트 참조가 끊기는 후보(승인 없음) | `OrphansBlocked` | 통과 |
| lock 쓰기 실패(lock이 symlink): `stellar.yaml`은 먼저 썼다가 되돌림 | `LockWrite`, `stellar.yaml` = 연 시점 bytes | 통과 |

## 성능

- 해당 없음.

## 차이

- 실패한 후보의 entry가 공용 cache에 남을 수 있습니다(손상 release처럼 materialize 뒤 검증에서 거부된 경우). entry는 immutable·
  content-addressed라 이전 lock이 가리키는 entry를 바꾸지 않고, 시험은 이전 snapshot을 그대로 읽는지로 확인했습니다. 남은 entry 정리는
  cache GC(XV.8) 몫입니다.
- 앱 창의 in-memory snapshot은 lock이 바뀌지 않으므로 다시 로드하지 않습니다(XV.7.4 지문 규칙).

## 남은 위험

- lock 쓰기 뒤 `stellar.yaml` 되돌리기 전에 다른 writer가 `stellar.yaml`을 바꾸면 되돌리지 않고 `RestoreFailed`로 알립니다(두 파일 확인
  필요).
