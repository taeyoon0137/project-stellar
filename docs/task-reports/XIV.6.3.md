# 작업 보고서: XIV.6.3 삭제와 file mode 보존

## WBS

- WBS: XIV.6.3
- 작업명: update-index 삭제·symlink/executable mode 검증
- 상태: `[x]`
- 선행: XIV.6.2 (`[x]`, `169903b0`)

## 변경 파일

- `crates/git-porcelain/src/index_publish/tests.rs`: mode-only 변경·두 경로 삭제·gitlink·실제 symlink 대상 불변 회귀.
- 구현 계획과 이 보고서. 기존 단일 index-info 구현을 그대로 사용하며 production 코드는 변경하지 않았습니다.

## 명령과 종료 코드

전용 `CARGO_TARGET_DIR="$PWD/target/semantic-index-publish"`를 사용했습니다.

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain index_publish:: -- --nocapture` | 0 — Apple Git에서 15개 통과 |
| 같은 test의 `index_publish::tests::mode_only_changes` 필터, 임시 upstream Git 2.54.0 | 0 — 1개 통과 |
| `cargo fmt -p stellar-git-porcelain` | 0 |

## 환경과 증거

- `DEV-MAC`, Apple Git 2.54.0 및 임시 빌드 upstream Git 2.54.0. Linux 실측은 아닙니다.
- 새 GitFixture에서만 실행했습니다. 사용자 저장소·다른 checkout을 실험 대상으로 쓰지 않았습니다.

## 테스트

- mode-only regular→executable 변경은 기존 blob OID를 유지합니다.
- index의 symlink→regular 변경과 두 경로 삭제 뒤에도 실제 Working Tree 파일 mode/bytes, symlink 종류/대상, 링크 밖 sentinel bytes가 같습니다.
- unrelated gitlink의 mode/commit OID는 그대로 남습니다.
- 선행 corpus의 SHA-256 삭제, 빈 index 결과, 양방향 D/F 전환, executable/symlink 추가도 통과했습니다.
- 검증하지 않은 flags bit는 unrelated 경로여도 진단합니다.

## 성능

- 기존 단일 update-index operation을 재사용합니다. 별도 성능 측정은 없습니다.

## 차이

- 삭제는 `0 <null OID> TAB path NUL`로 모두 먼저 보내고 upsert를 이어 보냅니다. Git이 끝에서 한 번 commit합니다. 직접 index 쓰기는 없습니다.

## 남은 위험

- OS/Git/flags 지원 범위와 결과 미확정 처리는 XIV.6.2 보고서와 같습니다. 조건부 복원 및 제품 semantic 실패 처리까지 완료했다는 의미는 아닙니다.
