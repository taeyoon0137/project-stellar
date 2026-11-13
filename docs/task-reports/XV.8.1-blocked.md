# 차단 보고서: XV.8.1 최근 사용 시각·lockfile reference set 기준 cache GC 후보 계산, 삭제 임계치는 config

## WBS

- WBS: XV.8.1
- 작업명: 최근 사용 시각·lockfile reference set 기준 cache GC 후보 계산, 삭제 임계치는 config
- 상태: `[!]`
- 선행: XV.3.6(`[x]`), XV.4.2(`[~]`)

## 실패 선행조건

- 엄격 선행 XV.4.2가 `[~]`입니다. XV.4.2의 엄격 선행 XV.3.4가 `[~]`이기 때문입니다.
- XV.3.4(`[~]`)는 실제 `ProjectProbe`(IV.5.1)·generation loader(V.4.4)를 `LibraryProjectValidator`로 연결하는 project crate adapter가 없어 부분 완료입니다. materialize된 cache entry에는 `.git`이 없으므로 root `.git`을 요구하지 않는 cache entry용 probe 모드도 필요합니다(`crates/project/src/probe.rs`).
- 이 lane(deps_wiring)의 허용 경로는 `crates/dependencies/**`와 project crate의 manifest·lockfile·YAML writer 파일뿐이라 probe·adapter 파일을 만들거나 고칠 수 없습니다.

## 재현 명령

- `grep -E '^\| XV\.(3\.4|4\.2) \|' docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md`

## 필요한 결정과 환경

- XV.3.4 → XV.4.2 `[x]`가 필요합니다. 삭제 임계치 config 위치(`config/` 아래 새 파일 또는 기존 budget 파일)는 작업 시 ADR로 정해야 합니다.
- 필수 환경: `DEV-MAC`.

## 변경되지 않은 파일

- 기능 코드는 변경하지 않았습니다(계획 §1.2 규칙 4). 이 보고서와 계획서 상태 표기만 바꿨습니다.

## 재개 조건

- XV.4.2 `[x]` 후 착수합니다. 재료: privacy index `last_used_unix_ms`(XV.3.3 touch), `try_delete_entry`(XV.3.6), 실제 lock parser.
