# 작업 보고서: I.1.3 .gitignore 검사·보강

- WBS: I.1.3
- 작업명: `.gitignore` 최소 항목 검사 및 누락 항목 추가
- 상태: 완료 (`[x]`)
- 선행: I.1.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`. (보고서 형식 보완: 계획서 작업 행 `[DEV-MAC]`·ledger `required_environments` 기준)

## 변경 파일 목록

- `.gitignore` (누락 항목 추가: `build/`, `.stellar/cache/`)
- `docs/task-reports/I.1.3.md` (신규, 본 보고서)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.1.3 `[ ]` → `[x]`)

## 최소 항목 점검 결과

| 요구 항목 | 상태 | 근거 규칙 |
|---|---|---|
| `target/` | 기존 충족 | `/target/`, `**/target/` |
| `.DS_Store` | 기존 충족 | `.DS_Store` |
| `.vs/` | 기존 충족 | `.vs/` |
| `build/` | **추가함** | `build/` (CMake 등 범용 out-of-source 빌드) |
| `DerivedData/` | 기존 충족 | `DerivedData/` |
| IDE 사용자 설정 | 기존 충족 | `.idea/`, `*.xcuserstate`, `xcuserdata/`, `*.user`, `*.suo` |
| 로컬 진단·복구 데이터 | **추가함** | `.stellar/cache/` (아키텍처 §19에서 "Git 제외") |

### 판단 근거

- **`.vscode/`는 추가하지 않음**: 사용자 지시(작업 환경 유지)에 따라 `.gitignore`에서 제외. IDE 사용자 설정 항목은 JetBrains(`.idea/`)·Xcode·Visual Studio 사용자 파일로 충족.
- **bare `diagnostics/` 금지**: `crates/diagnostics/`가 추적 대상 소스 크레이트이므로(아키텍처 §7) 일반 `diagnostics/` 패턴을 추가하지 않음. "추적 대상 파일을 잘못 무시하지 않는다" 기준 직접 반영.
- **복구 스냅샷**: 아키텍처 §19 기준 프로젝트 폴더 밖(OS Application Support / App Data)에 저장되어 저장소에 나타나지 않으므로 별도 패턴 불필요.

## 실행한 명령 (`git check-ignore -v` fixture)

| 명령 | 결과 |
|---|---|
| `git check-ignore -v <경로>` (무시 대상 14개) | 전부 종료 코드 0, 매칭 규칙은 아래 출력 |
| `git check-ignore -v <경로>` (추적 대상 5개) | 전부 종료 코드 1(미무시) |

무시되어야 하는 경로 — 전부 종료 코드 0, 매칭 규칙 확인:

```
target/x.o            → **/target/
crates/foo/target/x.o → **/target/
.DS_Store             → .DS_Store
.vs/apply.txt         → .vs/
build/out.o           → build/
DerivedData/x         → DerivedData/
.idea/workspace.xml   → .idea/
foo.xcuserstate       → *.xcuserstate
xcuserdata/x          → xcuserdata/
app.user              → *.user
app.suo               → *.suo
.stellar/cache/blob   → .stellar/cache/
run.log               → *.log
scratch.tmp           → *.tmp
```

무시되면 안 되는 추적 대상 — 전부 종료 코드 1(미무시):

```
crates/diagnostics/lib.rs            미무시(정상)
.vscode/settings.json                미무시(정상)
README.md                            미무시(정상)
docs/status/repository-baseline.md   미무시(정상)
.gitignore                           미무시(정상)
```

## 테스트 결과

- 모든 fixture가 기대대로 동작(무시 항목 exit 0, 추적 대상 exit 1). 완료 기준 충족.

## 남은 경고 수

- 0

## 성능 수치

- 해당 없음

## 작업 지시와 다른 점

- "IDE 사용자 설정" 항목에서 `.vscode/`는 사용자 명시 지시에 따라 의도적으로 제외했습니다(다른 IDE 사용자 파일로 항목 충족).
- "로컬 진단·복구 데이터"는 아키텍처 문서 근거로 `.stellar/cache/`만 추가하고, 복구 스냅샷은 저장소 밖에 저장된다는 점을 명시했습니다(임의 패턴 발명 없음).
