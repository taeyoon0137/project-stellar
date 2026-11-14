<!--
  README.md는 assets/docs/README.preset.md를 기반으로 자동 생성됩니다.
  이 파일을 직접 수정한 내용은 재생성 시 사라집니다.
  내용을 바꾸려면 assets/docs/README.preset.md를 수정한 뒤 ./scripts/readme_update.sh를 실행하세요.
-->

<!-- Hero Image -->
<!--
  히어로 이미지를 넣으려면 assets/docs/hero.png를 추가하세요.
  라이트/다크 모드별 이미지는 assets/docs/hero.light.png, assets/docs/hero.dark.png로 추가합니다.
  같은 이름의 .jpg 파일도 지원합니다.
  이미지를 추가한 뒤 ./scripts/readme_update.sh를 실행하면 실제 존재하는 이미지가 assets/docs/readme-hero.svg에 base64로 내장됩니다.
-->
<p align="center">
  <a href="${repositoryUrl}">
    <img src="../../assets/docs/readme-hero.svg" alt="Project Stellar 히어로 이미지" width="100%" />
  </a>
</p>

<h1 align="center">Project Stellar</h1>
<p align="center">Stellar. Light, left behind.</p>

<p align="center">
  <a href="../../docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md"><img src="https://img.shields.io/badge/status-in%20development-4D24E2" alt="status: in development" /></a>
  <a href="../../docs/adr/ADR-0006-windows-support-deferred.md"><img src="https://img.shields.io/badge/platform-macOS%2026%2B-4D24E2" alt="platform: macOS 26+" /></a>
</p>
<p align="center">
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-%23000000?logo=rust&logoColor=%23FFFFFF" alt="Rust" /></a>
  <a href="https://www.swift.org/"><img src="https://img.shields.io/badge/Swift-%23F05138?logo=swift&logoColor=%23FFFFFF" alt="Swift" /></a>
</p>

- [🧭 역할](#-역할)
- [🚀 시작하기](#-시작하기)
- [📚 문서](#-문서)
- [🛠️ 스크립트](#️-스크립트)
- [🗂️ 저장소 구성](#️-저장소-구성)

<a id="role"></a>

## 🧭 역할

Project Stellar는 폴더와 실제 Git 저장소를 원본으로 사용하는 고성능 UI 디자인 소프트웨어입니다. 공통 Rust 코어 위에 운영체제별 네이티브 셸을 두는 구조이며, 현재는 macOS(Apple silicon, macOS 26 이상)만 지원합니다. Windows 지원은 [ADR-0006](../../docs/adr/ADR-0006-windows-support-deferred.md)에 따라 보류했고, 다시 시작할 수 있도록 코어는 플랫폼 중립으로 유지합니다.

핵심 목표는 다음과 같습니다.

- 실제 Git으로 협업하고, 객체와 속성 단위로 변경을 검토하고 커밋합니다.
- 입력을 받은 뒤 변경 프레임의 GPU 제출과 화면 표시 예약까지 5\~10ms를 목표로 합니다.
- CSS 기반의 타입 있는 디자인 속성으로 레이아웃과 컴포넌트를 설계하고, 타깃별 코드 생성에 같은 의미를 사용합니다.
- CLI와 MCP를 통해 AI 에이전트가 디자인을 조회하고 편집하며, 사용자는 변경을 검토할 수 있습니다.

다음은 목표로 하지 않습니다.

- 실시간 공동 편집 서버나 CRDT·OT 기반 공식 협업 모델
- 자체 AI 모델이나 에이전트 런타임
- 클라우드 문서 서비스 형태의 제품

공식 협업 모델은 Git 기반 비동기 협업입니다.

<a id="getting-started"></a>

## 🚀 시작하기

현재 저장소는 구현 단계입니다. 공통 Rust 코어 crate(`crates/`), macOS 네이티브 셸(`apps/macos/`, AppKit·Swift), 개발 도구와 `stellar` 명령·MCP 브리지(`tools/`)가 들어 있습니다. 기술 기준은 [`docs/PROJECT_STELLAR_TECHNICAL_ARCHITECTURE.md`](../../docs/PROJECT_STELLAR_TECHNICAL_ARCHITECTURE.md)에, 단계별 구현·검증 계획은 [`docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md`](../../docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md)에 정의되어 있습니다.

macOS 앱에서 프로젝트 폴더를 열면 사이드바, 인스펙터, 캔버스와 하단 도구 막대가 있는 프로젝트 창이 열립니다. CLI는 저장된 프로젝트의 구조와 변경을 조회하고 `apply --dry-run`으로 편집 결과를 미리 볼 수 있습니다. 화면별 코어 연결 상태와 검증 근거는 [작업 원장](../../planning/task-status.yaml)과 [작업 보고서](../../docs/task-reports/)에서 확인할 수 있습니다.

필요한 환경은 다음과 같습니다.

- macOS 26 이상, Apple silicon
- Rust 1.96.0 (`rust-toolchain.toml`에 고정)
- Xcode 26 이상 (macOS 26 SDK, macOS 앱 빌드)

Rust workspace 전체 검사(fmt·clippy·test·doctest)와 macOS 앱 Debug 빌드·smoke 검사는 다음 명령으로 실행합니다. Xcode 프로젝트(`apps/macos/StellarMac.xcodeproj`)는 빌드 단계에서 Rust 코어(`stellar-ffi`)를 cargo로 함께 빌드합니다.

```bash
cargo xtask check-core
./scripts/macos/smoke.sh
```

커밋 메시지 형식 검사 훅을 활성화하려면 저장소를 clone한 뒤 한 번 다음을 실행합니다.

```bash
git config core.hooksPath .githooks
```

<a id="docs"></a>

## 📚 문서

- [`docs/PROJECT_STELLAR_TECHNICAL_ARCHITECTURE.md`](../../docs/PROJECT_STELLAR_TECHNICAL_ARCHITECTURE.md): 제품 정의, 아키텍처 원칙, 성능 계약, 문서 모델, 파일 포맷, Git 동작, 로드맵을 담은 기술 기준 문서입니다. 구현 판단의 1차 기준입니다.
- [`docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md`](../../docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md): 아키텍처 기준 문서를 원자적 작업 패키지(WBS)로 분해한 구현·검증 실행 계획입니다. 단계별 작업과 완료 게이트(G0\~G8)를 정의하며 초기 릴리스 범위는 Phase I\~XVII(G8)입니다.
- [`docs/adr/`](../../docs/adr/): 아키텍처 결정 기록(ADR)입니다. 예를 들어 [ADR-0006](../../docs/adr/ADR-0006-windows-support-deferred.md)은 Windows 지원 보류와 macOS 우선 결정을 기록합니다.
- [`docs/spec/`](../../docs/spec/): Stellar 프로젝트 파일 포맷 명세, 버전 호환성, 보안 한도를 정의합니다.
- [`docs/design/macos-shell-design.md`](../../docs/design/macos-shell-design.md): macOS 셸 디자인 가이드입니다.
- [`docs/architecture/`](../../docs/architecture/): crate 의존 그래프, panic 정책, lockfile 정책 같은 세부 아키텍처 규칙입니다.
- [`docs/status/`](../../docs/status/): 저장소 기준선, 소스 inventory, 측정 결과 기록입니다.
- [`docs/task-reports/`](../../docs/task-reports/): WBS 작업별 완료·차단 보고서입니다.
- [`planning/task-status.yaml`](../../planning/task-status.yaml): 작업 상태 ledger입니다. 생성 결과물이며 `cargo xtask generate-ledger`로 다시 만듭니다.

<a id="scripts"></a>

## 🛠️ 스크립트

<!--
  README.md는 생성 결과물입니다.
  내용을 바꿀 때는 assets/docs/README.preset.md를 수정한 뒤 ./scripts/readme_update.sh를 실행하세요.
  placeholder, 경로, 섹션 구조가 바뀌면 생성 스크립트도 함께 갱신합니다.
-->

| 명령                                    | 설명                                                                                                                                  |
| :-------------------------------------- | :------------------------------------------------------------------------------------------------------------------------------------ |
| `./scripts/readme_update.sh`            | `assets/docs/README.preset.md`와 `assets/docs/readme-hero.preset.svg`에서 `README.md`와 `assets/docs/readme-hero.svg`를 재생성합니다. |
| `./scripts/source_inventory.sh`         | 추적 파일의 확장자별 수와 자동 생성물·vendored code를 집계해 `docs/status/source-inventory.md`를 재생성합니다.                        |
| `./scripts/macos/smoke.sh`              | macOS 앱을 Debug로 빌드하고 FFI·수명주기·캔버스·텍스트 입력·입력 이벤트·업데이트 smoke 검사를 실행합니다.                             |
| `./scripts/macos/qa.sh` | macOS 앱을 Debug로 빌드하고 임시 샘플 프로젝트에서 [실사용 QA 절차](../../docs/qa/README.md)를 시작합니다. |
| `./scripts/macos/release_build.sh`      | 서명하지 않은 arm64 Release 앱을 빌드하고 배포 구성과 번들 구조를 검사합니다.                                                         |
| `./scripts/macos/bundle_check.sh`       | `StellarMac.app`의 번들 구조·링크·rpath·코드 서명 구조를 검사하고 재배치 실행 smoke를 실행합니다.                                     |
| `./scripts/macos/codesign_bundle.sh`    | 앱 번들을 안쪽 코드부터 Hardened Runtime으로 서명합니다.                                                                              |
| `./scripts/macos/sign_release.sh`       | Release 빌드 후 Developer ID(`--development`이면 Apple Development)로 서명하고 검증합니다.                                            |
| `./scripts/macos/notarize.sh`           | Developer ID로 서명된 앱을 공증·stapling하고 Gatekeeper 평가를 확인합니다.                                                            |
| `./scripts/ci/check-commit-subjects.sh` | 기준 커밋 이후 커밋 제목이 커밋 원칙 형식을 따르는지 검사합니다.                                                                      |
| `./scripts/ci/check-adr-policy.sh`      | 변경된 경로에 ADR 보호 경로 정책을 적용합니다.                                                                                        |
| `cargo xtask <명령>`                    | 저장소 작업 도구입니다. `cargo xtask help`로 명령 목록을 봅니다.                                                                      |

<a id="repository-structure"></a>

## 🗂️ 저장소 구성

```plaintext
project-stellar
├── .cargo/                 # cargo 별칭 설정 (`cargo xtask`)
├── .githooks/              # 커밋 메시지 형식 검사 훅
├── .github/                # PR 템플릿과 Bootstrap CI workflow
├── apps/                   # 네이티브 셸 (현재 macOS: AppKit·Swift)
├── assets/                 # README 원본·히어로 이미지(docs/), 로고 마크·앱 아이콘(icons/)
├── benchmarks/             # 성능 기준 fixture 생성기
├── config/                 # 의존성·버전·성능 예산·검증 환경 등 정책 설정
├── crates/                 # 공통 Rust 코어 crate
├── docs/                   # 기술 아키텍처·구현 계획·ADR·명세·작업 보고서
├── planning/               # 작업 상태 ledger (생성 결과물)
├── scripts/                # README 생성, CI 정책 검사, macOS 빌드·서명·smoke 스크립트
├── tests/                  # crate 사이 종단 시나리오 통합 테스트
├── tools/                  # 저장소 작업 도구(xtask), stellar 명령, MCP 브리지
├── .gitattributes          # 줄바꿈·속성 규칙
├── .gitignore              # Git 추적 제외 규칙
├── AGENTS.md               # 에이전트 작업 지침 (source of truth)
├── CLAUDE.md               # AGENTS.md 심볼릭 링크
├── Cargo.lock              # 의존성 잠금 파일
├── Cargo.toml              # Rust workspace 정의
├── README.md               # 생성 결과물
└── rust-toolchain.toml     # Rust 도구체인 고정 (1.96.0)
```
