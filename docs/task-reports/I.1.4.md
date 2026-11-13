# 작업 보고서: I.1.4 .gitattributes 명시

- WBS: I.1.4
- 작업명: 텍스트 정규화·셸 LF·PowerShell CRLF·바이너리 fixture 속성 명시 (merge driver 미등록)
- 상태: 완료 (`[x]`)
- 선행: I.1.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`. (보고서 형식 보완: 계획서 작업 행 `[DEV-MAC]`·ledger `required_environments` 기준)

## 변경 파일 목록

(보고서 형식 보완: 커밋 `daa1f73`의 변경 파일 기준)

- `.gitattributes` (규칙 추가)
- `docs/task-reports/I.1.4.md` (신규, 본 보고서)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.1.4 `[ ]` → `[x]`)

## 문서화된 규칙

| 분류 | 패턴 | 속성 | 근거 |
|---|---|---|---|
| 텍스트 정규화 | `*` | `text=auto eol=lf` | 기존 |
| 셸 스크립트 | `*.sh` | `text eol=lf` | 기존 |
| PowerShell | `*.ps1`, `*.psm1`, `*.psd1` | `text eol=crlf` | **추가** (Windows 셸, CRLF) |
| 이미지 | `*.png/.jpg/.jpeg/.ico` | `binary` | 기존 |
| 바이너리 fixture/콘텐츠 | `*.starchunk` | `binary` | **추가** (아키텍처 §글로벌맵 "deterministic binary chunks") |
| 폰트 | `*.ttf/.otf/.woff/.woff2` | `binary` | **추가** (아키텍처 §11.3 폰트 Git 포함 허용) |

### 의도적으로 하지 않은 것

- **merge driver 미등록**: "Stellar 프로젝트용 merge driver는 아직 등록하지 않습니다" 지시에 따라 `merge=` 속성을 추가하지 않음. 이후 Git Semantic Merge 단계에서 등록 예정(주석으로 명시).
- **`.stargen`/`.stellarref` 분류 보류**: 문서에 text/binary 구분이 확정되어 있지 않아 임의 분류하지 않음. 현재 `* text=auto`가 적용되며 Git이 바이너리 콘텐츠를 자동 판별. 파일 포맷 구현 단계에서 확정.

## 실행한 명령 (`git check-attr -a -- <fixture>`)

| 명령 | 결과 |
|---|---|
| `git check-attr -a -- <fixture>` (아래 fixture 경로별) | 모든 결과가 문서화된 규칙과 일치(아래 출력) |

```
scripts/readme_update.sh                       → text: set,  eol: lf
build/win.ps1                                  → text: set,  eol: crlf
tools/mod.psm1                                 → text: set,  eol: crlf
resources/hero.jpg / hero.png                  → binary: set (text/diff/merge: unset)
design/objects/sha256/ab/deadbeef.starchunk    → binary: set (text/diff/merge: unset)
assets/sha256/cd/font.ttf                      → binary: set (text/diff/merge: unset)
crates/diagnostics/lib.rs                       → text: auto, eol: lf
stellar.yaml                                   → text: auto, eol: lf
design/current.stellarref                      → text: auto, eol: lf
```

- 모든 결과가 문서화된 규칙과 일치. 바이너리 항목에 표시되는 `eol: lf`는 `*` 규칙에서 비롯되나 `binary` 매크로가 `text`를 unset 하므로 EOL 변환은 적용되지 않음(모순 아님).

## 테스트 결과

- `git check-attr -a` 출력이 문서화된 규칙과 일치. 완료 기준 충족.

## 남은 경고 수

- 0

## 성능 수치

- 해당 없음

## 작업 지시와 다른 점

- 바이너리 fixture 속성은 문서 근거가 있는 `*.starchunk`와 폰트만 명시하고, `.stargen`/`.stellarref`는 분류를 보류했습니다(임의 판단 금지).
