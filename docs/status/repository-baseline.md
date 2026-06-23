# 저장소 기준선 (Repository Baseline)

- WBS: I.1.1
- 기준일: 2026-06-23
- 목적: 현재 저장소의 실제 상태(루트, HEAD, dirty 여부)를 재검증해 기록합니다.

> 보안 원칙에 따라 절대 경로는 기록하지 않습니다. 경로는 저장소 상대 경로로만 표기합니다.

## 실행 명령과 결과

| 명령 | 종료 코드 | 결과 요약 |
|---|---|---|
| `git rev-parse --show-toplevel` | 0 | 저장소 최상위 디렉터리 확인됨 |
| `git status --porcelain=v2` | 0 | tracked 변경 없음, untracked 1건 |
| `git log -1 --format=%H` | 0 | HEAD 커밋 해시 확인됨 |

## 저장소 상태

- 저장소 루트: `project-stellar` (저장소 최상위, 상대 경로 기준 `.`)
- HEAD: `59b1416be23fd187dafd7dfed79834264011aa94`
- dirty 여부:
  - tracked 변경: 없음 (clean)
  - untracked: `.vscode/` 1건 — 작업 환경 디렉터리이며 Git 추적 대상이 아닙니다(`.gitignore`에 포함하지 않는 정책).

## 브랜치와 remote (I.1.2)

| 명령 | 종료 코드 | 결과 |
|---|---|---|
| `git branch --show-current` | 0 | `develop` |
| `git remote -v` | 0 | `origin` (fetch/push) |

- 현재 브랜치: `develop`
- 기본 remote: `origin` → `https://github.com/taeyoon0137/project-stellar.git` (fetch/push 동일)
- 자격 증명 문자열: 없음 — remote URL에 `user:password@` 형태의 인증 정보가 포함되어 있지 않습니다.

### 브랜치 정책 구분

- **본 소스 저장소**(아키텍처·구현 계획·생성 체계를 담는 저장소): 현재 작업 브랜치는 `develop`이며, `main` 브랜치도 존재합니다. 작업은 `develop`에서 진행합니다.
- **제품 프로젝트**(Stellar가 폴더·Git 원본으로 관리하는 디자인 프로젝트): 아키텍처 문서 기준 `main` 고정 브랜치 정책을 따릅니다.
- 위 두 정책은 서로 다른 대상에 적용되며, 제품 프로젝트의 `main` 고정 정책이 본 소스 저장소의 브랜치 운영을 규정하지 않습니다.
