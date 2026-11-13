# 차단 보고서: XV.6.3 local override의 current commit·dirty 상태 진단, lockfile exact remote commit 유지

## WBS

- WBS: XV.6.3
- 작업명: local override의 current commit·dirty 상태 진단, lockfile exact remote commit 유지
- 상태: `[!]`
- 선행: XV.6.2(`[~]`), XIV.2.2(`[x]`)

## 실패 선행조건

- 엄격 선행 XV.6.2가 `[~]`입니다.
- XV.6.2(`[~]`)는 local override 대상의 "일반 repo" 판정을 실제 `ProjectProbe`로 하는 project crate 연결이 없어 부분 완료입니다(현재 테스트는 표 기반 probe fake).
- 이 lane(deps_wiring)의 허용 경로는 `crates/dependencies/**`와 project crate의 manifest·lockfile·YAML writer 파일뿐이라 probe·adapter 파일을 만들거나 고칠 수 없습니다.

## 재현 명령

- `grep -E '^\| XV\.6\.2 \|' docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md`

## 필요한 결정과 환경

- project crate 소유 lane이 `LibraryProjectValidator`를 실제 `probe_project`로 구현해 XV.6.2를 `[x]`로 올려야 합니다.
- 필수 환경: `DEV-MAC`.

## 변경되지 않은 파일

- 기능 코드는 변경하지 않았습니다(계획 §1.2 규칙 4). 이 보고서와 계획서 상태 표기만 바꿨습니다.

## 재개 조건

- XV.6.2 `[x]` 후 착수합니다. XIV.2.2 `RepositoryStatus`(HEAD·dirty counts)를 override root에서 읽고, lock 기록 경로(`write_resolved_lockfile`, XV.4.5)에는 원격 exact commit만 남기는지 검증합니다.
