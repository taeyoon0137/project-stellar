# Project Stellar 작업 지침

이 문서는 Project Stellar 저장소에서 작업하는 에이전트의 기준 지침입니다. 작업 전에 먼저 읽고, 이 문서의 규칙을 따릅니다.

## 기본 응답 원칙

- 응답 언어는 한국어를 기본으로 합니다.
- 결론을 먼저 말하고, 확인하지 못한 내용은 명확히 분리합니다.
- 불필요한 칭찬이나 장식적 표현을 쓰지 않습니다.
- 검증하지 않은 내용을 확신하듯 말하지 않습니다.
- 명령, 테스트, 빌드, 파일 상태는 실제로 실행하거나 확인한 경우에만 성공했다고 보고합니다.

## 저장소 역할

Project Stellar는 폴더와 실제 Git 저장소를 원본으로 사용하는 고성능 UI 디자인 소프트웨어입니다. macOS와 Windows에서 공통 Rust 코어를 사용하되 각 운영체제의 네이티브 셸을 제공합니다.

- 현재 저장소는 아키텍처 기준선(Architecture Baseline) 단계입니다. 구현 코드는 아직 포함되어 있지 않습니다.
- 기술 기준은 `docs/PROJECT_STELLAR_TECHNICAL_ARCHITECTURE.md`에 정의되어 있습니다.
- 구현 범위, 모듈 경계, 성능 계약, 파일 포맷, Git 동작에 대한 판단은 위 문서를 우선 기준으로 사용합니다.

## 지침 우선순위

1. 최신 사용자 지시
2. 이 `AGENTS.md`
3. `docs/PROJECT_STELLAR_TECHNICAL_ARCHITECTURE.md`의 기술 기준
4. 편집 대상 파일의 기존 관례

상위 항목이 하위 항목과 충돌하면 상위를 따릅니다. 충돌이 모호하면 임의로 판단하지 말고 사용자에게 확인합니다.

## 작업 시작 체크

- `git status --short`로 현재 변경 상태를 확인합니다.
- 수정할 파일과 관련 문서를 먼저 읽습니다.
- 아키텍처 관련 결정이 필요하면 `docs/PROJECT_STELLAR_TECHNICAL_ARCHITECTURE.md`의 해당 절을 확인합니다.
- 단순하지 않은 작업은 먼저 변경 가능 파일, 기대 동작, 검증 방법을 담은 짧은 계획을 세웁니다.
- 요청이 모호하면 안전한 최소 해석을 선택하고, 선택한 해석을 명시합니다.

## 주요 참고 문서

- `docs/PROJECT_STELLAR_TECHNICAL_ARCHITECTURE.md`: 제품 정의, 아키텍처 원칙, 성능 계약, 문서 모델, 파일 포맷, Git 동작, 로드맵을 담은 기술 기준 문서입니다. 구현 판단의 1차 기준입니다.

세부 문서에 이미 있는 내용을 이 문서에 그대로 복제하지 않습니다. 어떤 작업에서 어느 문서를 먼저 읽어야 하는지를 기준으로 관리합니다.

## 파일 구조와 source of truth

- `README.md`는 생성 결과물입니다. 직접 수정으로 끝내지 않습니다.
- README 원본(source of truth)은 `resources/README.preset.md`입니다.
- README 히어로 wrapper 원본은 `resources/readme-hero.preset.svg`이며, `resources/readme-hero.svg`는 생성 결과물입니다.
- `CLAUDE.md`는 `AGENTS.md`를 가리키는 심볼릭 링크입니다. 별도 사본으로 분리하지 않습니다.
- `docs/`의 기술 문서는 수동 관리 원본입니다.

## 변경 정책

- 가능한 한 작고 국소적인 변경을 만듭니다.
- 기존 동작과 문서의 source-of-truth 관계를 보존합니다.
- 요청과 직접 관련 없는 포맷 변경을 하지 않습니다.
- 사용자가 명시적으로 요청하지 않은 큰 구조 변경이나 광범위한 리팩터링을 하지 않습니다.
- 생성 파일은 원본 파일을 수정한 뒤 생성 명령을 실행해 갱신합니다. 생성 파일을 직접 고쳐 끝내지 않습니다.
- 이미 수정된 파일은 사용자 또는 이전 작업의 변경으로 간주하고 임의로 되돌리지 않습니다.
- 그래픽 리소스를 임의로 만들지 않습니다. 지시에 따르거나, 사용자가 추후 채울 수 있도록 비워 둡니다.

## README 유지보수

- README 내용을 바꿀 때는 `README.md`가 아니라 `resources/README.preset.md`를 수정합니다.
- `resources/README.preset.md` 또는 `resources/readme-hero.preset.svg`를 수정한 뒤 `./scripts/readme_update.sh`를 실행해 `README.md`와 `resources/readme-hero.svg`를 재생성합니다.
- README H1 제목, 한 줄 설명, 히어로 `alt` 텍스트는 `resources/README.preset.md`에 최종 문구로 직접 작성합니다. placeholder를 남기거나 스크립트가 생성·덮어쓰게 하지 않습니다.
- 저장소 구성 섹션은 표가 아니라 `plaintext` 코드블록 tree로 작성하고, 실제로 존재하는 항목만 남깁니다.
- 목차가 있으면 H2 섹션의 추가·삭제·이름 변경에 맞춰 함께 갱신합니다.
- badge는 확인된 핵심 기술과 상태에만 추가하고, 추가·제거 시 README 원본과 생성 결과물을 함께 갱신합니다.
- 히어로 이미지를 넣으려면 `resources/hero.png`(모드별은 `resources/hero.light.png`, `resources/hero.dark.png`)를 추가한 뒤 생성 명령을 실행합니다. 실제로 존재하는 이미지만 SVG에 내장됩니다.
- `resources/readme-hero.preset.svg` 구조(히어로 wrapper)를 재설계하지 않습니다.
- 원본의 placeholder, 경로, 섹션 구조가 바뀌면 `scripts/readme_update.sh`도 함께 갱신합니다.

## 검증 기준

변경 후 가능한 범위에서 아래를 실행합니다.

- README 또는 히어로 변경: `./scripts/readme_update.sh` 실행 후 결과 확인
- `git diff --check`
- `git status --short`
- 심볼릭 링크 확인이 필요하면: `ls -l CLAUDE.md`

실행하지 못한 검증을 성공으로 보고하지 않습니다. 실패한 검증은 무시하지 않고 원인과 변경 관련 여부를 분리해 설명합니다.

## 보안과 기록 원칙

- secret, token, credential, 개인 키를 저장소에 기록하지 않습니다.
- 운영 설정이나 접근 정보는 사용자의 명시적 요청과 확인된 근거가 없으면 수정하지 않습니다.
- 사용자가 명시적으로 요청하지 않은 내용을 커밋 메시지, 코드 주석, 문서, 설정 파일에 임의로 삽입하지 않습니다. 특히 다음은 명시적 지시 없이 추가하지 않습니다.
  - `Co-Authored-By`, `Signed-off-by` 등 trailer
  - 에이전트, 모델, 제공사 이름이나 이메일
  - "Generated with ...", "Made by AI" 등 생성 도구 표기
  - 작업과 무관한 광고, 홍보, 외부 링크

## 커밋 원칙

- 사용자가 명시적으로 요청할 때만 커밋합니다.
- 커밋 메시지 제목은 아래 두 형식 중 하나를 사용합니다. **`Task`와 Prefix는 영문, 제목은 한국어**로 작성합니다.
  - 구현 작업: `Task(<WBS-ID>): <제목>` — 구현 계획(`docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md`)의 WBS 작업 단위 커밋입니다. WBS-ID는 로마 숫자 Phase 번호와 점으로 구분한 번호입니다(예: `I.1.1`).
    - 예: `Task(I.1.1): 저장소 기준선 검증`
  - 유지보수 작업: `<Prefix>: <제목>` — WBS에 속하지 않는 변경입니다.
    - 기능 추가: `Feat: ...`
    - 문서 변경: `Docs: ...`
    - 이름·구조 정리: `Refactor: ...`
    - 버그 수정: `Fix: ...`
    - 설정 변경: `Chore: ...`
    - 테스트 추가·수정: `Test: ...`
    - 예: `Docs: 기술 아키텍처 정의서 추가`
- 이 형식은 `.githooks/commit-msg` 훅으로 강제합니다. 저장소를 새로 clone한 뒤에는 한 번 `git config core.hooksPath .githooks`를 실행해 훅을 활성화합니다.
- 한 커밋에는 하나의 목적만 담습니다. WBS 작업은 한 작업당 한 커밋을 원칙으로 합니다.
- 관련 없는 변경은 stage하지 않습니다. 같은 파일에 사용자 변경과 작업 변경이 섞여 있으면 필요한 hunk만 stage합니다.

## 최종 응답 형식

작업을 마친 뒤 항상 아래를 포함합니다.

1. 변경 요약
2. 변경 파일
3. 실행한 검증 명령과 결과
4. 남은 위험, 한계, 후속 필요 사항

코드 변경 없이 문서만 변경했다면 그 사실을 명시합니다.
