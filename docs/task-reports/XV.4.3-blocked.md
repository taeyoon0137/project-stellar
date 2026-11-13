# 차단 보고서: XV.4.3 lockfile가 있으면 선택자가 바뀌지 않은 dependency는 network 없이 lock commit 우선 사용

## WBS

- WBS: XV.4.3
- 작업명: lockfile가 있으면 선택자가 바뀌지 않은 dependency는 network 없이 lock commit 우선 사용
- 상태: `[!]`
- 선행: XV.4.2(`[~]`), XV.3.3(`[x]`)

## 실패 선행조건

- 엄격 선행 XV.4.2가 `[~]`입니다. XV.4.2의 엄격 선행 XV.3.4가 `[~]`이기 때문입니다.
- XV.3.4(`[~]`)는 실제 `ProjectProbe`(IV.5.1)·generation loader(V.4.4)를 `LibraryProjectValidator`로 연결하는 project crate adapter가 없어 부분 완료입니다. materialize된 cache entry에는 `.git`이 없으므로 root `.git`을 요구하지 않는 cache entry용 probe 모드도 필요합니다(`crates/project/src/probe.rs`).
- 이 lane(deps_wiring)의 허용 경로는 `crates/dependencies/**`와 project crate의 manifest·lockfile·YAML writer 파일뿐이라 probe·adapter 파일을 만들거나 고칠 수 없습니다.

## 재현 명령

- `grep -E '^\| XV\.(3\.4|4\.2) \|' docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md`

## 필요한 결정과 환경

- project crate 소유 lane이 XV.3.4 adapter(실제 ProjectProbe·storage validator)를 구현해 XV.3.4를 `[x]`로 올리고, 이어서 XV.4.2를 `[x]`로 복원해야 합니다.
- 필수 환경: `DEV-MAC`.

## 변경되지 않은 파일

- 기능 코드는 변경하지 않았습니다(계획 §1.2 규칙 4). 이 보고서와 계획서 상태 표기만 바꿨습니다.

## 재개 조건

- XV.3.4·XV.4.2 `[x]` 후 착수합니다. 재료는 이미 있습니다: XV.3.3 `LibraryEntryStore::lookup`·`open_entry`(offline 재사용), XV.1.2 `LockPin`, 실제 lock parser(`Lockfile::from_yaml`).
