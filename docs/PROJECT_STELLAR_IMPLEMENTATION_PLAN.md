# Project Stellar 원자적 구현·검증 실행 계획

- 기준 아키텍처: `PROJECT_STELLAR_TECHNICAL_ARCHITECTURE.md`
- 기준일: 2026-06-23
- 현재 진행 상황은 아래 WBS 표와 [작업 원장](../planning/task-status.yaml)에서 확인합니다.
- 원자 작업 수: 1152개
- 구현 Phase 수: 30개 (`I`\~`XXX`)
- 초기 릴리스 범위: Phase `I`~`XVII`, G8 통과
- 로드맵 범위: Phase `XVIII`\~`XXX`
- 이 문서는 세부 실행 계획만 포함하며 전체 기술 아키텍처를 중복 수록하지 않음

---

## 개정 기록

- **2026-06-23 — Rust 부트스트랩 순서 교정:** Phase I의 정책 검증기가 Rust `xtask`를 요구하면서 Rust 워크스페이스는 Phase II에 있던 순환 의존성을 제거했습니다. Rust 도구체인, 최소 Cargo 워크스페이스, `cargo xtask help`, 양 플랫폼 Bootstrap CI를 작업 추적 검증보다 먼저 배치했습니다.
- **2026-06-23 — 검증 환경 책임 분리:** macOS 개발자가 Windows 검증을 로컬에서 대신하지 않도록 `DEV-MAC`, `CI-MAC`, `CI-WIN`, `LAB-WIN`, `REF-MAC`, `REF-WIN`의 책임과 증거 조건을 고정했습니다.
- **2026-06-23 — 커밋 규칙 고정:** 모든 작업 커밋 제목은 예외 없이 `Task(<WBS-ID>): <작업명>`을 사용합니다. `Docs:`, `Chore:`, 소문자 `task(...)`와 임의 유지보수 prefix는 허용하지 않습니다.
- **2026-06-23 — I.2.1 원자화:** `I.2.1`은 TaskStatus Rust 데이터 모델만 정의합니다. YAML 파싱, 레코드 검증, 그래프 검증, 증거 검증, 계획 동기화는 별도 WBS로 분리했습니다.
- **2026-11-14 — Windows 지원 보류(ADR-0006):** 당분간 macOS만 지원합니다. Windows 전용 행은 `[!]`(사용자 결정 차단)로 표기하고, 양 플랫폼 비교 행은 ADR-0006의 판정 규칙(IEEE 기본 연산만 쓰는 결과는 macOS 증거로 `[x]`, 플랫폼 의존 결과는 `[~]`)을 따릅니다. 코어는 플랫폼 중립을 유지합니다.
- **2026-11-14 — 커밋 규칙 정합:** 지침 우선순위상 위인 `AGENTS.md`에 맞춰 WBS 작업은 `Task(<WBS-ID>)`, WBS 밖 유지보수는 `Feat:`·`Docs:`·`Refactor:`·`Fix:`·`Chore:`·`Test:`를 허용합니다. 2026-06-23 "커밋 규칙 고정"을 대체하며, `cargo xtask validate-commit-message`와 CI commit subject 검사도 같은 기준을 씁니다.
- **2026-11-14 — 제품 테스트 실행 위치:** 제품 crate 테스트는 CI에 추가하지 않고, 통합 시 로컬 `cargo xtask check-core`(workspace fmt·clippy·test·doctest)로 검증합니다. 행 지시의 "CI smoke"는 이 로컬 통합 검증을 뜻합니다(VI.10.5 문구 수정). 2026-11-14 후속으로 Actions의 `Rust core checks`에서 같은 명령을 수동 실행하고 실패 로그를 받을 수 있습니다. push·PR의 기본 검사 구성은 기존 결정을 따릅니다.
- **2026-11-14 — 3계층 에이전트 도구 결정 반영(ADR-0005 Accepted):** 제품 소유자 결정에 따라 내장 에이전트는 사용자가 설치하고 직접 로그인한 공급자 공식 CLI를 공식 문서의 비대화형 인터페이스로만 실행하고, Stellar는 인증·자격 증명·사용량 중개에 참여하지 않습니다. 출력은 채팅 보기(기본)와 원문 보기를 함께 제공하고 공급자 출력 디코드는 표시 전용으로 격리합니다. 앱 없이 동작하는 읽기 전용 MCP backend를 신뢰된 고정 프로젝트에 한해 허용합니다. 에이전트 세션이 끊기거나 중지돼도 적용 변경은 자동 rollback하지 않고 출처 표시와 함께 남겨 사용자가 변경·세션 단위로 직접 되돌립니다. headless 명령의 실행 이름은 `stellar`이며 macOS 앱 첫 실행 때 명령 설치 화면을 제공합니다. 목적이 사라진 행은 폐기 표기 없이 삭제하는 규칙으로 바꿨습니다(3. 계획 변경 규칙). XIX.1.4, XIX.3.3, XIX.6.1, XIX.6.3, XIX.6.4, XIX.7.2와 Phase XIX 완료 조건을 그 자리에서 고쳤으며, XIX.1.6\~XIX.1.9, XIX.6.7\~XIX.6.10, XIX.9, XXIV.4, XXIV.5와 Phase XXIX를 추가했습니다.
- **2026-11-14 — CSS 기반 디자인 속성 어휘(ADR-0024 Accepted):** 제품 소유자 방향에 따라 인스펙터가 편집하는 디자인 속성의 이름·값 문법·단위·레이아웃 의미는 CSS longhand를 정본으로 삼습니다. 저장은 바이너리 chunk와 숫자 `PropertyKey`를 유지하고 CSS 텍스트를 파일에 저장하지 않습니다. 영속 record는 longhand만 저장하고, Stellar가 받아들인 선언은 CSS와 같은 결과를 내야 하며 예외는 `docs/design/property_model.md` 14장 목록뿐입니다. 이미 저장되는 속성의 기본값은 바꾸지 않습니다. 완료된 행은 고치지 않고 Phase IX에 IX.11(CSS 기반 속성 체계, 23행)을 추가했습니다. 내용은 registry CSS 매핑, 레이아웃 의미 정렬과 CSS 적합성 fixture, 영속 layout record, Frame·Text migration, CSS 값 형식화·파싱과 복사·붙여넣기, 인스펙터 CSS 범주, export 정렬입니다. 교차 검토 뒤 longhand 하나를 PropertyKey 하나로 두는 결정(ADR-0024 결정 14)을 더하고, 제품 문서 migration registry(IX.11.21), 합성 key 분할(IX.11.22), flex item 자동 최소 크기(IX.11.23) 행을 추가했습니다. 내보내기·코드 생성이 로드맵인데도 export 정렬 행 IX.11.18·IX.11.20을 Phase IX에 두는 근거는 ADR-0024 Consequences에 적었습니다.
- **2026-11-14 — 캔버스 편집 UX(XII.10) 추가:** 제품 소유자 방향(편집 UX는 전문 UI 디자인 도구의 관례를 참고하되 베끼지 않음, UI는 Xcode 계열 macOS 네이티브 형태 위의 Stellar 고유 형태, 인스펙터·캔버스 편집 속성은 CSS 기반)에 따라 `docs/design/ui/canvas_editing.md`를 기준으로 XII.10.1\~XII.10.44를 추가했습니다. 도구·수식키 결정(ADR), 선택·깊은 선택·마퀴·hover, 변형 핸들, nudge, 스냅·스마트 가이드·눈금자·그리드, 정렬·분배, 순서·묶음, 흐름 컨테이너 드래그와 padding·gap 핸들, 생성·펜·Boolean, 캔버스 텍스트, 인스턴스 편집·교체, 확대 제스처 입력과 줌·팬, 클립보드, 문맥 메뉴, 한 제스처 한 history 적합성, 키보드 전용·VoiceOver 경로, 성능 replay, Object 메뉴의 뒤집기(XII.10.44)를 다룹니다. CSS 속성을 편집하는 행은 그 속성을 만드는 IX.11 행을 선행으로 둡니다(캔버스 명세 24장). 기존 행의 상태와 내용은 바꾸지 않았습니다.
- **2026-11-14 — 노드 프로토타이핑·Component Context·코드 생성 방향(ADR-0025):** 제품 소유자 방향 P4에 따라 CSS 기반 속성을 값 연결 대상, 노드 그래프 포트, 코드 생성 매핑 단위로 함께 쓰기로 했습니다. 제품 소유자가 프로토타이핑·Context·코드 변환을 "추후" 지원한다고 했으므로 Component Context 기능은 로드맵에 두고, 초기 릴리스 범위인 Phase IX에는 파일 포맷 공개(XXVIII.3.1) 전에 필요한 저장 번호 예약 IX.12.1만 추가했습니다(ADR-0025 결정 6). 로드맵 Phase XXX에 노드 기반 프로토타이핑(기존 binding·상태 머신 위의 projection, `MotionSpec`, 논리 시계 평가, 프리뷰, macOS 그래프 편집기, XXX.1.1\~XXX.6.3)과 Component Context(선언된 provide·require·consume, slot 해석, 해석·진단·command·인스펙터, XXX.7.1\~XXX.7.13)를 추가했습니다. 착수 전인 XXII.1.1, XXII.2.2, XXII.2.3, XXII.3.2, XXII.4.2를 CSS 기반 속성(IX.11)·Context를 소비하도록 그 자리에서 고쳤고, XXVIII.3.1의 선행에 IX.12.1을 더했으며, XXII.1.4\~XXII.1.7, XXII.2.5, XXII.2.6, XXII.3.4, XXII.4.3을 추가했습니다. XXII.1.4는 IX.11.18·IX.11.19를 다시 정의하지 않고 CSS 기반 속성의 Apple·Android lowering만 더하며, Context·`MotionSpec` 항목은 XXII.1.7, CodegenIR의 Context는 XXII.1.6이 맡습니다. 그래서 CSS 기반 속성·레이아웃 코드 생성 행은 Phase XXX를 기다리지 않고, 상태 머신·`MotionSpec`·Context를 다루는 행만 XXX.1.1·XXX.7.4 뒤에 진행합니다(Phase XXII 순서 문단, ADR-0025 결정 6). 기존 상태 표기는 바꾸지 않았습니다.
- **2026-11-14 — 내장 터미널 사용자 셸 환경(ADR-0023 Accepted):** 제품 소유자 결정에 따라 대화형 내장 터미널은 계정 기본 로그인 셸을 사용자의 전체 로그인 환경과 설정(rc 파일, `PATH`, prompt, alias, plugin)으로 실행합니다. Stellar는 터미널 식별 변수(`TERM`, `COLORTERM`, `TERM_PROGRAM`, `TERM_PROGRAM_VERSION`)와 기본 셸 profile의 셸 경로 `SHELL`만 설정하고, 셸 설정을 바꾸지 않은 채 terminal view에 Stellar 테마(색, 글꼴, cursor, Light·Dark·대비 높이기)를 적용합니다. 신뢰된 프로젝트 게이트는 유지하고, 앱이 시작하는 agent CLI 하위 프로세스는 ADR-0023 결정 9의 허용 목록 환경(launch profile에 적은 이름만 전달)으로 따로 실행합니다. 완료 행 XVIII.1.1은 고치지 않았고, 바뀐 요구사항은 새 행 XVIII.2.5(로그인 환경 상속)와 XVIII.4.7(terminal view 테마)이 맡습니다. XXIX.2.4와 XXIX.2.6에 허용 목록 환경 지시와 검증을 더했고, 미착수 행 XVIII.4.1에는 `TERM`·`COLORTERM` 값을 정하는 지시와 검증을 그 자리에서 더했습니다.
- **2026-11-14 — 새 macOS 행의 Windows `[!]` 선행 사슬 기록:** 위 네 개정에서 추가한 macOS 행 중 다음 행은 기존 행의 선행을 따라 Windows `[!]` 행에 닿아 ADR-0006 아래에서는 착수할 수 없습니다. XII.10.16과 XVIII.4.7은 XII.9.3→XII.9.2, XVIII.4.7은 XVIII.4.3→XVIII.4.2→XVIII.4.1→XVIII.3.1에도 닿습니다. XII.10.33은 XII.5.6→XII.5.5, XII.10.43은 XII.10.16과 XVI.4.2→XVI.4.1→XVI.1.3→XVI.1.2→XI.9.5, XXX.6.2는 XVI.4.6→XVI.2.5→XVI.2.4입니다. 사슬 중간의 XII.9.3, XII.5.6, XVI.1.2, XVI.2.5, XVIII.4.1은 이 개정 전부터 있던 행이라 선행을 바꾸지 않았습니다. 이 행들을 macOS 전용 선행으로 다시 잇는 일은 ADR-0006의 판정 규칙과 함께 별도 계획 개정에서 합니다.

---

## 1. 원자적 구현·검증 실행 계획

본 장의 작업 번호는 기존 문서 장 번호와 독립된 WBS 식별자입니다. 예를 들어 `II.1.1`은 작업 패키지 ID입니다. 작업 하나는 한 번의 에이전트 실행, 한 개의 완료 보고서, 한 개의 `Task(...)` 커밋으로 끝내는 것을 원칙으로 합니다.

### 1.1 작업 상태 읽기

각 WBS 행의 상태는 이 계획서에서 관리합니다. [작업 원장](../planning/task-status.yaml)은 행별 상태, 필수 환경, 근거 커밋과 보고서 경로를 함께 기록합니다. 개별 검증의 범위와 미확인 항목은 [작업 보고서](task-reports/)에서 확인합니다.

상태 표기는 다음과 같습니다.

- `[x]`: 작업에 지정된 모든 필수 환경의 검증 증거, 보고서, 올바른 커밋까지 존재
- `[~]`: 구현 또는 일부 환경 검증은 끝났으나 필수 증거·보고서·커밋 중 하나 이상이 미완료
- `[ ]`: 미착수
- `[!]`: 선행 조건, 외부 장비, 사용자 결정 때문에 차단
- 폐기 표기는 두지 않습니다. 목적이 사라진 행은 계획서와 `planning/task-status.yaml`에서 삭제하고 이력은 Git으로 확인합니다(3. 계획 변경 규칙). ledger의 `cancelled` 상태 값은 XXIX.1.4에서 제거합니다.

### 1.2 원자적 작업 패키지 규칙

1. 한 번에 WBS 작업 하나만 수행합니다.
2. 작업 행에 명시된 경로 밖의 파일은 수정하지 않습니다. 자동 생성 파일이 바뀌면 보고서에 생성 원인과 명령을 기록합니다.
3. 공용 API, 파일 포맷, FFI 시그니처, numeric registry를 작업 지시 없이 추가하지 않습니다.
4. 선행 작업이 모두 `[x]`가 아니면 코드를 수정하지 않고 blocked 보고서만 작성합니다.
5. 테스트 또는 재현 fixture를 먼저 추가하고 요구사항이 실패하는 것을 확인한 뒤 구현합니다. 단순 문서·설정 작업은 작업 행의 결정적 검증 명령을 먼저 정의합니다.
6. `TODO`, `unimplemented!`, 빈 함수, 항상 성공하는 테스트를 남기지 않습니다. 의도적인 placeholder는 이름에 `Stub`을 포함하고 제한을 검증하는 테스트를 둡니다.
7. 오류는 구조화된 타입 또는 명시된 종료 코드로 반환합니다. stderr 문자열만 남기고 성공 처리하지 않습니다.
8. 커밋 제목은 `AGENTS.md` 커밋 원칙에 따라 다음 정규식을 만족해야 합니다(Git이 만드는 `Revert "..."` 제목은 예외).

```regex
^(Task\([IVXLCDM]+\.[1-9][0-9]*\.[1-9][0-9]*\)|Feat|Docs|Refactor|Fix|Chore|Test): .+$
```

9. WBS 작업 커밋은 반드시 `Task(<WBS-ID>): <제목>`을 씁니다. `Feat:`·`Docs:`·`Refactor:`·`Fix:`·`Chore:`·`Test:`는 WBS에 속하지 않는 유지보수 변경에만 씁니다. 소문자 `task(...)`와 그 밖의 prefix는 금지하고, `fixup!`·`squash!` 커밋은 병합 전 rebase 또는 squash로 정리합니다.
10. 실패 시 기능 코드는 커밋하지 않고 `docs/task-reports/<WBS-ID>-blocked.md`만 작성합니다.
11. 완료 보고서 `docs/task-reports/<WBS-ID>.md`에는 다음을 기록합니다.
    - 변경 파일 목록
    - 실행한 명령과 종료 코드
    - 검증 환경 코드
    - CI·LAB·REF 실행 URL 또는 artifact locator
    - 테스트 결과와 남은 경고 수
    - 성능 원시 결과 경로
    - 작업 지시와 다른 점 및 이유
12. 부트스트랩 단계에 존재하지 않는 도구를 완료 기준으로 요구하지 않습니다.
    - `I.1.1`~`I.1.7`: 작업 행의 Git·shell·`rustup` 명령만 사용
    - `I.1.8`: `cargo metadata`, `cargo check -p xtask --locked`까지 사용
    - `I.1.9` 이후 Phase I: `cargo fmt -p xtask -- --check`, `cargo clippy -p xtask --all-targets -- -D warnings`, `cargo test -p xtask`
    - `II.1.2` 완료 후: 전체 workspace `fmt`, `clippy`, `test`
13. 한 작업에서 직접 작성하는 프로덕션 파일 5개 또는 테스트 파일 5개를 초과해야 하면 구현을 중단하고 하위 WBS 분할안을 작성합니다. 자동 생성 fixture와 lockfile은 집계에서 제외하되 보고서에는 기록합니다.
14. 외부 의존성은 `config/dependencies.yaml` 승인 항목만 허용합니다. 해당 파일이 생기기 전에는 `I.2.3`의 단일 bootstrap 예외만 허용합니다.
15. 날짜·경로·난수·해시·직렬화 순서가 결과에 영향을 주는 테스트는 고정 입력을 사용합니다.
16. macOS 로컬 결과로 Windows 완료를 주장하지 않습니다. Windows 필수 작업은 `CI-WIN`, `LAB-WIN`, `REF-WIN` 중 작업에 지정된 환경 증거가 있어야 `[x]`가 됩니다.
17. 공통 코어 작업의 로컬 구현은 `DEV-MAC`에서 완료할 수 있지만, 플랫폼 동등성은 별도 CI 또는 Gate 작업이 통과하기 전까지 제품 수준 보장으로 간주하지 않습니다.

### 1.3 검증 환경 계약

| 코드 | 실행 위치 | 허용하는 검증 | 허용하지 않는 주장 |
|---|---|---|---|
| `DEV-MAC` | 개발자의 macOS 26+ Apple silicon 장비 | 구현, Rust unit test, AppKit·Metal 로컬 smoke, 빠른 benchmark | Windows x64 빌드·WinUI·D3D12·Windows 성능 완료 |
| `CI-MAC` | 깨끗한 macOS CI runner | clean checkout Rust test, macOS compile, 결정적 fixture | 물리 M1 기준 성능, 수동 UI·GPU 체감 |
| `CI-WIN` | 깨끗한 Windows x64 hosted CI runner | Rust x64, MSBuild, WinUI compile, headless test | Windows 11 데스크톱 상호작용·실제 D3D12·DPI·성능 |
| `LAB-WIN` | Windows 11 x64 interactive self-hosted 장비 | WinUI 실행, UI automation, DPI, D3D12 기능 검증 | 기준 하드웨어 성능 통과 |
| `REF-MAC` | Apple M1·16GB·macOS 26+ 기준 장비 | macOS 성능 릴리스 게이트 | Windows 성능 |
| `REF-WIN` | 2020년급 6코어·16GB·D3D12 iGPU·Windows 11 기준 장비 | Windows 성능 릴리스 게이트 | macOS 성능 |

검증 적용 규칙:

1. 작업 행에 `[DEV-MAC]`, `[CI-WIN]`처럼 환경이 명시되어 있으면 모두 필수입니다.
2. 환경이 명시되지 않은 문서·모델·플랫폼 비종속 단일 작업의 기본 필수 환경은 `DEV-MAC`입니다.
3. `CI-WIN`은 Windows 빌드와 비대화형 테스트 증거입니다. Windows 11 런타임·WinUI UI automation·D3D12 기능 검증은 `LAB-WIN`에서만 인정합니다.
4. Apple silicon에서 실행하는 Windows ARM VM의 x64 에뮬레이션 결과는 `LAB-WIN` 또는 `REF-WIN` 증거로 인정하지 않습니다.
5. 필수 환경이 준비되지 않으면 작업 상태는 `[!]` 또는 `[~]`이며, 구현자가 해당 환경을 모방해 `[x]`로 바꾸지 않습니다.
6. CI 증거는 workflow run URL, job name, commit SHA, artifact checksum을 보고서에 기록합니다.
7. LAB·REF 증거는 장비 프로필 ID, OS build, GPU/driver, 실행 artifact checksum과 원시 결과 경로를 기록합니다.

### 1.4 공통 산출물 위치

```text
config/
├─ bootstrap-dependencies.yaml   # I.2.3~I.3.2 사이에서만 존재
├─ dependencies.yaml
├─ verification-environments.yaml
├─ versions.yaml
└─ performance-budgets.yaml

docs/
├─ adr/
├─ task-reports/
├─ fixtures/
└─ status/

planning/
├─ task-status.yaml
└─ milestones.yaml

rust-toolchain.toml
Cargo.toml
Cargo.lock
.cargo/config.toml

crates/
apps/
tests/
benchmarks/
tools/xtask/
```

### 1.5 단계 완료 게이트

| 게이트 | 완료 조건 | 필수 환경 |
|---|---|---|
| G0 저장소·검증 기준선 | Phase I 전체 완료. 고정 Rust, xtask-only workspace, TaskStatus 검증, 커밋·ADR·의존성 정책, 계획 동기화가 clean checkout에서 실행 가능 | `CI-MAC`, `CI-WIN` |
| G1 코어 실행 | Rust 코어와 양쪽 네이티브 셸이 동일 ABI의 FFI ping 호출 성공 | `CI-MAC`, `CI-WIN`, `LAB-WIN` |
| G2 프로젝트 수명주기 | 새 프로젝트 생성·초기 커밋·열기·Clone 검증 통과 | `CI-MAC`, `CI-WIN`, 플랫폼 UI 작업은 `LAB-WIN` |
| G3 영속 문서 | Frame 하나를 생성하고 저장·재실행·복구 가능 | `CI-MAC`, `CI-WIN` |
| G4 편집 코어 | Command·Undo/Redo·선택 히스토리 통과 | `CI-MAC`, `CI-WIN` |
| G5 디자인 모델 | 토큰·Binding·Component·View Variant가 결정론적으로 평가됨 | `CI-MAC`, `CI-WIN` |
| G6 화면 표시 | Metal·D3D12 결과가 허용 오차 내 동등 | `LAB-WIN`, macOS GPU 장비 |
| G7 Git MVP | Semantic Diff·부분 Stage·Commit·Push·Conflict 해결 통과 | `CI-MAC`, `CI-WIN`, UI는 `LAB-WIN` |
| G8 초기 릴리스 | 성능·보안·복구·패키징 수용 기준 전부 통과 | `REF-MAC`, `REF-WIN` 포함 |

---

## Phase I. 저장소 기준선과 검증 도구 부트스트랩

> **순서 불변조건:** `I.1.1`~`I.1.10`을 완료한 뒤 `I.2.*`를 시작합니다. `I.1.8` 전에는 Cargo workspace를 요구하지 않습니다. `I.1.9` 전에는 `cargo xtask`를 완료 기준으로 요구하지 않습니다. `II.1.2` 전에는 product workspace 전체 검증을 요구하지 않습니다.

### I.1 저장소 기준선, 이력 정합화와 Rust 부트스트랩

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| I.1.1 | `[x]` | 저장소 루트에서 `git rev-parse --show-toplevel`, `git status --porcelain=v2`, `git log -1 --format=%H`를 실행하고 결과를 `docs/status/repository-baseline.md`에 기록합니다. 절대 경로는 저장하지 않습니다. | `[DEV-MAC]` 세 명령 종료 코드 0, 보고서에 루트·HEAD·dirty 여부 존재. | 없음 |
| I.1.2 | `[x]` | 현재 브랜치와 remote를 기록합니다. 소스 저장소 브랜치 정책과 Stellar가 생성하는 제품 프로젝트의 `main` 정책을 구분합니다. Git 설정은 변경하지 않습니다. | `[DEV-MAC]` `git branch --show-current`, `git remote -v` 결과가 보고서에 있고 자격 증명 문자열 없음. | I.1.1 |
| I.1.3 | `[x]` | `.gitignore`를 검사하고 `target/`, `.DS_Store`, `.vs/`, `build/`, `DerivedData/`, IDE 사용자 설정, 로컬 진단·복구 데이터를 반영합니다. | `[DEV-MAC]` `git check-ignore -v` fixture 통과, 추적 대상 오탐 0. | I.1.1 |
| I.1.4 | `[x]` | `.gitattributes`에 텍스트 정규화, shell LF, PowerShell 정책, binary fixture 속성을 명시합니다. Stellar merge driver는 등록하지 않습니다. | `[DEV-MAC]` `git check-attr -a -- <fixture>`가 문서화 규칙과 일치. | I.1.1 |
| I.1.5 | `[x]` | 현재 파일 목록과 언어별 파일 수를 `docs/status/source-inventory.md`에 기록하고 자동 생성물·vendored code를 별도 집계합니다. | `[DEV-MAC]` 동일 commit에서 두 번 생성한 보고서가 byte-identical. | I.1.1 |
| I.1.6 | `[x]` | Phase I 계획과 검증 환경 계약을 이 문서 및 `config/verification-environments.yaml`에 반영합니다. 기존 `e6432db`를 `Task(I.1.6): 검증 환경 계약 및 Phase I 재구성`으로 reword합니다. 개인 오타 사전만 포함한 `1f6a170`은 drop하고 `.vscode/settings.json`을 제거합니다. 기존 I.1.1~I.1.5 commit은 보존합니다. | `[DEV-MAC]` `git log --format=%s d123376..HEAD`의 모든 제목이 Task 정규식과 일치하고 `Docs:`·`Chore:`·소문자 `task(`가 0건. 계획서 Phase는 모두 Roman numeral. working tree clean. | I.1.5 |
| I.1.7 | `[x]` | `rust-toolchain.toml`만 사용해 Rust `1.96.0`, `profile = "minimal"`, components `rustfmt`, `clippy`를 고정합니다. `config/versions.yaml`은 만들지 않습니다. 기존 `6446175`를 `Task(I.1.7): Rust 도구체인 고정`으로 reword합니다. | `[DEV-MAC]` `rustup show active-toolchain`이 repo override `1.96.0`, `rustc -Vv`, `cargo -V`, `cargo fmt -V`, `cargo clippy -V` 종료 0. 보고서 `I.1.7.md`. Windows 완료 주장 금지. | I.1.6 |
| I.1.8 | `[x]` | 루트 `Cargo.toml`에 resolver `2` 최소 workspace를 생성하고 member를 `tools/xtask` 하나로 고정합니다. package 이름은 `xtask`, 외부 dependency는 0개입니다. `Cargo.lock`을 생성·추적합니다. 기존 `0411b77`을 `Task(I.1.8): 최소 Cargo 워크스페이스와 xtask 크레이트`로 reword합니다. | `[DEV-MAC]` `cargo metadata --format-version 1 --no-deps` member가 `xtask` 하나, `cargo check -p xtask --locked` 종료 0, `Cargo.lock` tracked. | I.1.7 |
| I.1.9 | `[x]` | `.cargo/config.toml`에 `xtask = "run --package xtask --"`를 추가하고 `tools/xtask/src/main.rs`에 `help`만 지원하는 router를 구현합니다. 인자 없음과 unknown command는 usage를 stderr에 출력하고 `64`로 종료합니다. 기존 `9f4a1a6`을 `Task(I.1.9): cargo xtask help 진입점`으로 reword합니다. | `[DEV-MAC]` `cargo xtask help`=`0`, `cargo xtask`=`64`, unknown=`64`; package fmt·clippy·test 종료 0. | I.1.8 |
| I.1.10 | `[x]` | `.github/workflows/bootstrap.yml`에 `bootstrap-macos`와 `bootstrap-windows` job을 추가합니다. 각 job은 clean checkout에서 pinned toolchain 활성화, `rustc -Vv`, `cargo -V`, `cargo metadata`, `cargo check -p xtask --locked`, `cargo xtask help`, xtask fmt·clippy·test를 실행합니다. Windows job은 compile/headless 증거이며 Windows 11 UI·D3D12 증거로 표시하지 않습니다. | `[CI-MAC][CI-WIN]` 두 job 모두 동일 commit에서 성공하고 Rust release=`1.96.0`; job URL과 log artifact checksum이 `docs/task-reports/I.1.10.md`에 존재. | I.1.9 |

### I.2 작업 추적 데이터와 검증기

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| I.2.1 | `[x]` | `tools/xtask/src/task_status/model.rs`에 `TaskStatusRecord`, `TaskState`, `VerificationEnvironment`, `VerificationEvidence` 데이터 모델만 정의합니다. 필드는 `id`, `title`, `status`, `owner`, `depends_on`, `required_environments`, `evidence`, `commit`, `report`, `verified_at`입니다. YAML 파싱과 파일 I/O는 구현하지 않습니다. | `[DEV-MAC]` 생성자·enum unit test 통과. 모듈에서 filesystem/YAML parser import 0. | I.1.10 |
| I.2.2 | `[x]` | `tools/xtask/src/task_status/wbs_id.rs`에 Roman Phase와 두 개의 decimal 하위 번호를 갖는 `WbsId` parser·Display·Ord를 구현합니다. `0`, 음수, suffix, 두 단계·네 단계 ID는 거부합니다. | `[DEV-MAC]` 정상 `I.1.1`, `XXVIII.3.3` 통과; `1.1.1`, `I.0.1`, `I.1`, `I.1.1.1`, `I.01.1` 실패. | I.2.1 |
| I.2.3 | `[x]` | Phase I parser의 유일한 bootstrap dependency로 `yaml-rust2 = "=0.11.0"`을 승인합니다. `config/bootstrap-dependencies.yaml`에 crate, exact version, license `MIT OR Apache-2.0`, 사용 범위 `Phase I YAML event parsing only`, 제거 시점 `I.3.2`를 기록하고 `tools/xtask/Cargo.toml`에 추가합니다. 다른 crate는 추가하지 않습니다. | `[DEV-MAC]` `cargo tree -p xtask`에 직접 dependency `yaml-rust2 v0.11.0` 하나만 추가되고 lockfile 갱신 이유가 보고서에 기록. | I.2.2 |
| I.2.4 | `[x]` | `tools/xtask/src/yaml/restricted_loader.rs`에 yaml-rust2 event 기반 loader를 작성합니다. 단일 document와 mapping/sequence/scalar만 허용하고 anchor, alias, custom tag, merge key, 다중 document를 위치 포함 오류로 거부합니다. TaskStatus 필드 해석은 하지 않습니다. | `[DEV-MAC]` 허용 syntax fixture 통과; 금지 syntax 각 1개 fixture가 고유 오류 코드와 line/column을 반환. | I.2.3 |
| I.2.5 | `[x]` | `tools/xtask/src/task_status/decode.rs`에 restricted YAML value를 `TaskStatusRecord` raw field 구조로 변환하는 decoder를 구현합니다. top-level은 `tasks` sequence 하나만 허용하고 unknown key·duplicate key·잘못된 scalar/sequence 형식을 거부합니다. 상태 규칙과 선행 graph는 검사하지 않습니다. | `[DEV-MAC]` 필드별 정상 fixture와 unknown·duplicate·type mismatch fixture 통과. | I.2.4 |
| I.2.6 | `[x]` | `tools/xtask/src/task_status/validate_record.rs`에 단일 record 필수값과 nullable 규칙을 구현합니다. 빈 title, 빈 required environment, 중복 environment, 잘못된 report 상대 경로, commit SHA 형식 오류를 거부합니다. 다른 record 존재 여부는 검사하지 않습니다. | `[DEV-MAC]` 각 필드 오류를 하나씩 가진 table-driven test 통과. | I.2.5 |
| I.2.7 | `[x]` | `tools/xtask/src/task_status/validate_collection.rs`에 task ID 중복, 허용하지 않는 상태, title mismatch 기본 검사를 구현합니다. 허용 상태는 `not_started`, `in_progress`, `reported_done`, `verified`, `blocked`, `cancelled`입니다. | `[DEV-MAC]` duplicate ID·unknown status fixture 실패, 정상 collection 통과. | I.2.6 |
| I.2.8 | `[x]` | `tools/xtask/src/task_status/validate_graph.rs`에 `depends_on` 대상 존재, self dependency, 중복 edge, cycle 검사를 구현합니다. 범위 표기와 Phase/Gate 문자열은 ledger의 exact dependency로 허용하지 않습니다. | `[DEV-MAC]` missing·self·duplicate·2-node/10-node cycle fixture 실패, DAG 통과. | I.2.7 |
| I.2.9 | `[x]` | `tools/xtask/src/task_status/validate_evidence.rs`에 상태별 증거 규칙을 구현합니다. `verified`는 commit·report·verified_at과 모든 required environment evidence가 필요하고, `reported_done`은 report가 필요하며, `not_started`는 commit·report·evidence가 없어야 합니다. | `[DEV-MAC]` 환경 1개 누락, report 누락, 과도한 not_started evidence fixture 실패. | I.2.7 |
| I.2.10 | `[x]` | `tools/xtask/src/plan/markdown_wbs.rs`에 Markdown table 행에서 WBS ID·title·status·선행 exact ID를 추출하는 parser를 구현합니다. fenced code, prose 예시, range 표기는 WBS 정의로 세지 않습니다. | `[DEV-MAC]` 실제 계획서에서 ID를 중복 없이 추출하고 code fence 속 가짜 ID fixture를 무시. | I.2.2 |
| I.2.11 | `[x]` | `tools/xtask/src/task_status/write.rs`에 deterministic ledger writer를 구현합니다. key 순서, 2-space indent, LF, final newline, task ID 정렬을 고정하고 주석은 생성하지 않습니다. | `[DEV-MAC]` 동일 collection 100회 출력 byte-identical, parse→write→parse 의미 동일. | I.2.5 |
| I.2.12 | `[x]` | 계획서 WBS 추출 결과로 `planning/task-status.yaml`을 생성합니다. `I.1.1`\~`I.1.5`는 `verified`, `I.1.6`\~`I.1.9`는 현재 교정 전이면 `reported_done`, 나머지는 `not_started`로 기록합니다. 각 record의 required environment는 작업 행과 Gate 규칙을 반영합니다. 수작업 복사 대신 I.2.10·I.2.11 library를 호출하는 일회성 subcommand를 사용합니다. | `[DEV-MAC]` 계획 ID 집합과 ledger ID 집합 동일, decoder·record·collection·graph·evidence validator 모두 통과. | I.2.8, I.2.9, I.2.10, I.2.11 |
| I.2.13 | `[x]` | `docs/task-reports/TEMPLATE.md`를 생성합니다. WBS, 변경 파일, 명령·종료 코드, 환경·증거, 테스트, 성능, 차이, 남은 위험 섹션을 고정합니다. | `[DEV-MAC]` template fixture에 필수 heading이 정확히 한 번씩 존재. | I.2.1 |
| I.2.14 | `[x]` | `docs/task-reports/BLOCKED_TEMPLATE.md`를 생성합니다. 실패 선행조건, 재현 명령, 필요한 결정·환경, 변경되지 않은 파일, 재개 조건을 고정합니다. | `[DEV-MAC]` 필수 heading fixture 통과. | I.2.13 |
| I.2.15 | `[x]` | `tools/xtask/src/task_report/validate.rs`에 완료·blocked 보고서 validator를 구현합니다. template heading, WBS 일치, 검증 환경, 명령 종료 코드, artifact locator 형식을 검사합니다. | `[DEV-MAC]` heading·WBS·evidence를 하나씩 제거한 fixture가 실패. | I.2.13, I.2.14 |
| I.2.16 | `[x]` | `.github/pull_request_template.md`에 WBS ID, 변경 경로, 명령·환경 증거, 파일 포맷·성능·보안 영향, 보고서 링크를 추가합니다. validator는 별도 함수로 구현하고 WBS 없는 body를 거부합니다. | `[DEV-MAC]` 정상 body 2개 통과, WBS·증거·보고서 각각 누락 fixture 실패. | I.2.15 |
| I.2.17 | `[x]` | `tools/xtask/src/commit_message.rs`에 commit subject validator를 구현합니다. 허용 형식은 `Task(<WBS-ID>): <비어 있지 않은 제목>`과 `AGENTS.md` 유지보수 형식 `<Feat·Docs·Refactor·Fix·Chore·Test>: <비어 있지 않은 제목>`입니다. Git `Revert "..."` 외에 merge·fixup·소문자 task 예외를 만들지 않습니다. | `[DEV-MAC]` 올바른 Roman ID 3개와 유지보수 prefix 6개 통과; lowercase, Arabic Phase, 미승인 prefix, 빈 제목, malformed ID fixture 실패. | I.2.2 |

### I.3 버전·의존성·ADR 정책

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| I.3.1 | `[x]` | `config/versions.yaml`과 validator를 생성합니다. Rust는 `exact: 1.96.0`만 허용하고 `rust-toolchain.toml`과 일치해야 합니다. 다른 도구는 실제 사용이 시작된 항목만 기록하며 `exact` 또는 `range` 중 하나만 허용합니다. | `[DEV-MAC]` duplicate·empty·exact/range 동시·Rust mismatch fixture 실패. | I.2.4, I.1.7 |
| I.3.2 | `[x]` | `config/dependencies.yaml`과 validator를 생성합니다. `yaml-rust2 0.11.0` bootstrap 승인을 정식 항목으로 이전한 뒤 `config/bootstrap-dependencies.yaml`을 삭제합니다. 필드는 name, ecosystem, exact version, license, approval, purpose, replaceable입니다. 미등록 direct dependency를 거부합니다. | `[DEV-MAC]` `cargo metadata` direct dependency 집합과 승인 목록 일치, bootstrap file 없음, unknown dependency fixture 실패. | I.3.1, I.2.3 |
| I.3.3 | `[x]` | dependency validator에 license policy를 추가합니다. 자동 허용은 MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC; GPL·AGPL·SSPL은 자동 거부; 그 밖은 `separate_approval: true`가 필요합니다. | `[DEV-MAC]` 허용·거부·별도 승인 fixture 통과. | I.3.2 |
| I.3.4 | `[x]` | `docs/adr/TEMPLATE.md`를 생성합니다. Status, Context, Decision, Alternatives, Consequences, Compatibility, Performance, Security, Migration 섹션을 고정합니다. | `[DEV-MAC]` heading fixture 통과. | I.2.13 |
| I.3.5 | `[x]` | `tools/xtask/src/adr_policy.rs`에 보호 경로 정책을 구현합니다. storage format, public API, FFI, numeric registry, performance budget 변경은 같은 diff에 새·수정 ADR이 없으면 실패합니다. | `[DEV-MAC]` 보호 경로만 변경 fixture 실패, ADR 동반 fixture 통과, docs-only 오탐 0. | I.3.4 |

### I.4 공통 검증 명령과 G0 CI

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| I.4.1 | `[x]` | 기존 `help` router를 모듈형 command registry로 확장하고 `validate-task-status`, `validate-task-report`, `validate-pr-body`, `validate-commit-message`, `validate-versions`, `validate-dependencies`, `validate-adr`를 연결합니다. 성공 `0`, 입력 오류 `64`, 정책 위반 `65`, 내부 오류 `70`을 사용합니다. | `[DEV-MAC]` help snapshot에 각 명령 1회, 정상·정책 위반 fixture의 종료 코드 일치, xtask fmt·clippy·test 통과. | I.2.12, I.2.15, I.2.16, I.2.17, I.3.3, I.3.5 |
| I.4.2 | `[x]` | `cargo xtask verify-task <ID>`를 구현합니다. ledger 선행 상태, 상태별 필수 필드, report, commit subject, required environment evidence를 확인합니다. `II.1.2` 전에는 product workspace build 결과를 요구하지 않습니다. | `[DEV-MAC]` success·missing dependency·missing report·missing CI-WIN evidence·premature workspace requirement fixture가 기대 코드 반환. | I.4.1 |
| I.4.3 | `[x]` | `cargo xtask sync-plan --check`를 구현합니다. 계획 WBS와 ledger의 누락·추가·중복·title·depends_on 차이를 보고하며 파일을 수정하지 않습니다. | `[DEV-MAC]` 각 drift fixture를 정확히 보고하고 실제 계획서에서는 종료 0. | I.2.10, I.2.12, I.4.1 |
| I.4.4 | `[x]` | Bootstrap CI의 macOS·Windows job에 `sync-plan --check`, 모든 Phase I validator, xtask fmt·clippy·test를 추가합니다. commit subject 검사는 기준 commit 이후 모든 non-merge commit을 검사하며 예외 prefix는 없습니다. | `[CI-MAC][CI-WIN]` clean branch 성공. task ledger, commit subject, dependency, ADR 정책을 각각 하나씩 깨뜨린 fixture branch에서 해당 job 실패. | I.4.2, I.4.3 |

**Phase I 완료 조건:** `I.*` 전체가 `[x]`이고 G0를 통과합니다. `I.1.6`~`I.1.9`의 현재 `[~]`는 교정된 `Task(...)` 이력과 보고서가 반영된 뒤에만 `[x]`로 변경합니다.

---

## Phase II. Rust 런타임, 모노레포와 네이티브 셸 골격

### II.1 Rust 도구체인과 제품 워크스페이스

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| II.1.1 | `[x]` | Phase I에서 고정한 Rust `1.96.0`을 제품 코어 공식 baseline으로 승격합니다. `docs/adr/ADR-0001-rust-toolchain.md`에 변경 절차, MSRV 정책, 업그레이드 전용 WBS 요구사항을 기록합니다. 버전을 자동 변경하지 않습니다. | `[CI-MAC][CI-WIN]` `rustc -Vv`의 release와 commit-hash가 동일하고 ADR heading·결정이 검증됩니다. | G0 |
| II.1.2 | `[x]` | I.1.8의 xtask-only workspace를 제품 workspace로 확장합니다. member는 여전히 `tools/xtask` 하나로 두고 root `Cargo.toml`에 resolver v2, debug/release profile, workspace package metadata, workspace lint를 추가합니다. 기능 crate는 추가하지 않습니다. | `[DEV-MAC][CI-MAC][CI-WIN]` `cargo metadata --no-deps`, workspace fmt·clippy·test 통과, member=`xtask` 하나. | II.1.1 |
| II.1.3 | `[x]` | panic 정책을 정의합니다. 앱 라이브러리 release는 `panic=abort`, 테스트는 unwind를 사용하고 FFI 경계에서 panic이 넘어가지 않도록 정책 문서를 추가합니다. | release profile 검사와 panic fixture 테스트 통과. | II.1.2 |
| II.1.4 | `[x]` | workspace 공통 lint에 `unsafe_op_in_unsafe_fn`을 오류로 설정하고 공개 API missing docs는 초기에는 경고로 둡니다. | 의도적 unsafe fixture가 설명·명시 블록 없이 실패. | II.1.2 |
| II.1.5 | `[x]` | I.1.8에서 생성한 `Cargo.lock`을 제품 workspace의 단일 lockfile로 유지하고 모든 CI Rust 명령에 `--locked`를 적용합니다. dependency-update WBS 외 lockfile drift를 금지합니다. | lockfile 변경 없이 `cargo check --workspace --locked` 통과, 누락·무단 drift fixture 실패. | II.1.2 |
| II.1.6 | `[x]` | `cargo xtask check-core`를 구현해 전체 workspace fmt, clippy, test, doc test를 고정 순서로 실행하고 첫 실패에서 종료합니다. 명령별 로그를 `target/xtask-logs/`에 저장하며 Phase I xtask-only 검사와 혼합하지 않습니다. | 각 하위 명령 실패 fixture에서 정확한 실패 코드·로그 생성, 성공 시 네 단계 모두 실행. | II.1.2, I.4.1 |

### II.2 공통 crate 골격

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| II.2.1 | `[x]` | `crates/core-types`를 생성하고 공개 모듈만 선언합니다. 구현은 없습니다. | `cargo test -p stellar-core-types` 통과, 금지된 외부 의존성 없음. | II.1.5 |
| II.2.2 | `[x]` | `crates/document`, `crates/commands`, `crates/history`를 각각 생성하고 의존 방향을 `history -> commands -> document -> core-types`로 고정합니다. 역방향 의존은 금지합니다. | `cargo tree` 검사에서 순환·역방향 의존 없음. | II.2.1 |
| II.2.3 | `[x]` | `crates/expressions`, `crates/bindings`, `crates/layout`, `crates/text` 골격을 생성합니다. 모두 `core-types`만 직접 의존하게 시작합니다. | 각 crate 단독 `cargo test` 통과. | II.2.1 |
| II.2.4 | `[x]` | `crates/scene`, `crates/renderer-core`, `crates/renderer-metal`, `crates/renderer-d3d12` 골격을 생성합니다. 플랫폼 backend는 공통 backend를 역참조하지 않게 합니다. | target별 feature 없이 공통 플랫폼에서 platform backend crate가 빌드 대상에서 제외됩니다. | II.2.1 |
| II.2.5 | `[x]` | `crates/storage`, `crates/project`, `crates/assets`, `crates/dependencies` 골격을 생성합니다. | 의존 방향 문서와 `cargo tree` 검사 통과. | II.2.1 |
| II.2.6 | `[x]` | `crates/semantic-diff`, `crates/semantic-merge`, `crates/git-porcelain` 골격을 생성합니다. Git crate는 document를 직접 수정하지 않고 snapshot 인터페이스만 의존하도록 빈 trait를 둡니다. | 컴파일 및 의존성 검사 통과. | II.2.2, II.2.5 |
| II.2.7 | `[x]` | `crates/target-profile`, `crates/accessibility`, `crates/diagnostics`, `crates/ffi` 골격을 생성합니다. | workspace 전체 check 통과. | II.2.1 |
| II.2.8 | `[x]` | `docs/architecture/crate-dependency-graph.md`에 허용 의존 방향을 Mermaid와 표로 기록하고 CI에서 금지 edge를 검사합니다. | 의도적으로 역방향 edge를 넣은 fixture가 실패합니다. | II.2.2~II.2.7 |

### II.3 공통 오류·로그·시간 계측

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| II.3.1 | `[x]` | `core-types`에 `ErrorCode` enum과 `StellarError { code, message, context }`를 정의합니다. 사용자 문자열과 개발자 context를 분리합니다. | 직렬화 round-trip과 안정적인 numeric code golden test 통과. | II.2.1 |
| II.3.2 | `[x]` | `diagnostics`에 구조화 로그 이벤트 타입을 정의합니다. 프로젝트 내용·텍스트·경로를 기본 필드에 넣을 수 없도록 별도 민감 타입을 둡니다. | 민감 타입을 일반 telemetry event에 넣으면 컴파일 실패하는 UI test 또는 compile-fail test 통과. | II.3.1, II.2.7 |
| II.3.3 | `[x]` | 단조 시계를 추상화한 `MonotonicClock` trait와 테스트용 `ManualClock`을 추가합니다. 시스템 wall clock은 성능·히스토리 순서에 사용하지 않습니다. | ManualClock으로 동일 입력이 동일 duration을 생성합니다. | II.2.1 |
| II.3.4 | `[x]` | `DocumentRevision`, `SceneRevision`, `StorageGeneration`을 서로 다른 newtype으로 추가해 혼용을 컴파일 단계에서 막습니다. | 서로 다른 revision 비교·대입 compile-fail test 통과. | II.2.1 |

### II.4 FFI 경계 골격

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| II.4.1 | `[x]` | `ffi` crate에 `stellar_runtime_create`, `stellar_runtime_destroy`, `stellar_runtime_version` C ABI를 추가합니다. handle은 opaque pointer가 아니라 정수형 registry handle로 노출합니다. | C 테스트 프로그램이 create/version/destroy를 1,000회 반복해 누수·double free 없이 종료. | II.2.7, II.3.1 |
| II.4.2 | `[x]` | 모든 FFI 함수가 `StellarStatus`를 반환하고 오류 세부 정보는 호출별 `stellar_last_error(handle)`로 읽게 합니다. 전역 last error는 금지합니다. | 두 handle에서 동시에 오류를 만들었을 때 서로 섞이지 않습니다. | II.4.1 |
| II.4.3 | `[x]` | FFI 문자열은 UTF-8 pointer+length 입력, 호출자 제공 buffer 출력 규칙으로 고정합니다. NUL 종료 의존을 금지합니다. | embedded NUL, 빈 문자열, 잘못된 UTF-8 fixture 처리 테스트 통과. | II.4.2 |
| II.4.4 | `[x]` | FFI 경계에 `catch_unwind`를 적용하고 panic을 `InternalPanic` 오류 코드로 변환합니다. release abort 정책에서는 panic을 발생시키는 프로덕션 경로가 없음을 별도 테스트합니다. | 테스트 전용 panic 함수가 프로세스를 죽이지 않고 오류로 변환됩니다. | II.4.2, II.1.3 |
| II.4.5 | `[x]` | Core→Shell 이벤트 큐 인터페이스를 정의합니다. 이벤트는 pull 방식 `stellar_runtime_poll_event`로 읽고 callback 재진입을 금지합니다. | 빈 큐, 단일 이벤트, 여러 이벤트, buffer 부족 테스트 통과. | II.4.2 |
| II.4.6 | `[x]` | C header를 자동 생성하거나 고정 템플릿에서 생성하는 `cargo xtask generate-ffi-header`를 추가하고 생성물 drift를 CI에서 검사합니다. | 재생성 후 Git diff가 없어야 합니다. | II.4.1~II.4.5 |

### II.5 macOS 셸 골격

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| II.5.1 | `[x]` | `apps/macos`에 macOS 26, arm64 전용 AppKit 앱 프로젝트를 생성합니다. SwiftUI는 앱 설정 화면용 placeholder target만 두고 캔버스는 AppKit view로 둡니다. | `xcodebuild -project ... -scheme StellarMac -configuration Debug CODE_SIGNING_ALLOWED=NO build` 통과. | II.4.6 |
| II.5.2 | `[x]` | Rust static 또는 dynamic library를 Debug 빌드에 연결하고 앱 시작 시 `stellar_runtime_version`을 호출해 로그에 기록합니다. | 앱 smoke test가 FFI 버전 문자열을 읽고 종료 코드 0. | II.5.1 |
| II.5.3 | `[x]` | `NSApplicationDelegate`와 빈 `NSWindowController`를 추가합니다. 창 닫기·앱 종료 시 runtime destroy가 정확히 한 번 호출되게 합니다. | lifecycle UI test에서 create/destroy count가 1:1입니다. | II.5.2 |
| II.5.4 | `[x]` | 최소 `NSView` CanvasHost를 추가하고 backing scale과 drawable size 변경 이벤트를 Rust core에 전달하는 구조만 만듭니다. 렌더링은 하지 않습니다. | resize fixture에서 pixel size가 point size×scale과 일치합니다. | II.5.3 |

### II.6 Windows 셸 골격

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| II.6.1 | `[!]` | `apps/windows`에 Windows 11 x64, C++/WinRT, WinUI 3 앱 솔루션을 생성합니다. ARM64 configuration은 만들지 않습니다. | `msbuild ... /p:Configuration=Debug /p:Platform=x64` 통과. | II.4.6 |
| II.6.2 | `[!]` | Rust library를 Debug 빌드에 연결하고 앱 시작 시 `stellar_runtime_version`을 호출합니다. | packaged/unpackaged 개발 실행 중 하나를 고정하고 smoke test 통과. | II.6.1 |
| II.6.3 | `[!]` | `App`, `MainWindow`, 빈 CanvasHost custom control을 추가하고 종료 시 runtime destroy를 한 번 호출합니다. | lifecycle test에서 handle 누수 없음. | II.6.2 |
| II.6.4 | `[!]` | DPI 변경과 logical/physical pixel size를 core에 전달하는 이벤트를 추가합니다. | 100%, 150%, 200% DPI fixture의 physical size가 정확합니다. | II.6.3 |

### II.7 양 플랫폼 빌드 게이트

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| II.7.1 | `[x]` | macOS arm64 CI job에 Rust core, FFI header drift, Xcode Debug build를 연결합니다. | clean runner에서 성공. | II.5.4 |
| II.7.2 | `[!]` | Windows x64 CI job에 Rust core, FFI header drift, MSBuild Debug build를 연결합니다. | clean runner에서 성공. | II.6.4 |
| II.7.3 | `[!]` | 양 플랫폼 smoke artifact가 같은 core version과 ABI version을 보고하는 검사를 추가합니다. | 버전 mismatch fixture에서 CI 실패. | II.7.1, II.7.2 |

**Phase II 완료 조건:** G1 통과.

---

## Phase III. 코어 식별자, 값 타입과 런타임 자료구조

### III.1 StableId와 런타임 ID

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| III.1.1 | `[x]` | `core-types`에 128비트 `StableId`를 구현하고 UUIDv7 생성과 16바이트 직렬화를 제공합니다. 문자열 형식은 표준 소문자 hyphen UUID로 고정합니다. | RFC 형식, 시간 순 정렬, binary/string round-trip 테스트 통과. | G1 |
| III.1.2 | `[x]` | 테스트용 결정적 `StableIdGenerator`를 구현합니다. seed와 수동 시각이 같으면 동일 ID 시퀀스를 생성합니다. | golden sequence test 통과. | III.1.1, II.3.3 |
| III.1.3 | `[x]` | `RuntimeEntityId { slot, generation }`을 추가하고 저장·FFI에 노출되지 않게 visibility를 제한합니다. | 삭제 후 slot 재사용 시 이전 generation lookup 실패. | III.1.1 |
| III.1.4 | `[x]` | `StableId ↔ RuntimeEntityId` 양방향 registry를 구현합니다. duplicate StableId 삽입은 오류입니다. | 삽입·삭제·재삽입·중복 fixture 통과. | III.1.3 |
| III.1.5 | `[x]` | 복제용 `IdRemapTable`을 추가해 하위 그래프 StableId 전체를 새 ID로 바꾸고 내부 참조를 재매핑합니다. | 외부 참조는 유지되고 내부 참조만 새 ID를 가리키는 fixture 통과. | III.1.2, III.1.4 |

### III.2 PropertyPath와 참조

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| III.2.1 | `[x]` | `PropertyKey`를 문자열이 아닌 고정 numeric ID로 정의하고 사람이 읽는 이름과 별도 registry를 둡니다. | 이름 변경 후 numeric key가 유지되는 golden test 통과. | III.1.1 |
| III.2.2 | `[x]` | `PropertyPath`를 `StableId + Vec<PropertyKey> + optional index/key segment`로 정의합니다. 경로는 canonical 비교·정렬이 가능해야 합니다. | 동일 의미 경로가 동일 bytes로 직렬화됩니다. | III.2.1 |
| III.2.3 | `[x]` | 강한 타입 참조 `NodeRef`, `TokenRef`, `ComponentRef`, `AssetRef`를 분리합니다. 서로 암묵 변환을 금지합니다. | 잘못된 참조 대입 compile-fail test 통과. | III.1.1 |
| III.2.4 | `[x]` | dangling reference를 표현하는 `UnresolvedRef`를 추가하고 원래 ID·예상 타입·발견 위치를 보존합니다. | resolve 실패 후 정보 손실 없는 round-trip 통과. | III.2.3 |

### III.3 기본 값과 단위 타입

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| III.3.1 | `[x]` | `FiniteF32` 또는 동등 타입을 구현해 NaN·±Infinity를 생성·직렬화 단계에서 거부합니다. `-0.0`은 `0.0`으로 정규화합니다. | 모든 특수 float fixture 통과. | G1 |
| III.3.2 | `[x]` | `Length`, `Percentage`, `Angle`, `Duration`, `Scalar`를 별도 newtype으로 구현합니다. | 잘못된 단위 산술 compile-fail test 통과. | III.3.1 |
| III.3.3 | `[x]` | `Vec2`, `Size2`, `Rect`, `Insets`, `Transform2D`를 finite 값만 허용하도록 구현합니다. | 역행렬 가능·불가능, 빈 rect, 음수 크기 정책 테스트 통과. | III.3.1 |
| III.3.4 | `[x]` | 문자열 값은 UTF-8을 사용하고 저장 전 Unicode NFC 정규화를 선택적으로 적용할 위치를 분리합니다. 사용자 텍스트 자체는 정규화하지 않고 식별자·파일 경로만 정규화합니다. | 사용자 텍스트 보존과 경로 정규화 fixture 모두 통과. | G1 |
| III.3.5 | `[x]` | `TypedValue` tagged union에 Boolean, Number, String, Enum, Length, Percentage, Angle, Duration, Color, Gradient, Shadow, Typography, ResourceRef를 정의합니다. | 모든 variant binary round-trip과 type tag 안정성 golden test 통과. | III.3.2, III.3.4 |

### III.4 색상과 그래픽 값

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| III.4.1 | `[x]` | `ColorSpace`에 sRGB와 DisplayP3를 정의하고 색상은 비연관 알파(unassociated alpha)로 저장합니다. | alpha 0·1, gamut edge fixture round-trip 통과. | III.3.1 |
| III.4.2 | `[x]` | 선형 색공간 변환 함수를 정의하고 sRGB/P3 encode/decode reference fixture를 추가합니다. | 허용 오차가 `config/performance-budgets.yaml`의 색상 기준 이하. | III.4.1 |
| III.4.3 | `[x]` | Gradient stop을 안정 ID, position, color로 정의하고 position 정렬은 저장 순서와 시각 순서를 분리합니다. | 동일 position stop의 안정 순서 fixture 통과. | III.1.1, III.4.1 |
| III.4.4 | `[x]` | Shadow와 blur parameter 타입을 정의하고 음수·비정상 radius를 거부합니다. | 경계값 테스트 통과. | III.3.1, III.4.1 |

### III.5 자식 순서 키

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| III.5.1 | `[x]` | `OrderKey` 요구사항 fixture를 먼저 작성합니다. 앞·뒤·사이 삽입, 서로 다른 브랜치 동시 삽입, 결정론적 정렬, 키 길이 상한을 검증합니다. 구현은 하지 않습니다. | fixture가 현재 미구현으로 실패하고 요구사항 문서가 존재합니다. | III.1.1 |
| III.5.2 | `[x]` | 승인된 OrderKey 알고리즘을 `core-types`에 구현합니다. 키는 비교 가능한 byte sequence이며 생성에 난수를 사용하지 않습니다. | 100만 회 임의 삽입 property test에서 중복·순서 역전 없음. | III.5.1, ADR 승인 |
| III.5.3 | `[x]` | 키가 임계 길이를 넘을 때 부모 단위 재균형 함수를 구현합니다. 재균형은 의미 순서를 바꾸지 않습니다. | 재균형 전후 StableId 순서 동일, 두 번 실행 시 동일 결과. | III.5.2 |
| III.5.4 | `[x]` | Semantic Merge에서 독립 삽입이 충돌 없이 합쳐지는 OrderKey fixture를 추가합니다. merge 구현 전에는 fixture data만 만듭니다. | fixture 스키마 검증 통과. | III.5.2 |

### III.6 Arena와 SoA 기초

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| III.6.1 | `[x]` | `document`에 generational arena wrapper를 추가합니다. 개별 node box allocation을 공개 API에서 금지합니다. | 100만 insert/remove stress test에서 stale handle 접근이 모두 실패합니다. | III.1.3 |
| III.6.2 | `[x]` | 공통 node 필드를 SoA column으로 저장하는 최소 `NodeColumns`를 구현합니다. 종류별 payload는 아직 없습니다. | 삽입·삭제 후 모든 column 길이 일치 invariant test 통과. | III.6.1 |
| III.6.3 | `[x]` | `DocumentSnapshotView` 읽기 전용 인터페이스를 추가하고 mutable arena reference를 외부 crate에 노출하지 않습니다. | 외부 crate에서 직접 mutate compile-fail test 통과. | III.6.2 |

**Phase III 완료 조건:** 모든 코어 primitive가 결정론적 round-trip과 compile-fail 타입 안전성 테스트를 통과합니다.

---

## Phase IV. 프로젝트 매니페스트, 생성·열기·Clone

### IV.1 제한된 YAML 파서

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| IV.1.1 | `[x]` | `project` crate에 `stellar.yaml` 데이터 구조를 정의합니다. 필수 필드는 format/schema version, project id/name, target profiles, dependencies입니다. | 최소·전체 예제 round-trip 통과. | Phase III |
| IV.1.2 | `[x]` | YAML 파서를 래핑해 다중 문서, custom tag, anchor, alias, merge key를 거부합니다. | 각 금지 문법 fixture가 명확한 오류 코드로 실패. | IV.1.1 |
| IV.1.3 | `[x]` | 중복 mapping key를 위치 정보와 함께 거부합니다. parser가 마지막 값을 덮어쓰는 동작은 금지합니다. | top-level·nested duplicate fixture 실패. | IV.1.2 |
| IV.1.4 | `[x]` | scalar 암묵 타입 추론을 제한합니다. 날짜·yes/no·leading zero 숫자는 문자열로만 허용하거나 명시 오류 처리합니다. | YAML 1.1 혼동 fixture가 기대 타입으로 처리됩니다. | IV.1.2 |
| IV.1.5 | `[x]` | canonical YAML writer를 구현합니다. 키 순서, 들여쓰기 2칸, LF, 마지막 newline을 고정합니다. 주석 보존은 지원하지 않습니다. | 동일 모델을 100회 기록해 byte-identical. | IV.1.1 |
| IV.1.6 | `[x]` | `stellar.lock.yaml` 모델을 별도로 정의합니다. resolved dependency는 logical ID, source, exact commit, content hash, schema version, dependency path를 포함합니다. | 다중 버전 동일 라이브러리 fixture round-trip 통과. | IV.1.1 |
| IV.1.7 | `[x]` | lock writer는 사용자 편집 필드를 받지 않고 전부 resolver 결과에서 생성되게 합니다. | 임의 unknown field가 있는 lockfile을 거부하고 원본을 수정하지 않습니다. | IV.1.6 |

### IV.2 프로젝트 루트 검증

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| IV.2.1 | `[x]` | 플랫폼 공통 폴더 이름 검증기를 구현합니다. Windows 예약 문자·이름, trailing space/dot, 대소문자 충돌, 길이 정책을 적용합니다. | 각 예약 이름과 Unicode NFC fixture 통과. | III.3.4 |
| IV.2.2 | `[x]` | 프로젝트 root는 `stellar.yaml`과 `.git` directory 또는 gitfile이 같은 경로에 있어야 한다는 검증기를 구현합니다. | 정상 repo, linked worktree, 상위 `.git`만 있는 하위 폴더 fixture 결과가 정확합니다. | IV.1.1 |
| IV.2.3 | `[x]` | `ProjectIdentity`를 project StableId와 canonical root path의 조합으로 생성합니다. symlink 경로와 실제 경로 중복 open 판정을 위한 canonical target도 별도 저장합니다. | 같은 target의 두 symlink 경로가 동일 identity target으로 판정됩니다. | III.1.1, IV.2.1 |
| IV.2.4 | `[x]` | 일반 Git repo지만 `stellar.yaml`이 없는 경우 `NotAStellarProject`로 거부합니다. 자동 변환 코드는 만들지 않습니다. | fixture repo가 정확한 안내 오류를 반환합니다. | IV.2.2 |

### IV.3 시스템 Git 탐지 기초

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| IV.3.1 | `[x]` | `git-porcelain`에 Git executable 탐색 순서를 구현합니다: 사용자 지정 절대 경로, 알려진 시스템 경로, PATH. shell을 거치지 않습니다. | 경로 우선순위 fixture 통과. | Phase II |
| IV.3.2 | `[x]` | `git --version`을 실행해 semantic version을 파싱하고 raw output도 보존합니다. 최소 버전은 아직 강제하지 않습니다. | Git 정상·없음·잘못된 executable fixture 처리. | IV.3.1 |
| IV.3.3 | `[x]` | 모든 Git 실행에 argv 배열, cwd, 환경 allowlist, timeout, stdout/stderr byte capture를 사용하는 `GitRunner`를 구현합니다. | shell metacharacter가 명령으로 해석되지 않는 보안 테스트 통과. | IV.3.1 |
| IV.3.4 | `[x]` | Git exit code, stdout, stderr를 보존한 `GitCommandResult`를 정의하고 비영 종료를 자동 문자열 성공으로 바꾸지 않습니다. | known failing command fixture 통과. | IV.3.3 |
| IV.3.5 | `[x]` | `GitInstallationState`를 Available, Missing, UnsupportedVersion, InstallInProgress, ReprobeRequired로 정의합니다. | state transition fixture 통과. | IV.3.2 |
| IV.3.6 | `[x]` | macOS에서 사용자가 설치를 승인한 경우에만 `xcode-select --install`을 시작하는 shell adapter를 구현합니다. 자동 관리자 권한 상승과 무승인 설치는 금지합니다. | 승인 없음 process spawn 0, 승인 시 설치 요청 호출. | IV.3.5, II.5.3 |
| IV.3.7 | `[x]` | Windows에서 사용자가 승인한 경우 `winget install --id Git.Git -e --source winget`을 argv 방식으로 실행하고 WinGet 미존재 시 수동 설치 안내 상태를 반환합니다. | 승인·미승인·winget 없음 fixture 통과. | IV.3.5, II.6.3 |
| IV.3.8 | `[x]` | 설치 프로세스 종료 후 Git executable을 다시 탐지하고 version/capability probe를 재실행합니다. 앱 재시작을 필수로 요구하지 않습니다. | mock install 후 Available 전환. | IV.3.6, IV.3.7 |

### IV.4 새 프로젝트 생성

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| IV.4.1 | `[x]` | `ProjectCreateRequest { parent_dir, folder_name, project_name }`를 정의하고 사전 검증만 수행하는 함수부터 구현합니다. | 존재하는 경로·금지 이름·권한 없음 fixture 실패. | IV.2.1, IV.3.2 |
| IV.4.2 | `[x]` | 검증 후 새 폴더를 생성하고 실패 시 생성한 빈 폴더를 제거하는 filesystem transaction helper를 구현합니다. | 각 단계 fault injection에서 원래 parent가 오염되지 않습니다. | IV.4.1 |
| IV.4.3 | `[x]` | 기본 `stellar.yaml`, 빈 `stellar.lock.yaml`, `.gitignore`, `.gitattributes`, `design/`, `assets/`를 임시 staging directory에 생성합니다. | golden scaffold와 byte-identical. | IV.1.5, IV.1.7, IV.4.2 |
| IV.4.4 | `[x]` | 새 폴더에서 `git init -b main`을 시스템 Git으로 실행합니다. Git이 `-b`를 지원하지 않으면 지원 오류를 내고 대체 명령을 추측하지 않습니다. | branch가 정확히 `main`. | IV.3.4, IV.4.3 |
| IV.4.5 | `[x]` | 기본 파일을 `git add --all`하고 user.name/email 존재를 검사합니다. 누락 시 파일은 남기되 커밋 단계에서 중단 상태를 반환합니다. | identity 있음·없음 fixture 결과 정확. | IV.4.4 |
| IV.4.6 | `[x]` | 커밋 메시지 `New Stellar project initialized`로 최초 커밋을 생성합니다. | `git log -1 --format=%B`가 정확히 일치하고 working tree clean. | IV.4.5 |
| IV.4.7 | `[x]` | 생성 중 Git init 이전 실패와 이후 실패의 rollback 정책을 구현합니다. Git repo 생성 후 commit 실패는 폴더를 삭제하지 않고 `InitializationIncomplete`로 반환합니다. | fault injection 단계별 기대 상태 통과. | IV.4.2~IV.4.6 |

### IV.5 프로젝트 열기

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| IV.5.1 | `[x]` | 경로 선택 후 root 검증, YAML parse, Git root 확인만 수행하는 `ProjectProbe`를 구현합니다. 문서 청크는 아직 읽지 않습니다. | 정상·손상 YAML·Git 불일치 fixture 결과 정확. | IV.2.2, IV.1.7 |
| IV.5.2 | `[x]` | 프로젝트 신뢰 상태를 OS 사용자 데이터 영역에 `ProjectIdentity` 기준으로 저장하는 인터페이스를 정의합니다. 실제 UI는 나중에 연결합니다. | root rename·symlink alias 정책 fixture 통과. | IV.2.3 |
| IV.5.3 | `[x]` | 신뢰되지 않은 프로젝트 probe는 문서 메타데이터만 읽고 Git 실행, 외부 symlink 대상, 필터 실행을 하지 않게 합니다. | 악성 fixture가 외부 프로세스를 실행하지 않습니다. | IV.5.1, IV.5.2 |

### IV.6 Clone 후 열기

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| IV.6.1 | `[x]` | clone request에 URL, parent, folder name을 정의하고 Git URL을 로그·telemetry에서 민감값으로 취급합니다. | 보고서와 로그에 URL 원문이 노출되지 않는 테스트 통과. | IV.3.4, II.3.2 |
| IV.6.2 | `[x]` | 임시 폴더로 `git clone -- <url> <temp>`를 실행하고 성공 후 최종 폴더로 rename합니다. | clone 실패·취소 시 최종 폴더가 존재하지 않습니다. | IV.6.1 |
| IV.6.3 | `[x]` | clone 결과에 `stellar.yaml`이 없으면 열기를 거부하고 clone 결과 보존·삭제 선택은 호출자에게 상태로 반환합니다. 자동 변환은 금지합니다. | 일반 repo fixture에서 `NotAStellarProject`. | IV.6.2, IV.5.1 |
| IV.6.4 | `[x]` | valid Stellar repo는 `ProjectProbe`를 통과한 뒤에만 성공으로 반환합니다. | corrupt manifest clone fixture 실패. | IV.6.3 |

**Phase IV 완료 조건:** 새 프로젝트 생성, 자동 최초 커밋, 기존 프로젝트 probe, clone-open 통합 테스트 통과. G2는 저장 엔진 완료 후 최종 통과 처리합니다.

---

## Phase V. 결정론적 청크 저장, 세대 전환과 스키마 마이그레이션

### V.1 바이너리 인코딩 기준

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| V.1.1 | `[x]` | 저장 primitive의 canonical 규칙 문서를 작성합니다: little-endian, fixed integer width, length-prefixed UTF-8, sorted map key, finite float bits, unknown field policy. 구현 전 golden bytes를 정의합니다. | `docs/fixtures/storage/canonical-primitives.yaml` 존재 및 스키마 검사 통과. | Phase III |
| V.1.2 | `[x]` | `storage`에 canonical encoder를 구현합니다. map 입력 순서와 무관하게 key order로 기록합니다. | 동일 값의 삽입 순서를 바꾼 1,000 fixture가 동일 bytes. | V.1.1 |
| V.1.3 | `[x]` | canonical decoder를 구현하고 length overflow, duplicate key, unknown mandatory tag를 거부합니다. | malformed corpus 전부 panic 없이 오류 반환. | V.1.2 |
| V.1.4 | `[x]` | 저장 float가 NaN·Infinity·negative zero를 포함하면 encoder가 거부 또는 정규화하도록 core 규칙과 일치시킵니다. | primitive golden bytes 일치. | V.1.2, III.3.1 |
| V.1.5 | `[x]` | content hash 알고리즘을 ADR로 고정하고 `ContentHash` newtype과 문자열 형식을 구현합니다. 알고리즘 이름을 hash 값에 포함합니다. | hash test vector와 문자열 parse round-trip 통과. | I.3.5, ADR 승인 |

### V.2 Chunk envelope와 저장소

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| V.2.1 | `[x]` | `ChunkId`를 StableId 기반 불변 논리 ID로 정의합니다. Node StableId와 타입이 섞이지 않게 newtype을 사용합니다. | compile-fail 혼용 테스트 통과. | III.1.1 |
| V.2.2 | `[x]` | chunk envelope에 magic, format version, chunk kind, uncompressed length, payload hash를 정의합니다. 압축은 아직 사용하지 않습니다. | golden header bytes 고정. | V.1.5 |
| V.2.3 | `[x]` | content hash에서 `design/objects/<prefix>/<hash>.starchunk` 경로를 생성합니다. path traversal 문자열 입력은 불가능한 타입으로 제한합니다. | 모든 hash가 고정 길이 안전 경로 생성. | V.1.5 |
| V.2.4 | `[x]` | immutable chunk write를 구현합니다. 같은 hash 파일이 있으면 내용 검증 후 재사용하고 다르면 corruption 오류를 반환합니다. | dedup·collision simulation fixture 통과. | V.2.2, V.2.3 |
| V.2.5 | `[x]` | temp file write, flush, fsync 정책, atomic rename을 플랫폼 abstraction으로 구현합니다. | 각 write 단계 강제 종료 fault injection에서 부분 파일을 current로 읽지 않습니다. | V.2.4 |
| V.2.6 | `[x]` | chunk read 시 envelope, length, hash를 전부 검증합니다. 실패한 bytes를 자동 수정하지 않습니다. | single-bit corruption fixture 전부 탐지. | V.2.4 |
| V.2.7 | `[x]` | chunk kind registry를 추가하고 unknown optional kind는 skip, unknown required kind는 오류가 되게 version policy를 구현합니다. | optional/required unknown fixture 결과 정확. | V.2.2 |

### V.3 논리 ChunkId와 ContentHash 매핑

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| V.3.1 | `[x]` | `ChunkPointer { chunk_id, content_hash, kind, schema_version }`를 정의합니다. | binary round-trip·정렬 통과. | V.2.1, V.1.5 |
| V.3.2 | `[x]` | 동일 ChunkId가 한 generation manifest에서 두 hash를 가리키면 거부하는 validator를 구현합니다. | duplicate pointer fixture 실패. | V.3.1 |
| V.3.3 | `[x]` | 변경된 logical chunk만 새 payload로 encode하고 나머지 pointer를 재사용하는 planner를 구현합니다. | 단일 속성 변경 fixture에서 예상 chunk 하나만 hash 변경. | V.3.1, V.2.4 |
| V.3.4 | `[x]` | chunk split/merge 전에도 외부 참조가 ChunkId를 통해 유지되도록 remap manifest 타입을 추가합니다. 실제 split은 후속 작업입니다. | remap chain cycle·missing target 검증 통과. | V.3.1 |

### V.4 Generation manifest

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| V.4.1 | `[x]` | generation manifest에 generation number, parent generation, project id, schema version, chunk pointers, root document IDs를 정의합니다. | golden manifest round-trip. | V.3.2, IV.1.1 |
| V.4.2 | `[x]` | manifest 자체를 canonical encode하고 content hash를 계산합니다. | 동일 모델 byte-identical. | V.4.1, V.1.2 |
| V.4.3 | `[x]` | current pointer 파일에는 manifest hash 하나만 기록합니다. temp write 후 atomic replace로 전환합니다. | current pointer가 partial 상태를 노출하지 않는 fault test 통과. | V.4.2, V.2.5 |
| V.4.4 | `[x]` | 프로젝트 열기 시 current pointer→manifest→chunks 순으로 검증하는 loader를 구현합니다. | 누락 manifest, 누락 chunk, 잘못된 project id fixture 실패. | V.4.3, V.2.6 |
| V.4.5 | `[x]` | current generation이 손상되면 parent chain과 보존된 manifest 목록에서 최신 정상 세대를 찾는 scanner를 구현합니다. | 최신 2세대 손상 fixture에서 세 번째 정상 세대 선택. | V.4.4 |
| V.4.6 | `[x]` | 정상 세대 fallback 시 읽기 전용 recovery result를 반환하고 자동 current pointer 변경은 하지 않습니다. | open만으로 디스크 bytes가 변하지 않습니다. | V.4.5 |

### V.5 저장 트랜잭션

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| V.5.1 | `[x]` | `SavePlan`에 base generation, target generation, dirty logical chunks, new manifest를 정의합니다. | 동일 문서 상태에서 동일 SavePlan. | V.3.3, V.4.1 |
| V.5.2 | `[x]` | 저장 전 current pointer가 base generation과 같은지 비교하는 optimistic check를 구현합니다. 다르면 write를 시작하지 않습니다. | stale base fixture에서 chunk 파일이 새로 생기지 않습니다. | V.5.1 |
| V.5.3 | `[x]` | dirty chunk를 모두 쓰고 재읽어 검증한 뒤 manifest를 기록하는 단계 실행기를 구현합니다. | 중간 실패 시 current generation 유지. | V.5.2, V.2.5 |
| V.5.4 | `[x]` | 마지막 단계에서 current pointer를 전환하고 성공 후에만 in-memory saved generation을 갱신합니다. | pointer 전환 실패 시 memory generation 미변경. | V.5.3, V.4.3 |
| V.5.5 | `[x]` | 저장 취소 token을 지원하되 pointer 전환 시작 이후에는 취소를 무시하고 완료 또는 실패까지 진행합니다. | 각 단계 cancel fixture 결과 정확. | V.5.3 |
| V.5.6 | `[x]` | save transaction fault matrix를 작성해 chunk write, fsync, manifest write, pointer replace 각 지점의 실패를 자동 주입합니다. | 모든 fixture에서 이전 세대 또는 새 세대 중 하나만 열림. | V.5.4 |

### V.6 청크 경계와 지연 로딩

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| V.6.1 | `[x]` | 초기 chunk kind를 ProjectMetadata, DocumentManifest, ViewTree, ComponentLibrary, TokenSet, AssetMetadata로 정의합니다. | kind registry golden test. | V.2.7 |
| V.6.2 | `[x]` | ViewTree는 의미 있는 subtree root 목록을 포함하고 개별 subtree chunk를 참조하게 스키마를 정의합니다. | 2개 subtree fixture round-trip. | V.6.1 |
| V.6.3 | `[x]` | chunk 목표 크기 측정 harness를 만들고 64KiB, 256KiB, 1MiB 후보의 로드·저장·Git delta 결과를 기록합니다. 결론을 자동 선택하지 않습니다. | raw benchmark와 ADR 입력 보고서 생성. | V.5.6 |
| V.6.4 | `[x]` | 승인된 임계치를 `config/versions.yaml`이 아니라 `config/performance-budgets.yaml`에 기록하고 split planner를 구현합니다. | 경계 바로 아래·위 fixture에서 예상 split. | V.6.3, ADR 승인 |
| V.6.5 | `[x]` | 작은 인접 chunk를 idle 작업에서 merge하는 planner를 구현하되 current save와 동시에 실행되지 않게 합니다. | merge 전후 의미 snapshot 동일. | V.6.4 |
| V.6.6 | `[x]` | 프로젝트 열기에서 DocumentManifest까지만 먼저 읽고 ViewTree subtree는 요청 시 읽는 lazy loader를 구현합니다. | 비활성 view chunk에 read syscall이 발생하지 않는 계측 테스트. | V.4.4, V.6.2 |

### V.7 Garbage collection

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| V.7.1 | `[x]` | 보존 generation 정책 인터페이스를 만들고 기본값은 current+직전 정상 세대+복구에 필요한 세대로 둡니다. 수치는 config에서 읽습니다. | policy fixture 통과. | V.4.5 |
| V.7.2 | `[x]` | 보존 manifest에서 reachable content hash를 mark하는 scanner를 구현합니다. | shared chunk가 한 generation 삭제로 제거되지 않습니다. | V.7.1 |
| V.7.3 | `[x]` | unreferenced chunk를 grace period 이후 sweep하는 작업을 구현합니다. 파일 삭제 실패는 다음 실행으로 이월합니다. | active chunk 미삭제, orphan 삭제 fixture 통과. | V.7.2 |
| V.7.4 | `[x]` | GC는 프로젝트 open 직후가 아니라 idle+저장 없음 조건에서만 시작하도록 scheduler interface를 추가합니다. | save 중 GC 시작하지 않는 동시성 테스트. | V.7.3 |

### V.8 스키마 버전과 마이그레이션

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| V.8.1 | `[x]` | format_version, schema_version, minimum_reader_version, minimum_writer_version 타입과 호환성 판정 함수를 구현합니다. | 읽기 전용·편집 가능·거부 matrix test 통과. | V.4.1 |
| V.8.2 | `[x]` | `MigrationStep { from, to, migrate }` registry를 만들고 연속 경로가 없으면 거부합니다. | gap·cycle·duplicate step fixture 실패. | V.8.1 |
| V.8.3 | `[x]` | noop v1→v2 test migration을 만들어 원본 세대를 수정하지 않고 새 generation으로 결과를 기록합니다. | 원본 hash 유지, 새 generation 생성. | V.8.2, V.5.4 |
| V.8.4 | `[x]` | migration 전체를 temp generation에서 수행하고 모든 검증 후 current pointer를 전환합니다. | 단계 중간 실패 시 원본 current 유지. | V.8.3 |
| V.8.5 | `[x]` | migration 결과를 자동 commit하지 않고 Git working tree 변경으로 남기는 통합 fixture를 추가합니다. | `git status`에 expected files만 변경. | V.8.4, IV.3.4 |
| V.8.6 | `[x]` | migration golden fixture runner를 `cargo xtask test-migrations`로 추가합니다. | 모든 과거 fixture가 최신 snapshot으로 정확히 변환. | V.8.3 |

### V.9 복구 스냅샷 저장 기초

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| V.9.1 | `[x]` | 프로젝트 저장소 밖 OS app data에 `RecoverySnapshotStore` 인터페이스를 정의합니다. 키는 ProjectIdentity와 WindowSessionId입니다. | 경로에 프로젝트명·텍스트가 노출되지 않습니다. | IV.2.3, V.1.2 |
| V.9.2 | `[x]` | 문서 미저장 snapshot, 열린 탭, viewport, 선택, target profile을 canonical recovery payload로 정의합니다. 터미널·Git 실행 상태는 포함하지 않습니다. | payload round-trip. | V.9.1 |
| V.9.3 | `[x]` | 정상 저장 generation보다 recovery revision이 최신인지 비교하는 probe를 구현합니다. | 오래된 snapshot은 제안하지 않습니다. | V.9.2, V.4.4 |
| V.9.4 | `[x]` | 정상 종료 marker와 snapshot cleanup을 구현합니다. 비정상 종료 simulation에서는 snapshot이 남습니다. | 정상·비정상 fixture 결과 정확. | V.9.3 |

### V.10 의미 단위 텍스트 원본 전환 (ADR-0059)

기존 완료 행을 재정의하지 않습니다. V.1·V.2·V.3·V.6의 canonical codec·ChunkId·무결성·지연 로딩은 파생 cache에 재사용합니다. V.4·V.5·V.7·V.8의 generation 원본 경로는 기존 프로젝트 호환용으로 유지하고 V.10.7 검증 뒤 역할을 전환합니다. V.9의 프로젝트 밖 복구 snapshot은 계속 유지합니다.

V.10.1~V.10.3은 단일 View 텍스트 원본이라는 한 제품 개선입니다. §1.2.13의 파일 수 상한을 유지하기 위해 I/O, 문서 왕복, CLI 연결을 아래 세 하위 작업으로 나눕니다. 각 하위 작업은 명시 파일과 테스트·문서만 바꾸며 별도 커밋으로 검증합니다.

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| V.10.1 | `[x]` | `crates/storage/src/{source_file,atomic_file,lib}.rs`에서 제한 handle 읽기·기대 hash 확인·단일 파일 원자 저장·권한 보존·파생 cache 교체를 제공합니다. 기존 정책을 유지하고 실패 시 원본을 보호합니다. | `[DEV-MAC]` 생성 충돌·stale hash·rename 전 fault·권한 보존·fsync 후 오류 의미 검사, storage 테스트와 전체 Rust 검증. | V.2.5 |
| V.10.2 | `[x]` | `crates/project/src/document_io/{text_source,text_codec,mod}.rs`에서 단일 문서·페이지·View 및 Frame/Fragment 전체 속성·StableId의 제한 YAML 왕복과 선택적 chunk cache 재생성을 구현합니다. 미지원 의미를 거부하고 ADR-0059·문법·호환 절차를 갱신합니다. | `[DEV-MAC]` 실제 모델 편집→저장→재열기→Git diff, 미지원 필드·외부 변경 보존, 캐시 없음·손상·I/O 실패, 문서 예시 검증. | V.10.1, VI.10.3 |
| V.10.3 | `[~]` | `crates/ops/src/{text_source,lib}.rs`와 `tools/stellar-cli/src/{text_source,cli,lib}.rs`에 명시적 export/check를 연결합니다. 기존 generation 전체 pointer 집합을 비교해 미지원 chunk를 거부합니다. 기본 열기·저장은 바꾸지 않습니다. | `[DEV-MAC]` 실제 CLI export→check→재export 거부, 기존 generation bytes 보존, unknown optional·미사용 chunk 거부, `cargo xtask check-core`. | V.10.2, XXIV.1.3 |
| V.10.4 | `[ ]` | 전체 영속 의미(모든 node·modifier·component·token·resource)를 텍스트화하고 문서·페이지·View·큰 Frame 파일 참조 및 이름/StableId 규칙을 완성합니다. 미지원 필드·레코드는 보존하거나 쓰기를 거부합니다. 기존 generation 변환은 전체 semantic snapshot이 같을 때만 성공합니다. | `[DEV-MAC]` 전체 golden 의미 왕복·미지원 입력 보존·독립 View 변경의 Git 병합; 공개 포맷 동결 전 검증. | V.10.3 |
| V.10.5 | `[ ]` | 여러 의미 파일의 temp/fsync/rename 저장 journal과 앞/새 세대 한쪽 복구를 구현합니다. 삭제·rename·디스크 오류도 원본과 미저장 작업을 보호합니다. | `[DEV-MAC]` 단계별 종료·오류 fault matrix, 재열기에서 섞인 세대 0건. | V.10.4 |
| V.10.6 | `[~]` | 텍스트 원본을 파일 감시·외부 변경·명령 저장 및 재열기 경로에 연결합니다. 첫 부분은 opt-in 단일 파일 CLI apply의 source hash·필수 잠금·명령 transaction·dry-run 및 조건부 저장입니다. 열린 뒤 외부 편집이나 git pull이 있으면 dirty 편집을 보존하고 재로드·병합하는 전체 연결은 후속입니다. | `[DEV-MAC]` CLI dry-run→저장→재열기→Git diff, clean/dirty 외부 변경 및 cache 무효화 통합 시나리오. | V.10.5, XIII.3.1 |
| V.10.7 | `[ ]` | 검증된 전체 변환 경로에서만 기본 원본을 텍스트로 전환하고 기존 chunk 저장은 로컬 cache로 내립니다. source hash 기반 빠른 열기·부분 로딩과 cache GC를 연결합니다. | `[DEV-MAC]` 기존 프로젝트 전체 semantic 왕복·복구·외부 변경 모두 통과. `[REF-MAC]` 실제 통합 T0→T5 및 열기·저장 예산 실측 후 전환. | V.10.6 |

**Phase V 완료 조건:** 저장 fault matrix, migration golden test, 정상 세대 fallback을 통과합니다.

---

## Phase VI. 문서 도메인 모델과 그래픽 노드

### VI.1 문서·페이지·View 정의

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VI.1.1 | `[x]` | `document`에 `ProjectDocumentIndex`, `DocumentDefinition`, `PageDefinition`, `ViewDefinition` 타입을 추가합니다. 각 타입은 고유 StableId와 표시 이름을 가집니다. | 생성·삭제·binary round-trip 테스트 통과. | Phase V, III.1.1 |
| VI.1.2 | `[x]` | Document→Page→View 소유 관계를 명시하고 한 View는 정확히 하나의 root Frame을 갖게 합니다. | root 없음·root 두 개 fixture 거부. | VI.1.1 |
| VI.1.3 | `[x]` | 표시 이름은 비어 있을 수 있지만 ID와 저장 경로에 사용하지 않도록 API를 분리합니다. | 이름 변경 후 참조·chunk 경로가 변하지 않습니다. | VI.1.1 |
| VI.1.4 | `[x]` | 현재 열려 있는 문서와 저장된 문서 index를 분리하는 `OpenDocumentSet` 런타임 타입을 추가합니다. | 같은 문서를 중복 open할 때 기존 handle 반환. | VI.1.1, III.6.3 |
| VI.1.5 | `[x]` | 최대 10개 열린 문서 제한을 config로 검사하고 초과 시 기존 문서를 자동 닫지 않습니다. | 10개 성공, 11번째 명확한 오류. | VI.1.4 |

### VI.2 공통 Node 레코드

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VI.2.1 | `[x]` | `NodeKind`에 Frame, Fragment, Shape, Path, Text, Image, ComponentInstance를 정의합니다. ComponentDefinition은 scene node가 아닌 resource로 유지합니다. | numeric kind golden test. | VI.1.2 |
| VI.2.2 | `[x]` | 공통 Node column에 StableId, kind, parent RuntimeEntityId, OrderKey, visible, locked, display name을 추가합니다. | column invariant와 round-trip 통과. | III.6.2, VI.2.1 |
| VI.2.3 | `[x]` | parent-child 삽입 validator를 구현합니다. 자기 자신·자손을 부모로 지정하거나 다른 View root로 무단 이동하는 것을 거부합니다. | cycle·cross-view fixture 실패. | VI.2.2 |
| VI.2.4 | `[x]` | 자식 조회는 OrderKey 정렬 결과를 반환하고 arena 삽입 순서에 의존하지 않게 합니다. | 삽입 순서가 다른 동일 tree fixture의 child order 동일. | VI.2.2, III.5.2 |
| VI.2.5 | `[x]` | node 삭제 전 참조 inbound 목록을 조회하는 `ReferenceIndex` 인터페이스를 추가합니다. 삭제 정책은 command 단계에서 처리합니다. | inbound/outbound fixture 정확. | III.2.3, VI.2.2 |
| VI.2.6 | `[x]` | node count와 depth 계산을 O(1) 또는 incremental metadata로 유지할 수 있는 훅을 추가합니다. 실제 성능 최적화는 후속 작업입니다. | insert/move/delete 후 count/depth 정확. | VI.2.3 |

### VI.3 Frame

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VI.3.1 | `[x]` | Frame payload에 size, local transform, clipping, background, border, corner radius, layout mode 참조를 정의합니다. | 기본값 round-trip. | VI.2.2, III.3.3, III.4.1 |
| VI.3.2 | `[x]` | Frame만 자식의 layout containing block과 좌표계를 만들 수 있게 capability validator를 추가합니다. | Fragment가 containing block으로 사용되면 오류. | VI.3.1 |
| VI.3.3 | `[x]` | width/height가 음수가 되지 않게 하고 transform을 통한 반전과 size 음수를 구분합니다. | 음수 size 거부, mirror transform 허용. | VI.3.1 |
| VI.3.4 | `[x]` | clip path가 없을 때 frame bounds clip, 별도 PathRef clip 두 형식을 정의합니다. | missing clip ref는 unresolved diagnostic. | VI.3.1, III.2.4 |
| VI.3.5 | `[x]` | Frame property metadata registry를 작성해 각 PropertyKey, 타입, 기본값, layout/paint invalidation 범위를 고정합니다. | registry와 실제 payload 필드 누락 검사 통과. | VI.3.1, III.2.1 |

### VI.4 Fragment

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VI.4.1 | `[x]` | Fragment payload는 별도 렌더·layout 필드를 갖지 않는 zero-size marker로 구현합니다. | payload serialized bytes가 metadata 외 추가 필드 없음. | VI.2.2 |
| VI.4.2 | `[x]` | Fragment에 transform, opacity, blend, mask, filter, fill, border, padding 설정을 시도하면 validation 오류를 반환합니다. | 금지 속성 fixture 전부 실패. | VI.4.1 |
| VI.4.3 | `[x]` | Fragment의 visual bounds는 가시 자식 bounds의 union으로 계산하는 read-only helper를 구현합니다. | 빈 fragment, 숨김 자식, nested fragment fixture 통과. | VI.4.1 |
| VI.4.4 | `[x]` | Fragment→Frame 변환용 순수 변환 계획을 구현합니다. 새 Frame은 Fragment StableId를 유지하고 필요한 기본 payload를 추가합니다. | 자식 순서와 참조가 유지됩니다. | VI.4.1, VI.3.1 |

### VI.5 Shape

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VI.5.1 | `[x]` | ShapeKind에 Rectangle, Ellipse, Line, Polygon을 정의합니다. 별도 node kind를 늘리지 않습니다. | kind variant round-trip. | VI.2.1 |
| VI.5.2 | `[x]` | Rectangle 파라미터에 size와 개별 corner radius를 정의하고 radius를 bounds에 clamp하는 평가 함수를 구현합니다. 저장값은 원본을 유지합니다. | oversize radius reference fixture 통과. | VI.5.1, III.3.3 |
| VI.5.3 | `[x]` | Ellipse는 bounds로 정의하고 start/end angle을 지원하지 않는 초기 범위로 고정합니다. 해당 필드가 파일에 있으면 unknown field 정책을 따릅니다. | full ellipse path reference와 일치. | VI.5.1 |
| VI.5.4 | `[x]` | Line은 start/end point와 cap style을 정의합니다. zero-length line 정책을 점 cap으로 명시합니다. | cap별 bounds fixture 통과. | VI.5.1 |
| VI.5.5 | `[x]` | Polygon은 side count≥3, rotation, corner radius를 정의합니다. | 2 sides 거부, 3·4·100 sides path fixture 통과. | VI.5.1 |
| VI.5.6 | `[x]` | Shape를 Path geometry로 평가하는 순수 함수를 구현합니다. node를 Path로 변환하지 않습니다. | 동일 입력 bit-stable path commands. | VI.5.2~VI.5.5 |

### VI.6 Path

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VI.6.1 | `[x]` | Path command에 MoveTo, LineTo, CubicTo, Close만 정의합니다. Quadratic은 import 단계에서 Cubic으로 정규화합니다. | command round-trip. | III.3.3 |
| VI.6.2 | `[x]` | subpath와 winding rule을 정의하고 시작 MoveTo 없는 command를 거부합니다. | malformed path fixture 실패. | VI.6.1 |
| VI.6.3 | `[x]` | Path point와 handle에 StableId를 부여해 개별 point diff가 가능하게 합니다. | point 삽입 후 기존 point ID 유지. | VI.6.1, III.1.1 |
| VI.6.4 | `[x]` | Path bounds 계산을 구현하고 cubic extrema를 포함합니다. | analytic reference fixture 허용 오차 통과. | VI.6.1 |
| VI.6.5 | `[x]` | Shape→Path 파괴 변환 계획을 구현합니다. 사용자가 명시한 command에서만 node kind를 바꾸고 StableId는 유지합니다. | 변환 전 렌더 geometry 동일. | VI.5.6, VI.6.1 |

### VI.7 Text

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VI.7.1 | `[x]` | Text payload에 UTF-8 content, paragraph list, style runs를 정의합니다. run 경계는 byte index가 아니라 Unicode scalar/grapheme 정책을 명시한 text index 타입을 사용합니다. | 한글 조합, emoji ZWJ, surrogate 불가 fixture round-trip. | VI.2.2, III.3.4 |
| VI.7.2 | `[x]` | style run에 typography ref, fill, OpenType features, variation axes를 정의합니다. | overlapping run을 canonical non-overlap run으로 normalize. | VI.7.1, III.3.5 |
| VI.7.3 | `[x]` | paragraph alignment, line height, letter spacing, wrapping mode를 정의합니다. 실제 layout은 Phase X에서 구현합니다. | 기본값·경계값 validation 통과. | VI.7.1 |
| VI.7.4 | `[x]` | 빈 텍스트도 유효 node로 유지하고 삭제와 구분합니다. | empty content 저장·복원. | VI.7.1 |
| VI.7.5 | `[x]` | 텍스트 내용 변경 시 style run range를 보존·축소·확장하는 deterministic edit helper를 구현합니다. | 삽입·삭제·replace fixture 통과. | VI.7.2 |

### VI.8 Image

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VI.8.1 | `[x]` | Image payload에 AssetRef, fit mode, crop rect, sampling mode, color space override를 정의합니다. | round-trip. | VI.2.2, III.2.3 |
| VI.8.2 | `[x]` | missing AssetRef 상태를 invalid node가 아니라 missing resource diagnostic으로 표현하고 placeholder bounds는 유지합니다. | 에셋 누락 전후 layout bounds 동일. | VI.8.1, III.2.4 |
| VI.8.3 | `[x]` | crop rect는 normalized 0..1 범위로 clamp 평가하되 저장값 validation에서 범위 밖을 거부합니다. | 경계값 fixture 통과. | VI.8.1 |

### VI.9 Modifier Stack

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VI.9.1 | `[x]` | `ModifierId`를 StableId newtype으로 정의하고 ordered modifier list를 node payload와 분리 저장합니다. | ID와 순서 round-trip. | III.1.1, III.5.2 |
| VI.9.2 | `[x]` | 초기 modifier kind를 Transform, Boolean, StrokeExpand, Corner, Mask, Clip, Fill, Stroke, Shadow, Blur, Blend, Opacity로 정의합니다. | numeric kind golden test. | VI.9.1 |
| VI.9.3 | `[x]` | 각 modifier에 enabled flag와 typed payload를 정의하고 disabled modifier는 평가에서 skip하되 저장에는 유지합니다. | enable/disable 결과 fixture 통과. | VI.9.2 |
| VI.9.4 | `[x]` | Boolean·Mask·Clip의 다른 NodeRef 의존성을 ReferenceIndex에 등록합니다. | 참조 추가·제거 시 inbound index 정확. | VI.2.5, VI.9.2 |
| VI.9.5 | `[x]` | modifier 참조 graph cycle detector를 구현합니다. direct/self/indirect cycle을 저장 전 거부합니다. | cycle fixture 전부 실패. | VI.9.4 |
| VI.9.6 | `[x]` | Fragment에 modifier 삽입을 거부합니다. | Fragment modifier command fixture 실패. | VI.4.2, VI.9.1 |
| VI.9.7 | `[x]` | modifier property metadata와 geometry/layout/paint invalidation class를 정의합니다. | registry 누락 검사 통과. | VI.9.2 |

### VI.10 문서 불변식과 직렬화

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VI.10.1 | `[x]` | 전체 document validator를 구현합니다: ID uniqueness, parent consistency, root count, OrderKey uniqueness, ref type, modifier cycle, depth≤64. | 각 불변식 위반 fixture가 정확한 위치 오류. | VI.1~VI.9 |
| VI.10.2 | `[x]` | document→chunk model serializer를 구현하고 runtime ID를 저장하지 않습니다. | save bytes에 RuntimeEntityId pattern 없음. | VI.10.1, V.6.2 |
| VI.10.3 | `[x]` | chunk model→document loader를 구현하고 ID registry와 parent order를 재구축합니다. | 100개 golden document round-trip 의미 동일. | VI.10.2 |
| VI.10.4 | `[x]` | canonical semantic snapshot JSON을 테스트 전용으로 생성합니다. 프로덕션 저장에는 사용하지 않습니다. | 양 플랫폼 fixture diff가 byte-identical. | VI.10.3 |
| VI.10.5 | `[x]` | random valid document generator를 추가하고 1,000 seed round-trip property test를 workspace 통합 검증(`cargo xtask check-core`)의 smoke로 포함합니다. | panic, 데이터 손실, 비결정성 0건. | VI.10.3 |

**Phase VI 완료 조건:** 빈 View와 기본 Node 조합의 생성·검증·저장·재로드가 통과합니다. G3의 문서 모델 부분 완료.

---

## Phase VII. Command, 트랜잭션과 창별 Undo/Redo

### VII.1 Command 공통 규격

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VII.1.1 | `[x]` | `CommandId`, `WindowSessionId`, `CommandOrigin`을 정의합니다. Origin은 User, System, Migration, FutureAgent를 포함하되 FutureAgent 동작은 구현하지 않습니다. | round-trip·타입 혼용 compile-fail. | Phase VI |
| VII.1.2 | `[x]` | `Command` trait 대신 직렬화 가능한 closed enum을 사용하고 `validate`, `apply`, `invert` 인터페이스를 정의합니다. | unknown command kind 거부. | VII.1.1 |
| VII.1.3 | `[x]` | 모든 command에 base DocumentRevision과 precondition 목록을 포함합니다. | revision mismatch fixture 실패. | II.3.4, VII.1.2 |
| VII.1.4 | `[x]` | command 결과에 changed nodes, changed properties, invalidation classes, new revision을 반환합니다. | 단일 property command expected change set 일치. | VII.1.2, VI.9.7 |
| VII.1.5 | `[x]` | apply 도중 오류가 나면 변경 전 arena state로 되돌리는 transaction guard를 구현합니다. 전체 document clone은 금지합니다. | 중간 실패 fault injection 후 semantic snapshot 동일. | VII.1.4 |
| VII.1.6 | `[x]` | 여러 command를 묶는 `CommandTransaction`을 구현하고 전부 성공하거나 전부 롤백하게 합니다. | 두 번째 command 실패 시 첫 번째 효과 없음. | VII.1.5 |

### VII.2 기본 문서 Command

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VII.2.1 | `[x]` | `InsertNode`를 구현합니다. caller가 StableId를 제공하며 duplicate, invalid parent, invalid OrderKey를 검증합니다. | apply/inverse round-trip. | VII.1.6, VI.2.3 |
| VII.2.2 | `[x]` | `DeleteSubtree`를 구현합니다. 삭제 전 subtree와 내부 참조를 inverse payload에 보존합니다. 외부 inbound ref가 있으면 정책 오류를 반환합니다. | delete→undo semantic snapshot 동일. | VII.2.1, VI.2.5 |
| VII.2.3 | `[x]` | `MoveNode`를 구현합니다. parent와 OrderKey를 함께 변경하고 cycle·cross-view를 검증합니다. | move→undo, reorder fixture 통과. | VII.2.1 |
| VII.2.4 | `[x]` | `SetProperty`를 구현합니다. metadata registry로 type과 invalidation을 검사합니다. | 잘못된 PropertyKey/type 거부. | VII.1.4, VI.3.5 |
| VII.2.5 | `[x]` | `SetVisibility`, `SetLocked`, `RenameNode`를 별도 command가 아니라 SetProperty specialization으로 노출합니다. | 동일 core patch 경로 사용 확인. | VII.2.4 |
| VII.2.6 | `[x]` | `InsertModifier`, `RemoveModifier`, `MoveModifier`, `SetModifierProperty`를 각각 구현합니다. | 각 apply/inverse fixture 통과. | VI.9.1~VI.9.7, VII.1.6 |
| VII.2.7 | `[x]` | `ConvertFragmentToFrame`과 `ConvertShapeToPath` command를 구현해 Phase VI 변환 계획을 적용합니다. | undo 후 원본 bytes 의미 복원. | VI.4.4, VI.6.5, VII.1.6 |
| VII.2.8 | `[x]` | `DuplicateSubtree`를 구현하고 IdRemapTable로 내부 ID/참조를 새로 생성합니다. | 원본과 중복 ID 없음, 내부 참조 새 subtree. | III.1.5, VII.2.1 |

### VII.3 선택 상태 Command

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VII.3.1 | `[x]` | 선택 상태를 document model이 아니라 `WindowViewState`에 저장합니다. selected StableId ordered set와 primary selection을 정의합니다. | 선택이 save bytes에 포함되지 않습니다. | VII.1.1, VI.1.4 |
| VII.3.2 | `[x]` | `SetSelection` history event를 구현하고 존재하지 않거나 숨김 정책상 선택 불가 ID를 걸러냅니다. | apply/inverse와 primary selection 규칙 통과. | VII.3.1 |
| VII.3.3 | `[x]` | 영역 선택 중간 상태는 transient preview로 유지하고 pointer up 때 최종 SetSelection 한 건만 생성하는 grouping API를 추가합니다. | 1,000 move event에서 history 1건. | VII.3.2 |
| VII.3.4 | `[x]` | node 삭제 시 각 WindowViewState에서 삭제된 ID를 selection에서 제거하는 system event를 적용하되 사용자 history 항목은 만들지 않습니다. | 여러 창 fixture에서 stale selection 없음. | VII.2.2, VII.3.2 |

### VII.4 History 항목과 그룹화

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VII.4.1 | `[x]` | `HistoryEntry`에 origin, forward transaction, inverse transaction, grouping key, timestamp 대신 monotonic sequence를 정의합니다. | 직렬화는 세션 메모리 전용이며 project serializer에서 제외. | VII.1.6, II.3.3 |
| VII.4.2 | `[x]` | pointer gesture begin/update/end API를 구현합니다. update는 같은 preview transaction을 교체하고 end에서 한 entry를 확정합니다. | drag 500 update가 history 1건. | VII.4.1 |
| VII.4.3 | `[x]` | 텍스트 입력 grouping은 동일 Text node, 인접 caret, 입력 종류, 짧은 manual-clock interval 조건으로 합칩니다. interval 값은 config에 둡니다. | 조건별 merge/split fixture 통과. | VI.7.5, VII.4.1 |
| VII.4.4 | `[x]` | 키보드 이동·resize 반복은 같은 command signature와 interval 조건으로 합칩니다. | key repeat 20회 history 1건. | VII.4.1 |
| VII.4.5 | `[x]` | slider/scrub begin/end grouping을 구현합니다. 취소 시 preview 전 상태로 돌아가고 history를 만들지 않습니다. | commit/cancel fixture 통과. | VII.4.2 |
| VII.4.6 | `[x]` | Selection event와 document edit를 동일 시간순 스택에 넣습니다. Undo는 마지막 종류와 무관하게 한 entry씩 되돌립니다. | selection→edit→selection 순서 undo fixture 통과. | VII.3.2, VII.4.1 |

### VII.5 창별 Undo/Redo

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VII.5.1 | `[x]` | `WindowHistory`를 WindowSessionId별로 생성하고 다른 창 entry를 포함하지 않습니다. | 창 A/B 교차 편집에서 stack 분리. | VII.4.6 |
| VII.5.2 | `[x]` | Undo는 해당 창의 inverse transaction을 현재 창의 in-memory document state에 적용합니다. 다른 창과 모델을 공유하지 않는 현재 정책을 반영합니다. | 각 창 독립 snapshot fixture 통과. | VII.5.1 |
| VII.5.3 | `[x]` | Redo stack은 Undo 후 같은 창에서 새 command가 확정되면 폐기합니다. 다른 창 저장 변화는 해당 창 history를 즉시 폐기하지 않습니다. | branch history fixture 통과. | VII.5.2 |
| VII.5.4 | `[x]` | 창 닫기 시 WindowHistory와 selection을 폐기하고 project 파일에는 기록하지 않습니다. | reopen 후 history 비어 있음. | VII.5.1 |
| VII.5.5 | `[x]` | Undo/Redo 중 생성되는 selection restoration은 새 history entry를 만들지 않게 reentrancy guard를 추가합니다. | undo 100회 후 stack size 예상 일치. | VII.5.2 |

### VII.6 Command property test

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VII.6.1 | `[x]` | 각 command에 `apply(forward); apply(inverse)`가 원본 semantic snapshot으로 돌아오는 property test macro를 작성합니다. | 모든 Phase VII command 등록 검사 통과. | VII.2, VII.3 |
| VII.6.2 | `[x]` | random valid command sequence 1,000개를 생성하고 전체 undo 후 초기 상태, 전체 redo 후 최종 상태를 확인합니다. | seed corpus 전부 통과. | VII.6.1, VII.5.3 |
| VII.6.3 | `[~]` | command 실행 중 panic·out-of-bounds가 없는 fuzz target을 추가합니다. | 정해진 CI fuzz smoke duration 무오류. | VII.1.5 |

**Phase VII 완료 조건:** G4 통과.

---

## Phase VIII. 토큰, 표현식과 통합 Binding Graph

### VIII.1 표현식 타입 시스템

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VIII.1.1 | `[x]` | `ExpressionType`을 TypedValue와 1:1 대응하도록 정의하고 nullable은 일반 타입과 별도 `Optional<T>`로 표현합니다. | type mapping 완전성 검사. | III.3.5 |
| VIII.1.2 | `[x]` | 산술 type rule table을 코드와 문서로 정의합니다: length+length, length×scalar 등 허용 조합만 등록합니다. | 모든 허용·금지 조합 table-driven test. | VIII.1.1 |
| VIII.1.3 | `[x]` | 비교·논리·조건식의 type rule을 정의합니다. `if` 두 branch 타입은 같거나 명시적 공통 타입이어야 합니다. | mismatch fixture 오류 위치 정확. | VIII.1.1 |
| VIII.1.4 | `[x]` | color, string, min/max/clamp/round/lerp/remap 함수 signature registry를 정의합니다. | unknown 함수·인자 수·타입 오류 fixture 실패. | VIII.1.2, III.4.2 |
| VIII.1.5 | `[x]` | 파일·네트워크·시간·난수·반복·재귀를 표현식 모델에 넣을 수 없게 closed AST로 유지합니다. | serialized malicious unknown op 거부. | VIII.1.1 |

### VIII.2 Expression AST

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VIII.2.1 | `[x]` | 모든 AST node에 StableId, node kind, typed child refs를 정의합니다. | ID uniqueness와 tree/DAG validation 통과. | VIII.1.1, III.1.1 |
| VIII.2.2 | `[x]` | Literal, Reference, Unary, Binary, Call, If, Match AST node를 구현합니다. | 각 node round-trip. | VIII.2.1 |
| VIII.2.3 | `[x]` | AST는 DAG 참조를 허용하되 cycle을 금지합니다. cycle detector를 구현합니다. | direct·indirect cycle fixture 실패. | VIII.2.2 |
| VIII.2.4 | `[x]` | type checker가 각 AST node의 inferred type과 오류 path를 반환하게 합니다. | nested 오류가 정확한 node StableId를 포함. | VIII.1.4, VIII.2.2 |
| VIII.2.5 | `[x]` | canonical AST serializer는 map 순서가 아니라 child slot 순서를 사용합니다. | GUI 생성 순서가 다른 동등 AST bytes 동일. | VIII.2.2, V.1.2 |

### VIII.3 GUI 편집용 Projection

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VIII.3.1 | `[x]` | AST를 GUI block 목록과 edge 목록으로 읽는 `ExpressionGraphView`를 구현합니다. 별도 원본 상태를 만들지 않습니다. | AST→view→AST 의미 동일. | VIII.2.5 |
| VIII.3.2 | `[x]` | GUI에서 허용 가능한 child socket 타입과 연결 가능 여부 API를 추가합니다. | 잘못된 socket 연결 거부. | VIII.2.4 |
| VIII.3.3 | `[x]` | block 추가·연결·삭제를 command plan으로 생성하는 API를 추가합니다. 실제 UI는 Phase XII에서 연결합니다. | 각 plan 적용 후 valid AST. | VIII.3.2, Phase VII |
| VIII.3.4 | `[x]` | AST validation 오류를 GUI block ID와 socket ID로 매핑합니다. | 오류 위치 fixture 정확. | VIII.3.1, VIII.2.4 |

### VIII.4 Bytecode와 평가기

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VIII.4.1 | `[x]` | 검증된 AST를 stack 또는 register bytecode로 컴파일하는 내부 포맷을 정의합니다. bytecode는 저장하지 않고 캐시로만 사용합니다. | AST와 bytecode evaluator 결과 동일 fixture. | VIII.2.4 |
| VIII.4.2 | `[x]` | literal·산술·비교·논리 evaluator를 구현합니다. division by zero는 typed evaluation error입니다. | table-driven test 통과. | VIII.4.1 |
| VIII.4.3 | `[x]` | if·match short-circuit 평가를 구현해 선택되지 않은 branch 오류가 실행되지 않게 합니다. | short-circuit fixture 통과. | VIII.4.2 |
| VIII.4.4 | `[x]` | color·string·수치 함수 evaluator를 구현합니다. | reference value fixture 통과. | VIII.4.2, VIII.1.4 |
| VIII.4.5 | `[x]` | evaluator에 instruction budget과 recursion 없는 call depth limit를 적용합니다. | 비정상적으로 큰 AST가 제한 오류로 종료. | VIII.4.1 |
| VIII.4.6 | `[~]` | 동일 input environment와 bytecode가 bit-stable TypedValue를 반환하는 결정론 테스트를 양 플랫폼에서 실행합니다. | macOS/Windows fixture bytes 동일. | VIII.4.4 |

### VIII.5 토큰 모델

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VIII.5.1 | `[x]` | `TokenSet`, `TokenDefinition`, `TokenName`, `TokenType`, default expression을 정의합니다. 참조는 StableId를 사용합니다. | rename 후 참조 유지. | VIII.2.5 |
| VIII.5.2 | `[x]` | 프로젝트·라이브러리·문서 scope token registry를 구현하고 동일 이름 허용 여부와 ID 우선 resolution을 명시합니다. | scope shadowing fixture 통과. | VIII.5.1 |
| VIII.5.3 | `[x]` | token dependency graph를 구축하고 cycle을 저장 전 거부합니다. | A→B→A fixture 실패. | VIII.5.1, VIII.2.3 |
| VIII.5.4 | `[x]` | token value evaluation cache를 구현하고 dependency revision이 바뀐 token만 invalidate합니다. | 무관 token 변경 시 평가 count 0. | VIII.4.6, VIII.5.3 |
| VIII.5.5 | `[x]` | token 생성·삭제·rename·expression 변경 command를 구현합니다. inbound 참조가 있는 삭제는 orphan plan 없이는 거부합니다. | apply/inverse 및 ref policy 통과. | VIII.5.1, Phase VII |

### VIII.6 Binding Graph

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VIII.6.1 | `[x]` | `ValueBinding`에 target PropertyPath, source expression root, expected type, scope owner를 정의합니다. | round-trip·type validation. | III.2.2, VIII.2.5 |
| VIII.6.2 | `[x]` | binding dependency graph가 token, property, component property, environment input을 공통 node로 표현하게 합니다. | dependency 종류별 fixture 통과. | VIII.6.1, VIII.5.3 |
| VIII.6.3 | `[x]` | binding cycle detector를 구현하고 Property A↔B, token↔property cycle을 모두 탐지합니다. | mixed cycle fixture 실패. | VIII.6.2 |
| VIII.6.4 | `[x]` | source 변경 시 영향받는 binding만 topological order로 재평가하는 scheduler를 구현합니다. | 무관 subtree 평가 count 0, 순서 deterministic. | VIII.6.2, VIII.4.6 |
| VIII.6.5 | `[x]` | evaluated value provenance chain에 token, expression node, component property, environment branch를 기록합니다. | 최종값→원본 chain fixture 정확. | VIII.6.4 |
| VIII.6.6 | `[x]` | binding 생성·교체·삭제 command를 구현합니다. type mismatch는 command validate에서 거부합니다. | apply/inverse와 invalid type fixture 통과. | VIII.6.1, Phase VII |

### VIII.7 환경 입력과 Target 조건

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VIII.7.1 | `[x]` | environment key registry에 platform, OS/API version, color scheme, contrast, reduced motion, viewport, locale, density, interaction state를 정의합니다. | key numeric ID golden test. | VIII.1.1 |
| VIII.7.2 | `[x]` | `EvaluationEnvironment`를 immutable map으로 정의하고 missing key의 기본값 정책을 key registry에 고정합니다. | missing·present fixture 통과. | VIII.7.1 |
| VIII.7.3 | `[x]` | environment 변경 시 해당 key를 참조한 expression만 invalidate하는 reverse index를 구현합니다. | color scheme 변경 시 viewport-only expression 미평가. | VIII.6.4, VIII.7.2 |

### VIII.8 Orphaned Binding과 기본값

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VIII.8.1 | `[x]` | target/source ref가 사라진 binding을 삭제하지 않고 `OrphanedBinding`으로 변환하는 validator를 구현합니다. 원래 ID·타입·path·원인을 보존합니다. | 삭제·타입 변경 fixture 정보 보존. | VIII.6.1, III.2.4 |
| VIII.8.2 | `[x]` | orphaned binding 평가 결과는 대상 Component Property의 선언된 기본값을 사용하게 합니다. 마지막 계산값 저장은 금지합니다. | 재실행 후 동일 기본값. | VIII.8.1 |
| VIII.8.3 | `[x]` | 기본값이 없거나 잘못된 파일은 타입별 안전값과 corruption diagnostic을 사용하되 프로젝트 파일을 자동 수정하지 않습니다. | malformed fixture open 가능·오류 표시. | VIII.8.2 |
| VIII.8.4 | `[x]` | orphaned binding 재연결과 명시 삭제 command를 구현합니다. | 재연결 후 live value, undo 후 orphan 상태. | VIII.8.1, Phase VII |

### VIII.9 성능·결정론 검증

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| VIII.9.1 | `[x]` | 100만 token/binding edge synthetic graph 생성기를 추가합니다. 활성 document 한도 fixture는 20만 node 기준으로 별도 생성합니다. | 생성 결과 seed-deterministic. | VIII.6.4 |
| VIII.9.2 | `[x]` | 단일 token 변경의 evaluation count와 duration을 측정하는 benchmark를 추가합니다. | 결과 JSON에 affected/visited/evaluated count 존재. | VIII.9.1 |
| VIII.9.3 | `[~]` | 양 플랫폼 evaluator golden corpus를 CI에 추가합니다. | 모든 result bytes 동일. | VIII.4.6, VIII.8.3 |

**Phase VIII 완료 조건:** 토큰·표현식·Binding의 생성, 증분 평가, provenance, orphan fallback이 통과합니다.

---

## Phase IX. Component, Property Drilling, 상태 머신, View Variant와 접근성

### IX.1 Component Definition과 Property

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| IX.1.1 | `[x]` | `ComponentDefinition` resource에 StableId, root FrameRef, 공개 Property 목록, Slot 목록, local token scope를 정의합니다. | root가 component 밖 node를 가리키면 거부. | Phase VIII, Phase VI |
| IX.1.2 | `[x]` | `ComponentProperty`에 StableId, 이름, type, 필수 default expression, 공개 여부를 정의합니다. default 없는 property는 생성 불가입니다. | default type mismatch 거부. | IX.1.1, VIII.2.5 |
| IX.1.3 | `[x]` | Property 생성·rename·default 변경·삭제 command를 구현합니다. inbound drilling이 있는 삭제는 orphan 정보를 생성합니다. | apply/inverse 및 orphan fixture 통과. | IX.1.2, VIII.8.1, Phase VII |
| IX.1.4 | `[x]` | component property의 runtime evaluated value를 instance별로 캐시하고 definition revision과 override revision으로 invalidate합니다. | 한 instance override 변경 시 다른 instance cache 유지. | IX.1.2, VIII.6.4 |

### IX.2 Component Instance

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| IX.2.1 | `[x]` | ComponentInstance node payload에 ComponentRef, sparse PropertyOverride map, SlotOverride map을 정의합니다. | round-trip. | IX.1.1, VI.2.1 |
| IX.2.2 | `[x]` | instance expansion view를 구현해 definition tree와 instance override를 합성한 read-only virtual subtree를 제공합니다. 실제 node 복제는 금지합니다. | 5만 instance에서 definition node count가 물리 복제되지 않습니다. | IX.2.1 |
| IX.2.3 | `[x]` | 공개 Property만 instance에서 override 가능하게 validator를 구현합니다. private·nested property 직접 접근은 거부합니다. | 직접 nested access fixture 실패. | IX.1.2, IX.2.1 |
| IX.2.4 | `[x]` | override 생성·변경·reset command를 구현합니다. reset은 map entry를 삭제하고 default로 돌아갑니다. | apply/inverse와 sparse 저장 확인. | IX.2.3, Phase VII |
| IX.2.5 | `[x]` | 자유 구조 편집은 금지하고 definition이 허용한 Slot만 변경 가능하게 virtual subtree mutation validator를 구현합니다. | nested child delete/move 거부. | IX.2.2 |
| IX.2.6 | `[x]` | 명시적 detach command를 구현해 virtual subtree를 실제 nodes로 복제하고 ComponentRef를 제거합니다. 새 ID remap을 사용합니다. | detach 후 시각 의미 동일, undo 복원. | IX.2.2, III.1.5, Phase VII |

### IX.3 Slot

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| IX.3.1 | `[x]` | `ComponentSlot`에 StableId, host FrameRef, allowed node kinds, cardinality, default content refs를 정의합니다. | host가 definition 밖이면 거부. | IX.1.1 |
| IX.3.2 | `[x]` | Slot override는 instance가 소유한 nodes를 slot에 투영하도록 정의하고 definition tree 안으로 parent를 직접 바꾸지 않습니다. | save/load 후 ownership 유지. | IX.3.1, IX.2.1 |
| IX.3.3 | `[x]` | slot content 삽입·삭제·재정렬 command를 구현합니다. allowed kind/cardinality를 검증합니다. | invalid kind·overflow fixture 실패. | IX.3.2, Phase VII |

### IX.4 명시적 Property Drilling

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| IX.4.1 | `[x]` | component boundary를 넘는 binding은 각 경계의 ComponentProperty StableId chain을 필수로 갖게 합니다. | chain 누락 fixture 거부. | IX.1.2, VIII.6.1 |
| IX.4.2 | `[x]` | nested instance property를 상위 instance에서 직접 수정하는 API를 제공하지 않고 validator에서도 거부합니다. | prohibited direct path fixture 실패. | IX.2.3 |
| IX.4.3 | `[x]` | 사용자가 내부 property를 상위로 노출할 때 필요한 중간 property와 binding chain을 계산하는 `DrillingPlan`을 구현합니다. 아직 적용하지 않습니다. | 1·2·10단계 nesting fixture의 계획 정확. | IX.4.1 |
| IX.4.4 | `[x]` | DrillingPlan preview에 생성될 property 이름, type, default, component 경로를 포함합니다. 이름 충돌은 deterministic suffix를 제안합니다. | 동일 입력 동일 plan. | IX.4.3 |
| IX.4.5 | `[x]` | 승인된 DrillingPlan을 하나의 CommandTransaction으로 적용합니다. 중간 실패 시 property 하나도 남기지 않습니다. | fault injection rollback. | IX.4.4, VII.1.6 |
| IX.4.6 | `[x]` | chain 일부 삭제·타입 변경 시 상위 chain 전체를 orphan 상태로 표시하되 저장에서 삭제하지 않습니다. | break point와 affected chain fixture 정확. | IX.4.5, VIII.8.1 |

### IX.5 선언형 상태 머신

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| IX.5.1 | `[x]` | component local state variable에 StableId, name, type, initial expression을 정의합니다. | initial type mismatch 거부. | VIII.2.5, IX.1.1 |
| IX.5.2 | `[x]` | EventKind에 Activate, Input, Focus, Blur, Key, SelectionChange, Open, Close를 정의합니다. | kind golden test. | IX.5.1 |
| IX.5.3 | `[x]` | Transition에 source event, condition expression, ordered actions를 정의합니다. action은 state set과 semantic action emit으로 제한합니다. | file/network action 표현 불가. | IX.5.2, VIII.2.5 |
| IX.5.4 | `[x]` | runtime state store를 preview session별로 유지하고 project 저장에는 definition·initial state만 포함합니다. | preview 조작 후 save bytes 동일. | IX.5.1 |
| IX.5.5 | `[x]` | 이벤트 dispatch와 condition 평가, action commit을 원자적으로 구현합니다. | action 중 오류 시 state 변화 없음. | IX.5.3, IX.5.4 |
| IX.5.6 | `[x]` | nested component state는 직접 수정할 수 없고 공개 Property/Event chain을 통해서만 연결되게 validator를 구현합니다. | direct nested state write fixture 실패. | IX.4.2, IX.5.3 |
| IX.5.7 | `[x]` | state machine의 unreachable transition, duplicate condition, missing target를 진단합니다. | diagnostic fixture 정확. | IX.5.3 |

### IX.6 Control Contract

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| IX.6.1 | `[x]` | Control을 node kind가 아닌 Frame에 부착하는 `SemanticContract`와 `InteractionContract`로 정의합니다. | contract 제거 후 일반 Frame으로 유지. | VI.3.1, IX.5.2 |
| IX.6.2 | `[x]` | Button contract에 activate, focused, disabled, pressed 의미를 정의합니다. | 필수 action 누락 diagnostic. | IX.6.1 |
| IX.6.3 | `[x]` | TextInput contract에 editable value, placeholder, focus, IME input 의미를 정의합니다. | editable binding type이 string이 아니면 오류. | IX.6.1, VI.7.1 |
| IX.6.4 | `[x]` | Modal contract에 overlay, focus scope, dismiss event 의미를 정의합니다. | focus scope root 누락 diagnostic. | IX.6.1 |
| IX.6.5 | `[x]` | List contract에 collection, item identity, selection mode, navigation 의미를 정의합니다. | duplicate item identity fixture 오류. | IX.6.1 |
| IX.6.6 | `[x]` | SingleSelectionGroup contract를 추가해 List+Button 합성 Radio UI를 표현합니다. 별도 RadioButton node는 만들지 않습니다. | 한 번에 둘 이상 selected인 runtime state 거부. | IX.6.2, IX.6.5 |
| IX.6.7 | `[x]` | contract별 유효성 registry와 target export hint를 정의합니다. 코드 생성은 하지 않습니다. | registry 누락 검사 통과. | IX.6.2~IX.6.6 |

### IX.7 View 변수와 Nullable 조합형 Variant

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| IX.7.1 | `[x]` | ViewVariable에 StableId, type, default value, allowed enum values를 정의합니다. | invalid enum default 거부. | VI.1.2, VIII.1.1 |
| IX.7.2 | `[x]` | VariantCondition은 변수별 `Any` 또는 `Equals(value)`만 저장합니다. `Any`와 실제 nullable 값은 별도 tag로 구분합니다. | ExplicitNull/Any fixture 구분. | IX.7.1 |
| IX.7.3 | `[x]` | VariantOverrideSet에 sparse property patches, node add/delete/move/replace, slot changes를 정의합니다. | round-trip·reference validation. | VI.2, IX.3 |
| IX.7.4 | `[x]` | 현재 변수 assignment와 일치하는 모든 variant를 찾고 specificity 순으로 정렬하는 resolver를 구현합니다. | 일반+구체 조합 fixture 순서 정확. | IX.7.2 |
| IX.7.5 | `[x]` | 동일 specificity variant가 같은 PropertyPath를 다르게 override하면 conflict diagnostic을 생성하고 임의 순서로 적용하지 않습니다. | ambiguity fixture 실패. | IX.7.4 |
| IX.7.6 | `[x]` | 구체 variant가 일반 variant 값을 덮어쓰고 존재하지 않는 조합은 추가 override 없이 base+일반 variant만 적용하게 합니다. | 요구 예제 fixture 통과. | IX.7.4 |
| IX.7.7 | `[x]` | variant structure override도 ComponentInstance 내부 캡슐화를 침범하지 못하게 validator를 연결합니다. | nested direct override fixture 실패. | IX.7.3, IX.2.5 |
| IX.7.8 | `[x]` | View variable/variant 생성·변경·삭제 command를 구현합니다. 삭제 시 참조 variant는 orphan diagnostic으로 남깁니다. | apply/inverse 통과. | IX.7.1~IX.7.7, Phase VII |
| IX.7.9 | `[x]` | 캔버스에 같은 View를 여러 variable assignment로 배치하는 `ViewPresentation` 런타임 타입을 정의합니다. definition을 복제하지 않습니다. | 100 presentations가 definition 1개 공유. | IX.7.4 |

### IX.8 접근성 의미 트리

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| IX.8.1 | `[x]` | `AccessibilityNode`에 source StableId, role, label binding, value binding, state, action, reading order를 정의합니다. | round-trip. | IX.6.1, VIII.6.1 |
| IX.8.2 | `[x]` | 시각 tree와 별도 accessibility parent/child 관계를 허용하되 source node는 정확히 하나여야 합니다. | orphan source·cycle fixture 실패. | IX.8.1 |
| IX.8.3 | `[x]` | decorative node 제외, Fragment flatten, hidden node 제외 규칙을 적용한 기본 accessibility tree generator를 구현합니다. | expected tree fixture 통과. | IX.8.2, VI.4.1 |
| IX.8.4 | `[x]` | Button, TextInput, Modal, List, SingleSelectionGroup contract를 role/state/action으로 매핑합니다. | control별 golden tree 통과. | IX.6.7, IX.8.3 |
| IX.8.5 | `[x]` | label 누락, invalid nesting, focusable action 없음, duplicate reading order diagnostic을 구현합니다. | diagnostic fixture 정확. | IX.8.4 |
| IX.8.6 | `[x]` | Target adapter가 사용할 중립 accessibility snapshot을 정의합니다. 플랫폼 API 호출은 Phase XII로 미룹니다. | 양 플랫폼 snapshot bytes 동일. | IX.8.4 |

### IX.9 통합 결정론 fixture

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| IX.9.1 | `[~]` | Button, TextInput, Modal, List, Radio 합성 예제 component library fixture를 만듭니다. | document validator 0 오류. | IX.6, IX.8 |
| IX.9.2 | `[ ]` | 10단계 nested component property drilling fixture를 만듭니다. | 최종 value provenance가 전체 chain 포함. | IX.4.5 |
| IX.9.3 | `[x]` | compact/error/dark nullable variant 조합 fixture를 만듭니다. | 예상 override resolution과 ambiguity diagnostic 통과. | IX.7.6 |
| IX.9.4 | `[ ]` | 위 fixture를 macOS·Windows CI에서 semantic snapshot 비교합니다. | byte-identical. | IX.9.1~IX.9.3 |

**Phase IX 완료 조건:** G5 통과.

---

### IX.10 Target Profile과 Capability Registry

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| IX.10.1 | `[x]` | `TargetProfileId`, profile kind Web/Apple/Android, profile schema version을 정의합니다. | numeric kind와 round-trip golden test. | IV.1.1, VIII.7.1 |
| IX.10.2 | `[x]` | Apple profile에 platform 종류와 최소 OS version, Android profile에 API level과 UI system version, Web profile에 browser baseline과 capability set version을 정의합니다. | profile별 required field validation. | IX.10.1 |
| IX.10.3 | `[~]` | `stellar.yaml`에 project target profile 목록과 기본 profile을 저장하고 존재하지 않는 default를 거부합니다. | manifest round-trip과 invalid default fixture. | IX.10.2, IV.1.5 |
| IX.10.4 | `[~]` | Stellar 앱 업데이트만으로 project profile version을 바꾸지 않게 합니다. profile registry version과 project pin을 분리합니다. | registry update 후 project bytes·resolved profile unchanged. | IX.10.3 |
| IX.10.5 | `[x]` | capability 상태를 Native, Approximated, Unsupported, Deprecated로 정의하고 element/property/semantic contract별 registry를 구현합니다. | 상태 matrix fixture 통과. | IX.10.2, IX.6.7 |
| IX.10.6 | `[x]` | selected TargetProfile에 따라 element·property 생성 가능 여부와 비활성화 사유를 조회하는 API를 구현합니다. 실제 UI는 Phase XII에 연결합니다. | Button/TextInput/Modal/List와 대표 CSS/Apple/Android property fixture 통과. | IX.10.5 |
| IX.10.7 | `[x]` | 공통 structure에 profile-specific sparse override를 적용하는 resolver를 구현하고 View Variant·Binding environment와 결합합니다. | 공통값·profile override·variant override 우선순위 fixture 통과. | IX.7.6, VIII.7.3, IX.10.5 |
| IX.10.8 | `[x]` | 한 View를 여러 TargetProfile에 대해 동시에 validate하고 Native/Approximated/Unsupported/Deprecated diagnostic을 생성합니다. | multi-profile fixture expected diagnostics. | IX.10.6, IX.10.7 |
| IX.10.9 | `[~]` | profile upgrade plan을 구현해 old/new capability registry의 semantic diff와 영향받는 nodes/properties를 반환합니다. 적용 전 project를 수정하지 않습니다. | upgrade preview fixture revision unchanged. | IX.10.4, IX.10.8, XIV.3.2 |
| IX.10.10 | `[~]` | 승인된 profile upgrade를 CommandTransaction과 manifest 변경으로 적용하고 자동 commit하지 않습니다. | apply/undo, Git working tree change fixture 통과. | IX.10.9, Phase VII |

**Phase IX 추가 완료 조건:** TargetProfile pin, capability filtering, 다중 profile validation을 통과합니다.

### IX.11 CSS 기반 속성 체계

> 기준: ADR-0024, `docs/design/property_model.md`(이하 "속성 모델"). 디자인 속성의 이름·값 문법·단위·레이아웃 의미는 CSS longhand를 따르고, 저장은 바이너리 chunk와 숫자 `PropertyKey`를 유지합니다. CSS 텍스트를 파일에 저장하지 않습니다.
>
> 선행 해석: IX.11과 IX.12는 Phase IX 완료 뒤에 추가한 절입니다. 완료 행(`[x]`·`[~]`)의 선행 열에 있는 `Phase IX`(X.1.1)는 그 행을 완료할 때 있던 IX.1\~IX.10만 뜻하고 IX.11·IX.12를 포함하지 않습니다. IX.11 행 일부가 Phase X 이후 행을 선행으로 두므로, 이 규칙이 없으면 X.1.1을 지나는 선행 순환이 생깁니다.

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| IX.11.1 | `[ ]` | `crates/document`에 registry 항목별 CSS 매핑 metadata 타입을 정의합니다. 필드는 CSS longhand 이름 목록, 값 문법 종류, 매핑 상태(`exact`·`converted`·`approximated`·`stellar-only`), `initial-differs`·`subset` 표시, 차이 설명입니다. `Length`=`px`, `Angle`=`deg`, `Duration`=`ms`, `Percentage`=`%` 단위 계약을 `core-types` 문서 주석에 고정합니다. `crates/core-types/`는 I.3.5 보호 경로이므로 같은 diff에서 ADR-0008을 개정해 단위 계약을 기록합니다(새 ADR도 허용). 저장 bytes는 바꾸지 않습니다. | 타입 golden test, CSS longhand 이름 inventory 상수(이 작업에서 추가)의 중복·형식 검사 test 통과, storage golden 불변, 같은 diff의 ADR 존재, `cargo xtask validate-adr` 통과. | VI.3.5, VI.9.7, IX.11.22 |
| IX.11.2 | `[ ]` | Frame·Modifier metadata registry의 모든 항목에 속성 모델 5장·9장 대응을 채웁니다. registry `name`을 CSS longhand 또는 `stellar-only` 이름으로 맞추고 semantic diff 표시 문자열을 같은 이름으로 바꿉니다. 묶음 key(IX.11.22)는 대응 longhand 목록으로 표시합니다. `clipping`(0x0103)은 CSS longhand로 나누지 않는 Stellar 고유 이름의 단일 속성으로 두고(ADR-0024 결정 14), 확인된 clip 기준(border box와 바깥 corner radius, `crates/scene/src/builder.rs` `FrameClipInput::Bounds`)과 변형별 CSS 대응(Disabled `overflow: visible`, FrameBounds `overflow: clip` + `overflow-clip-margin: border-box`, Path `clip-path`)을 변환 규칙과 차이 설명으로 기록합니다. Mask(0x1051)는 alpha·luminance mode가 정해지기 전(속성 모델 9장 Mask 행, 15장 O14) 매핑 상태를 `approximated`로 기록합니다. | 매핑 누락 0 검사, 속성 모델 표와 registry snapshot 일치 test, PropertyKey 숫자와 storage golden 불변. | IX.11.1, IX.11.22 |
| IX.11.3 | `[ ]` | Shape·Path·Text·Image property metadata registry를 Frame registry(VI.3.5)와 같은 방식으로 추가합니다. node kind 상위 byte 규칙(Frame `0x01xx`, Image `0x06xx`)으로 longhand마다 PropertyKey 하나를 할당하고(ADR-0024 결정 14) 기존 key는 재사용하지 않습니다. Shape·Image의 `width`·`height` key는 IX.11.6 item record가 sizing longhand를 어디에 저장하든 그대로 쓰는 key로 할당합니다. PropertyKey는 payload bytes에 담기지 않으므로 저장 위치와 독립입니다(속성 모델 6장). 각 항목에 속성 모델 6장·7장 CSS 매핑을 채웁니다. semantic snapshot이 임시로 쓰는 전용 key(`0xE3xx`·`0xE4xx`·`0xE6xx`, `crates/semantic-diff/src/keys.rs`)를 새 문서 key로 바꾸고 diff golden을 갱신합니다. Path command·point 목록과 Text content는 payload 단위 key를 유지하고, Text style longhand key는 IX.11.11이 같은 규칙으로 할당합니다(속성 모델 15장 O12). | payload 필드와 registry 1:1 누락 검사, key 중복 0, CSS 매핑 누락 0, 문서 key가 생긴 필드의 snapshot 전용 key 사용 0, 갱신한 diff golden 목록 기록. | IX.11.1, IX.11.22, VI.5.6, VI.6.2, VI.7.3, VI.8.1, XIV.3.1 |
| IX.11.4 | `[ ]` | layout 엔진 Stack 의미를 CSS Flexbox에 맞춥니다. `flex-shrink` 기본 1, `align-items`·`align-content` 생략값 `normal`(stretch), 자식별 `align-self`, 축 고정 `row-gap`·`column-gap` 입력, stretch 대상 제한을 구현합니다. stretch(`normal` 포함)는 cross 크기가 `auto`인 자식(속성 모델 4.3절 Fill)만 늘리고, HugContent(`fit-content`)·Fixed·Percentage 자식은 `flex-start`처럼 놓습니다. 지금 엔진은 `Stretch`에서 cross 축이 HugContent인 자식도 늘립니다(`crates/layout/src/stack.rs` `child_constraint`). flex item 자동 최소 크기는 IX.11.23이 맡습니다. 바뀐 X.11.1 corpus 기대값은 다섯 변경별로 나눠 fixture별 변경 이유와 함께 갱신합니다. | 속성 모델 4.2절 행별 fixture 통과, 기본 Stack(`align-items` 생략) 안 HugContent 자식이 cross 축으로 늘어나지 않고 시작 쪽에 놓이는 fixture, 변경별 갱신 fixture 목록이 보고서와 일치, 같은 입력 결정론 test 통과. | IX.11.1, X.3.5, X.6.4 |
| IX.11.5 | `[ ]` | Grid·Free·Overlay 의미를 CSS에 맞춥니다. grid item 생략 정렬을 `normal`(stretch)로 하고 `justify-self`·`align-self`를 추가합니다. grid item stretch도 크기가 `auto`인 축만 영역 크기로 늘리고 크기가 `auto`가 아닌 자식(Fixed·Percentage·HugContent)은 시작 쪽에 놓습니다. track 최솟값 `auto`(item 최소 기여)를 추가해 bare `fr`을 CSS `minmax(auto, <n>fr)`로 해석하고, 기존 `TrackSize::Fraction`은 `minmax(0, <n>fr)`과 같게 둡니다. fr 크기 계산은 CSS 규칙으로 맞춥니다. fr 가중치 합이 1보다 작으면 1로 나누고, 가용 크기가 indefinite이면 factor가 1 이하인 track은 base나 기여를 factor로 나누지 않습니다. 지금 엔진은 합과 관계없이 남는 공간 전체를 비례 분배합니다(`crates/layout/src/grid_tracks.rs` 규칙 4, 속성 모델 4.4절). Free containing block을 padding box로 바꾸고 백분율 위치·크기의 기준도 content rect(X.2.3)에서 padding box로 바꾸며, anchor 없는 축은 시작 anchor 0으로 해석합니다. absolute 자식이 부모 Hug 크기에 기여하지 않게 합니다. Overlay 정렬 이름을 `justify-items`·`align-items`로 맞추고, `OverlayAlignment`(지금 Start·Center·End, `crates/layout/src/overlay.rs`)에 stretch를 더해 생략값을 CSS initial value(`normal`, stretch)로 바꿉니다. stretch는 크기가 `auto`인 축만 영역 크기로 늘리고 크기가 `auto`가 아닌 자식(Fixed·Percentage·HugContent)은 시작 쪽에 놓습니다. `grid-auto-rows`의 CSS initial value `auto`는 지금 track 어휘로 표현할 수 없으므로(엔진 기본 `TrackSize::MaxContent`, 속성 모델 4.4절 `implicit_row` 행) `auto` track을 추가할지, 영속 record가 `grid-auto-rows`를 생략 없이 명시 `max-content`로 저장할지를 이 행에서 정해 보고서에 기록하고 IX.11.6이 따릅니다. 영속 record 생략값은 CSS initial value, 새 노드 생성 preset은 명시 값입니다. | 속성 모델 4.4절·4.5절 행별 fixture 통과, Overlay·Grid 생략 정렬에서 크기 `auto` 자식이 영역을 채우고 Fixed·HugContent 자식이 시작 쪽에 놓이는 fixture, fr 합이 1보다 작은 Grid(`0.5fr` 단독, `0.25fr 0.25fr`)의 definite·indefinite fixture, `grid-auto-rows` 생략·명시 fixture가 고른 규칙과 일치, bare `fr`과 `minmax(0, <n>fr)`의 콘텐츠 최소 크기 차이 fixture, padding이 있는 부모의 백분율 위치·크기 fixture, anchor 없는 자식 fixture, 갱신 corpus 목록과 이유 기록, HugContent cycle 진단 fixture 유지. | IX.11.1, X.2.3, X.4.5, X.5.1, X.6.5 |
| IX.11.6 | `[ ]` | 레이아웃 container·item 속성을 CSS longhand로 저장하는 영속 record와 schema version을 정의합니다. 생략값은 CSS initial value입니다. container 저장 위치(Frame payload 확장 또는 `layout_mode` 참조 resource)와 Frame `size` 흡수 방식을 이 작업의 ADR로 정합니다. item longhand(inset, `flex-*`, `align-self`·`justify-self`, `grid-*-start`·`-end`, sizing longhand)는 Frame만이 아니라 상자를 가진 모든 node 종류(Frame, Shape, Path, Text, Image, ComponentInstance)의 자식에 붙습니다. 지금 Frame이 아닌 node에는 위치 필드가 없으므로(`crates/document/src/records.rs` `NodeRecord`, Rectangle은 크기만 가짐) item record 저장 위치(NodeRecord 공통 필드 또는 node별 layout item record)와 Shape·Image payload 크기의 흡수 방식도 같은 ADR로 정합니다. 상자가 없는 Fragment는 item이 아니고 그 자식이 item입니다(아키텍처 §12.4). Ellipse·Polygon `bounds` 원점은 shape local 좌표의 기하 offset으로, Transform modifier 이동 성분은 layout 뒤에 적용하는 시각 변형으로 그대로 두고 위치 longhand로 migration하지 않으며, 새 위치 편집은 item record 위치 longhand를 씁니다(속성 모델 12장 Position 행). longhand key는 ADR-0024 결정 14로 할당하되, `width`·`height`는 IX.11.22(Frame)·IX.11.3(Shape·Image)이 할당한 key를 그대로 쓰고 아직 key가 없는 node 종류에만 새로 할당합니다. schema version 증가는 IX.11.21이 정한 묶음 규칙을 따릅니다. | ADR 존재, record round-trip과 canonical bytes golden, 같은 node 종류의 `width`·`height` key가 하나씩(IX.11.22·IX.11.3 key와 같음), Frame·Rectangle·Ellipse·Text·Image·ComponentInstance 자식의 item record round-trip, bounds 원점이 0이 아닌 Ellipse와 Transform modifier가 있는 Rectangle의 화면 위치가 item record 추가 전후 같음, unknown field 정책 fixture, `cargo xtask test-migrations` 통과. | IX.11.4, IX.11.5, IX.11.21, IX.11.22, V.8.6 |
| IX.11.7 | `[ ]` | sizing preset(Fixed·Percentage·HugContent·Fill)과 longhand 사이의 순수 변환 함수를 구현합니다. preset→longhand는 속성 모델 4.3절 표를 따르고, longhand→preset은 표와 정확히 일치할 때만 preset을 돌려주며 아니면 Custom입니다. | 부모 방식·축별 왕복 fixture, Custom 판정 fixture, 같은 입력 같은 결과. | IX.11.6 |
| IX.11.8 | `[ ]` | CSS 레이아웃 적합성 fixture를 추가합니다. Flexbox 60개, Grid 40개, positioning 30개 이상이며 각 fixture는 CSS 선언과 기대 border box rect를 가집니다. 기대값은 Chromium 계열 고정 버전 브라우저 하나(1/64 px LayoutUnit, 속성 모델 1.3절)로 생성한 JSON을 커밋하고, 생성 절차와 브라우저 버전을 fixture README에 기록합니다. 브라우저와 생성 스크립트는 빌드·테스트 의존성이 아닌 개발 도구이며, 도입 전에 `config/dependencies.yaml` 승인 절차(계획 1장 14번)를 거칩니다. 테스트는 브라우저를 실행하지 않습니다. 텍스트는 고정 intrinsic 크기 대체 요소로 둡니다. 기본 fixture는 남는 공간이 1/64 px 격자에서 나누어떨어지는 입력만 쓰고, 잔여 분배·가운데 정렬 반올림 차이는 별도 `distribution` 묶음으로 둡니다. 두 축 indefinite `aspect-ratio`, 한 축만 definite인 `aspect-ratio` Frame 안 넘치는 콘텐츠(14장 E10, 비율 축 `min-*: 0px`을 명시한 선언은 일치), 음수 남는 공간의 `space-around`·`space-evenly`, bare `fr`, fr 합이 1보다 작은 Grid, 기본 Stack(`align-items: normal`) 안 `fit-content` 자식, flex item 자동 최소 크기 fixture를 포함합니다. | 기본 fixture rect가 1 LayoutUnit 안에서 일치, `distribution` 묶음은 같은 줄 item 수 이하 LayoutUnit 차이, 속성 모델 14장 확장 fixture만 예외 표시, `space-around`·`space-evenly` 확인 결과를 속성 모델 4.2절에 기록, fixture count 검사 통과. | IX.11.4, IX.11.5, IX.11.23, X.11.1 |
| IX.11.9 | `[ ]` | Frame 위치와 transform 분리 migration을 IX.11.21 registry step으로 구현합니다. 행렬의 이동 성분만 위치로 옮기고 선형 부분은 `transform: matrix(a, b, c, d, 0, 0)`으로 bit 그대로 보존하며 `rotate`·`scale`로 분해해 저장하지 않습니다(ADR-0024 결정 8). 지금 저장 형식에는 layout 정의 record가 없어 모든 자식이 Free 자식이므로 이동 성분은 `left`·`top`으로 옮기고, 부모가 flow 방식인 입력이면 `translate`로 옮깁니다. 부모가 없는 Frame(View root Frame `ViewDefinition.root_frame`, ComponentDefinition root)의 이동 성분은 페이지 캔버스 배치이므로 위치 longhand가 아니라 x·y 두 longhand로 된 stellar-only 캔버스 배치 속성으로 옮기고(속성 모델 4.6절·5장), Web export는 이 속성을 출력하지 않습니다. migration된 노드에는 `transform-origin: 0 0`을 명시 저장하고, 새 노드 기본 origin은 `50% 50%`입니다. 새 longhand key는 ADR-0024 결정 14로 할당하고 0x0102는 묶음 key로 남기되 이 schema 이후 새 command로 받지 않습니다. 0x0102를 가리키는 저장 참조(ValueBinding target, View Variant·Component variant override property patch)는 `TypedValue`·`ExpressionType`에 Transform 타입이 없어 값 변환을 하지 않고, bytes를 보존한 채 migration 보고서와 IX.11.22 진단에 나열합니다. layout 결과 위치(부모가 없는 Frame은 캔버스 배치 값)와 `translate`·`rotate`·`scale`·`transform`·`transform-origin`을 CSS 순서로 합성해 scene `NodeInput.transform`을 만드는 함수와 `rotate` 각도→행렬 sin·cos 구현(속성 모델 15장 O13)을 정하고, O13 결정과 근거를 보고서에 기록합니다. 인스펙터 위치 X·Y 편집 경로를 새 속성으로 바꿉니다. Frame payload schema 변경은 storage format 변경이므로(I.3.5) 같은 diff에서 ADR을 작성하거나 ADR-0024를 개정합니다. | migration golden fixture에서 선형 부분(a, b, c, d) bit 동일과 화면 위치 1 LayoutUnit 이내 일치, Free 자식·flow 자식(`translate`) fixture, View root Frame과 ComponentDefinition root의 이동 성분이 캔버스 배치 속성으로 옮겨지고 위치 longhand·`translate`가 생기지 않는 fixture, O13 결정과 근거가 보고서에 있고 고른 sin·cos 규칙의 fixture(0·90·180·270도 등 직각 배수 각도의 bit 결과 포함) 통과, 같은 diff의 ADR 존재와 `cargo xtask validate-adr` 통과, binding·View Variant·Component variant override 참조를 담은 fixture의 참조 bytes 보존과 보고서 목록 일치, 원본 generation hash 유지, 자동 commit 없음, `cargo xtask test-migrations` 통과. | IX.11.6, IX.11.21, IX.11.22, X.1.2, V.8.6 |
| IX.11.10 | `[ ]` | Frame border를 4변 `border-*-width`·`-style`·`-color` longhand로 바꾸는 migration을 IX.11.21 registry step으로 구현합니다. 기존 `Border{color, thickness}`는 4변 같은 값과 style `solid`로, 없음은 `none`으로 옮기고, 12개 longhand에 ADR-0024 결정 14로 key를 할당하며 0x0105는 묶음 key로 남기되 이 schema 이후 새 command로 받지 않습니다. border가 CSS처럼 padding box를 줄이도록 layout 입력에 border 두께를 넣고, 화면 위치를 보존하려고 border가 있는 부모의 Free 자식 `left`·`top`에서 부모 border 두께를 뺍니다(IX.11.9 뒤 적용). IX.11.6 이전 schema에서 온 입력에는 layout record가 없어 모든 자식이 Free 자식입니다. 입력 schema에 영속 layout record(IX.11.6)가 있어 border가 있는 부모가 flow 방식(Stack·Grid·Overlay)이면, 부모의 각 변 `padding-*`에서 그 변 border 두께를 빼 flow 자식의 위치와 부모 Hug 크기를 보존합니다. padding이 border 두께보다 작은 변은 padding을 `0px`로 두고 보존하지 못한 차이를 노드별로 migration 보고서에 기록합니다(속성 모델 5장 `border` 행). 이런 중간 schema 입력의 Free 자식은 지정된 `right`·`bottom`도 그 변 border 두께로 같은 방식으로 보정하고, 백분율 inset·크기처럼 길이 보정으로 보존할 수 없는 값은 보고서에 기록합니다. 0x0105를 가리키는 저장 참조(index segment 포함)는 IX.11.9와 같은 규칙으로 값 변환 없이 보존·보고합니다. Frame payload schema 변경은 storage format 변경이므로(I.3.5) 같은 diff에서 ADR을 작성하거나 ADR-0024를 개정합니다. | migration golden fixture 외형 동일(위치는 1 LayoutUnit 이내), layout record가 있는 중간 schema 입력에서 border가 있는 Stack 부모의 flow 자식 위치와 Hug 크기가 보존되는 fixture, padding이 border 두께보다 작은 변의 보고서 기록 fixture, 4변 다른 값 round-trip, border가 있는 Stack 자식 위치 fixture가 IX.11.8 기대값과 일치, border가 있는 부모의 Free 자식 화면 위치 보존 fixture, 참조 bytes 보존 fixture, 같은 diff의 ADR 존재와 `cargo xtask validate-adr` 통과. | IX.11.6, IX.11.9, IX.11.21, V.8.6 |
| IX.11.11 | `[ ]` | Typography record를 CSS longhand(`font-family`, `font-size`, `font-weight`, `font-style`, `font-stretch`, `line-height`, `letter-spacing`)로 정의하고 `TextStyle.typography` 참조 대상으로 연결합니다. family 해석 결과는 `FontFaceRef`로 고정하고 누락 글꼴은 진단합니다. 지금 `TextPayload`는 content와 paragraph·run의 style index만 저장하고 `TextStyle`·`ParagraphStyle` 표는 저장하지 않으므로(`crates/document/src/text.rs` `encode`), style 표 저장 형식을 이 행에서 정하고 Text payload schema 변경으로 IX.11.21 step에 넣습니다(옛 payload는 기본 style 표로 migration). 저장하는 `line-height` 값(`ParagraphStyle.line_height`, Typography `line-height`)의 뜻은 처음부터 CSS 의미(속성 모델 7장, inline box마다 자기 글꼴 metric에 half-leading 적용)로 정의합니다. text 엔진이 그 의미를 따르게 하는 일은 IX.11.12이며, 그 사이에 저장된 값의 뜻은 바뀌지 않습니다. `ParagraphStyle`에 문단 typography 참조(CSS의 문단 요소 자신의 `font-*`, root inline box strut의 글꼴 출처)를 추가하고, typography 없는 run은 문단 typography, 그다음 문서 기본 typography를 상속합니다(문서 기본의 저장 위치도 이 행에서 정함). `letter-spacing`은 `ParagraphStyle.letter_spacing_em`에서 run `TextStyle`의 선택 필드(없으면 Typography 값)로 옮기며 Typography resource를 복제하지 않습니다. 저장된 값이 없으므로 이 이동은 파일 migration이 아닙니다. `TypedValue::Typography` tag는 유지하고 payload를 Typography canonical encoding으로 정의하며, 새 encoding으로 decode되지 않는 기존 불투명 blob은 bytes를 보존하고 `typography-unrecognized` 진단을 냅니다. style longhand에는 IX.11.3 Text registry에 ADR-0024 결정 14로 key를 할당하고, semantic snapshot(`crates/semantic-diff/src/build.rs`)의 Text style 경로를 `TEXT_PAYLOAD` 하나에서 style longhand key 경로로 바꿉니다. 이때부터 Text style은 longhand 단위로 merge합니다(속성 모델 15장 O12). Text payload schema와 `crates/core-types/`의 `TypedValue::Typography` encoding 변경은 I.3.5 보호 대상이므로 같은 diff에서 ADR을 작성하거나 ADR-0008·ADR-0024를 개정합니다. | record round-trip, `line-height` 저장 값의 CSS 의미 정의가 record 문서 주석과 ADR에 있음, 같은 diff의 ADR 존재와 `cargo xtask validate-adr` 통과, 생략값이 CSS initial value와 같은지 test, missing font diagnostic fixture, 옛 Text payload migration golden, run→문단→문서 기본 상속 순서 fixture, 불투명 blob 진단과 bytes 보존 fixture, 같은 Text의 서로 다른 style longhand(예: `font-size`와 `letter-spacing`)를 양쪽에서 바꾼 3-way merge conflict 0, 갱신한 diff golden 목록 기록. | IX.11.1, IX.11.3, IX.11.21, VI.7.2, X.8.4, XIV.10.2 |
| IX.11.12 | `[ ]` | Text paragraph 속성을 CSS에 맞춥니다. `line-height` 배율(`<number>`)과 고정 길이(`<length>`)를 CSS처럼 inline box마다 자기 글꼴 metric에 half-leading을 적용해 해석하도록 text layout을 고칩니다. 지금은 둘 다 줄 전체 높이를 정하고 줄 안 최대 ascent·descent로 baseline을 정합니다(`crates/text/src/text_layout.rs`). root inline box strut는 문단 typography(IX.11.11)의 첫 사용 가능 글꼴과 font-size로 만듭니다. run `letter-spacing`(IX.11.11)을 text 엔진 입력에 연결합니다. IX.11.11이 저장하는 `line-height`는 처음부터 CSS 의미로 정의되므로 이 행은 저장 bytes의 뜻을 바꾸지 않고 엔진 해석만 맞추며 파일 migration을 만들지 않습니다. 바뀐 X.10 golden은 이유와 함께 갱신합니다. | 배율과 고정 길이 각각의 혼합 크기 run line box fixture(예: `line-height: 20px`에 10px·40px run이면 line box가 20px보다 큼), 문단 글꼴보다 작은 run만 있는 줄의 strut 높이 fixture, run별 letter-spacing fixture, 단일 크기 문단 결과 불변 test. | IX.11.11, IX.11.21, X.10.2 |
| IX.11.13 | `[ ]` | registry 값 문법별 CSS 값 formatter를 구현합니다. 속성 모델 8.2절·8.3절 정규 형식(가장 짧은 왕복 10진, 소문자 keyword, 색 hex 또는 `color()`)을 따르고 토큰 참조는 `var(--name)`으로 씁니다. | 값 문법별 golden 출력, 같은 값 byte-identical 출력, f32 왕복 property test 통과. | IX.11.2, IX.11.3 |
| IX.11.14 | `[ ]` | registry 값 문법별 CSS 값 parser와 shorthand 확장을 구현합니다. 속성 모델 2장의 입력 형태만 받고 그 밖은 `unsupported-unit`·`unsupported-value`·`unknown-token`·`forbidden-value` 오류를 구조화해 반환합니다. 입력 길이 상한과 재귀 없는 문법을 지킵니다. | parse(format(v)) == v property test, 11장 shorthand 확장 fixture, 거부 입력 fixture, parser fuzz target이 panic 0. | IX.11.13 |
| IX.11.15 | `[ ]` | 선택 노드 속성을 CSS 선언 블록 텍스트로 복사하고, CSS 선언 블록을 붙여넣어 하나의 CommandTransaction으로 적용하는 core API를 구현합니다. 알 수 없는 속성과 받을 수 없는 값은 건너뛴 목록으로 돌려주고 조용히 버리지 않습니다. parser가 받지 않는 형태로 복사되는 값(Constraint CenterX·CenterY의 `calc(50% + <offset>)`, 속성 모델 2장·4.5절)도 건너뜀 보고 대상입니다. 클립보드 연결은 셸이 맡습니다. | copy→paste 왕복 후 semantic snapshot 동일(Constraint 노드는 건너뜀 목록과 일치), 건너뜀 보고 fixture, undo 한 번으로 전체 복원. | IX.11.14, IX.11.16, VII.2.4, VII.2.6 |
| IX.11.16 | `[ ]` | TokenName과 ContextKey 이름에서 CSS custom property 이름을 결정론적으로 파생합니다. 속성 모델 10장 규칙(구분자 치환, identifier escape, 토큰 `--`·ContextKey `--context-` 접두사, 토큰과 ContextKey를 합친 집합 기준 충돌 진단, 충돌 시 StableId 32자리 hex 접미사)을 이름·종류·StableId만 받는 순수 함수로 구현하고 parser의 `var()` 해석에 연결합니다. 토큰 `@property` 선언 텍스트는 `inherits: true`로 만들고, `initial-value`에는 default expression을 기본 환경에서 평가한 literal을 씁니다. 다른 토큰을 참조하는 alias 값은 `:root`의 `var()` 선언으로만 냅니다(10장 등록 속성 정책 3번). ContextKey 입력은 로드맵 XXX.7.1의 `ContextKey` resource가 생기면 연결하고, 그 전에는 같은 함수에 합성 입력을 넣어 검증합니다. | 이름 파생 golden, 토큰끼리와 토큰·ContextKey 사이 충돌 진단 fixture, 65초 안에 만든 StableId 두 개의 접미사가 다름, `@property` 선언 golden(`inherits: true`), alias 토큰 golden(`initial-value`는 literal이고 `var()`는 `:root` 선언에만 있음), rename 후 StableId 참조 유지. | IX.11.13, VIII.5.2 |
| IX.11.17 | `[ ]` | macOS 인스펙터 섹션을 속성 모델 12장 CSS 범주로 재구성합니다. `InspectorPropertyCatalog`와 `docs/design/ui/inspector.md`를 갱신하고, sizing preset, 4변·4모서리 묶음 편집(longhand key 변경 집합으로 적용), CSS 값 입력·오류 표시, longhand 이름 툴팁을 연결합니다. migration된 노드의 회전·배율은 `transform` 행렬에서 계산한 표시 전용 값으로 보여 주고, 그 값을 편집할 때의 규칙(행렬 갱신 또는 `rotate`·`scale` 사용)을 이 행에서 정합니다. 외형은 가이드와 인스펙터 명세를 따릅니다. | `--inspector-ui-check` 통과(Light·Dark, 섹션 구성, 접근성 레이블, key view 순서), 모든 행에 registry CSS 이름 연결, 표시 전용 파생값이 저장 bytes에 나타나지 않음, 표시 전용 회전·배율 편집 규칙 결정과 근거가 보고서에 있고 migration된 노드의 회전·배율 편집 fixture 결과가 고른 규칙과 일치. | IX.11.2, IX.11.3, IX.11.7, IX.11.9, IX.11.14 |
| IX.11.18 | `[ ]` | modifier stack을 CSS 고정 적용 순서로 선형화하는 export 계획 함수를 구현합니다. 속성 모델 9.1절 규칙으로 요소·wrapper 구조, filter 목록, path 굽기 지점을 계산하고 wrapper마다 `approximated` 진단을 냅니다. 요소 구조에는 모든 요소가 같은 층(`z-index: 0`)의 stacking context를 만들어 DOM 순서대로 그려지게 하는 출력 규칙(속성 모델 3장, 14장 E3: absolute 요소는 `z-index: 0`, 그 밖의 모든 요소는 `position: relative; z-index: 0`), Frame 칠하기 층과 Path clip의 분리 규칙(속성 모델 9.2절), Transform modifier 행렬을 `transform-origin` 반대 이동으로 감싸는 규칙(9장 Transform 행)을 넣습니다. 이 행은 파일을 출력하지 않는 core 계획 함수이고, 실제 HTML·CSS 출력은 로드맵 XXII.2가 합니다. | 순서별 기대 구조 fixture, 같은 stack 같은 구조, stellar-only modifier 굽기 지점 fixture, Stack 안 Free Frame과 뒤 형제의 stacking 선언 fixture, border가 있는 Frame의 Fill·Stroke overlay fixture, flex item인 Stack layout Frame(border + Fill)의 overlay containing block이 그 Frame인 fixture, `border-*-radius`가 있는 Frame의 overlay가 같은 바깥 radius를 갖는 fixture, 자기 칠하기가 있는 Path clip Frame의 분리 구조 fixture, `transform-origin: 50% 50%` 요소의 Transform modifier 출력이 원점 보정된 fixture, Blend가 있는 요소의 합성 범위 영향 기록. | IX.11.2, IX.10.5 |
| IX.11.19 | `[ ]` | registry CSS 매핑과 Web TargetProfile을 잇는 초기 capability matrix를 IX.10.5 registry에 등록합니다. 상태는 IX.10.5 어휘(Native·Approximated·Unsupported·Deprecated)이고, CSS 매핑 상태에서 ADR-0025 결정 1.3 대응표로 정합니다. `stellar-only` 항목은 구조 변환·path 굽기·래스터화 규칙이 있으면 Approximated, 출력이 없으면 Unsupported입니다. Deprecated는 CSS 매핑 상태에서 나오지 않으므로 이 행에서 쓰지 않습니다. CSS 매핑 상태에 capability 조건이 붙은 항목(예: 속성 모델 5장 `clipping` FrameBounds의 `converted`(capability), 필요한 기능은 `overflow-clip-margin`의 `<visual-box>` 값)은 대응표 대신 기준 browser baseline의 그 기능 지원 여부로 정합니다. 지원하면 Native, 지원하지 않으면 Approximated이고 판정 근거를 보고서에 기록합니다. Apple·Android profile 항목은 CSS 매핑 상태에서 유도하지 않으므로(ADR-0025 결정 1.3) 이 행에서 등록하지 않고 XXII.1.4가 더합니다. XXII.1.1, XXII.1.4, XXII.2.2는 이 matrix를 입력으로 쓰고, XXII.3.2·XXII.4.2는 XXII.1.4를 거쳐 씁니다. | 모든 registry 항목에 Web profile 상태 존재, capability 조건이 없는 항목의 상태가 CSS 매핑 상태를 ADR-0025 결정 1.3 대응표로 바꾼 값과 같음, capability 조건이 있는 항목은 baseline 지원 여부에 따라 Native 또는 Approximated이고 판정 근거가 보고서에 있음, 속성 모델 14장 E1\~E10 항목의 Web 상태가 14장 Web export 열을 결정 1.3으로 바꾼 값과 같음(E1·E2·E8·E9는 Approximated), Apple·Android profile 항목 0. | IX.11.2, IX.11.3, IX.11.18, IX.10.5 |
| IX.11.20 | `[ ]` | SVG export 매핑표(`crates/export/src/svg/mapping.rs`)의 각 행에 registry CSS 매핑을 연결하고, `fill`·`stroke`·`opacity`·`mix-blend-mode`·`filter` 출력이 registry 값 문법과 IX.11.13 formatter를 쓰게 합니다. formatter에 SVG profile을 더합니다. SVG profile은 지금 SVG 표기(`crates/export/src/svg/format.rs`, XX.3.1)를 따릅니다. 범위 안 sRGB는 SVG 1.1 호환 `rgb(R%,G%,B%)`, 그 밖의 색은 alpha 없는 `color()`, alpha는 별도 `*-opacity` 속성, 행렬은 `matrix(a b c d e f)`이며 수 표기는 CSS profile과 같습니다(속성 모델 8.2절). 이미 구현된 SVG export의 값 표기를 formatter 한 곳으로 모으는 정리이며 SVG 출력 bytes를 바꾸지 않습니다. | 매핑 inventory 누락 0, SVG 출력 문자열이 IX.11.13 formatter SVG profile 결과와 일치, 기존 SVG golden bytes 불변, same snapshot byte-identical. | IX.11.2, IX.11.13, XX.3.1 |
| IX.11.21 | `[ ]` | 제품 문서 migration registry를 document와 storage에 모두 의존하는 crate(지금은 `crates/project`)에 만들고 project 열기에 연결합니다. 지금 step 등록은 test와 xtask의 noop v1→v2뿐이고(`tools/xtask/src/migration_fixtures.rs` `LATEST_SCHEMA = 2`), 제품 manifest schema는 1이며(`crates/project/src/manifest.rs` `CURRENT_SCHEMA_VERSION`), `SavedProjectLoader::open`은 migration 없이 current generation을 엽니다. storage crate는 document에 의존하지 않아 node record를 해석하지 못합니다. schema version별 payload decoder(Frame·Text 등)는 고정 사본으로 보관합니다. `FramePayload::decode`는 version 없이 잉여 bytes를 거부하므로(`crates/document/src/frame.rs`) 현재 decoder로 옛 bytes를 읽을 수 없습니다. 옛 schema 문서는 열 때 V.8.4 경로(temp generation, 검증 후 전환)로 migration하고 결과를 자동 commit하지 않습니다(아키텍처 §36, §40.4). Semantic Diff·Staging·Merge가 읽는 Git snapshot 원천(HEAD·Index·commit·merge base, XIV.2.6이 임시 디렉터리에 materialize)은 고정 decoder로 읽어 메모리에서 최신 schema로 migration하고 쓰지 않습니다. 그래서 migration 직후 옛 schema HEAD와 새 schema Working Tree를 같은 schema로 비교하고, 옛 schema merge base로 3-way merge합니다. 제품 step 번호는 test noop step과 섞이지 않게 정합니다. IX.11.6·IX.11.9·IX.11.10·IX.11.11의 schema 변경을 공개 포맷(Phase XXVIII) 전 version 증가 한 번으로 묶을지 검토하고, 결정을 착수 시 ADR로 기록합니다. | 옛 schema fixture project를 제품 open 경로로 열면 V.8.1 판정 matrix대로 migration·읽기 전용·거부가 나옴, 고정 decoder golden(옛 version bytes) 통과, `cargo xtask test-migrations`가 제품 registry 사용, 원본 generation hash 유지, 자동 commit 없음, 옛 schema HEAD 대 migration만 한 Working Tree의 semantic diff 0건과 migration 뒤 편집을 더한 fixture에서 그 편집만 보고, HEAD snapshot 디렉터리 쓰기 0, 옛 schema merge base fixture의 3-way merge 결과가 모두 새 schema로 migration한 뒤의 merge 결과와 같음, 묶음 결정 ADR 존재. | V.8.1, V.8.4, V.8.5, V.8.6, VI.10.2, XIV.2.6, XIV.3.1, XIV.10.2 |
| IX.11.22 | `[ ]` | 합성 PropertyKey를 longhand key로 나눕니다(ADR-0024 결정 14). Frame `size`(0x0101)를 `width`·`height`로, `corner_radius`(0x0106)를 4 모서리 `border-*-radius`로, modifier `SHADOW_OFFSET`(0x1092)을 shadow offset x·y로 나누고 같은 대역의 빈 번호를 할당합니다. Frame `width`·`height` key는 IX.11.6이 sizing longhand를 NodeRecord 공통 필드와 node별 item record 중 어디에 저장하든 그 record가 그대로 쓰는 key입니다. PropertyKey는 payload bytes에 담기지 않으므로 저장 위치와 독립이고, 그래서 같은 노드의 `width`에 key가 둘 생기지 않습니다. 기존 key는 재사용하지 않는 묶음 key로 registry에 남기고, 기존 `SetProperty` bytes와 history journal은 그대로 decode·실행합니다(결과는 longhand 전체를 설정한 것과 같음). 새 command, 인스펙터 묶음 편집, semantic snapshot(`crates/semantic-diff/src/build.rs`)과 merge는 longhand key를 씁니다. 저장된 참조(ValueBinding target, View Variant·Component variant override property patch)가 묶음 key를 가리키면 변환하지 않고 validator 진단을 냅니다. payload bytes는 바꾸지 않습니다. `transform`(0x0102)·`border`(0x0105)는 IX.11.9·IX.11.10이 같은 규칙으로 나눕니다. | registry key 중복 0과 묶음 key→longhand 대응 test, 묶음 key command bytes의 decode·apply 결과가 longhand command 묶음과 같음, 같은 Frame의 서로 다른 모서리 반경을 양쪽에서 바꾼 3-way merge conflict 0, 묶음 key 참조 진단 fixture, storage golden 불변. | VI.3.5, VI.9.7, VII.2.4, XIV.3.1, XIV.10.2, XXV.1.1 |
| IX.11.23 | `[ ]` | flex item 자동 최소 크기(`min-width: auto`·`min-height: auto`)를 CSS Flexbox 규칙으로 구현합니다. scroll container가 아닌 item의 main 축 최소 크기는 content 기반 최소 크기(min-content 크기를 definite max 크기로 제한한 값과 지정 크기 중 작은 값, 비율 item은 비율 변환 포함)이고, 명시한 `min-*` 값은 그대로 씁니다. min-content 측정은 intrinsic hook과 cache(X.7.2)를 씁니다. 바뀐 X.11.1 corpus 기대값은 fixture별 이유와 함께 갱신합니다. | 속성 모델 4.2절 `SizeLimits` min 행 fixture와 IX.11.8 자동 최소 크기 fixture 일치, X.7.4 benchmark를 적용 전후로 실행해 visited·cache hit·duration을 보고서에 기록, 같은 입력 결정론 test 통과. | IX.11.4, X.7.2, X.7.4 |

**IX.11 완료 조건:** 모든 registry 항목이 CSS 매핑을 갖고, CSS 레이아웃 적합성 fixture와 의미가 다른 구현 속성의 migration fixture가 통과하며, 인스펙터가 CSS 범주와 CSS 값 입력을 사용합니다.

### IX.12 Component Context 저장 번호 예약

ADR-0025 결정 6에 따라 Component Context 기능(모델, validator, 해석, command, 인스펙터)은 로드맵 XXX.7입니다. 초기 릴리스에는 파일 포맷 공개(XXVIII.3.1) 전에 필요한 번호 예약만 둡니다. 이 절은 기능 동작과 기존 저장 bytes를 바꾸지 않습니다.

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| IX.12.1 | `[ ]` | Component Context 저장에 쓸 numeric 번호와 확장 위치를 예약하는 ADR을 작성하고 registry 예약 상수로 고정합니다. 대상은 ReferenceTarget의 Context tag(현재 tag는 `crates/expressions/src/ast.rs`의 0x01\~0x04), `ContextKey`·`ContextProvision` record를 담을 chunk kind(현재 `crates/storage/src/chunk/kinds.rs`의 1\~6이며 모르는 required kind는 V.2.7 규칙으로 오류) 또는 기존 chunk 안의 record kind, ComponentDefinition 공개 인터페이스의 `ContextRequirement` 목록과 Slot의 공개 Context key 목록(ADR-0025 결정 3.11)이 들어갈 확장 위치입니다. 예약 번호는 다른 용도로 쓰지 않습니다. encoder·decoder 동작은 바꾸지 않습니다. | ADR 존재, 예약 번호가 기존 번호와 겹치지 않는 golden test, 기존 golden bytes 불변, `cargo xtask validate-adr` 통과. | VIII.2.2, V.6.1, IX.1.1, IX.3.1 |

**IX.12 완료 조건:** Context 저장 번호와 확장 위치가 ADR과 golden test로 고정되고, 기존 저장 bytes가 바뀌지 않습니다.

---

## Phase X. 결정론적 레이아웃과 텍스트 엔진

### X.1 레이아웃 입력·출력 규격

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| X.1.1 | `[x]` | `layout`에 `LayoutInput`, `LayoutBox`, `LayoutResult`, `LayoutError`를 정의합니다. 결과는 local rect, content rect, baseline, overflow bounds를 포함합니다. | 타입 round-trip과 finite validation. | Phase IX, III.3.3 |
| X.1.2 | `[x]` | layout tree view는 Fragment를 flatten하고 ComponentInstance virtual subtree와 View Variant 적용 결과를 읽도록 합니다. 원본 document를 수정하지 않습니다. | flatten 전후 visual child order fixture 통과. | VI.4.1, IX.2.2, IX.7.6 |
| X.1.3 | `[x]` | layout property registry에 각 속성의 dirty class를 SelfSize, ParentFlow, DescendantConstraint, PaintOnly로 분류합니다. | 모든 layout property에 class가 존재. | VI.3.5 |
| X.1.4 | `[x]` | LayoutRevision과 node별 input hash/output cache 타입을 정의합니다. | 동일 input hash에서 cache hit. | II.3.4 |
| X.1.5 | `[x]` | 레이아웃 수치의 rounding 정책을 logical subpixel 단위로 고정하고 pixel snapping은 렌더 단계로 분리합니다. | 1/3, scale 1.5 fixture가 양 플랫폼 동일. | III.3.1 |

### X.2 자유 배치·절대 좌표

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| X.2.1 | `[x]` | Free layout에서 자식 local position과 size를 그대로 사용하고 parent padding·content origin을 적용합니다. | 기본 위치 fixture 통과. | X.1.1 |
| X.2.2 | `[x]` | absolute anchors left/right/top/bottom과 fixed size 조합의 해석 순서를 정의합니다. over-constrained 입력은 명시 우선순위가 아니라 오류로 처리합니다. | 가능한 조합 matrix와 오류 fixture 통과. | X.2.1 |
| X.2.3 | `[x]` | percentage position/size를 containing block content rect 기준으로 계산합니다. | nested percentage fixture 통과. | X.2.1, III.3.2 |
| X.2.4 | `[x]` | local transform은 layout rect 계산 후 적용하고 flow size에는 영향을 주지 않는 초기 정책을 구현합니다. | rotated child가 sibling flow를 바꾸지 않습니다. | X.2.1 |

### X.3 Stack/Flex

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| X.3.1 | `[x]` | Stack axis, direction, gap, padding, main/cross alignment 타입을 정의합니다. | enum golden test. | X.1.1 |
| X.3.2 | `[x]` | 고정 크기 child의 순방향 배치와 gap 계산을 구현합니다. | horizontal/vertical/reverse fixture 통과. | X.3.1 |
| X.3.3 | `[x]` | grow, shrink, basis를 구현하고 분배 잔여값 처리 순서를 StableId가 아닌 child visual order로 고정합니다. | reference flex fixture 통과. | X.3.2 |
| X.3.4 | `[x]` | wrap과 multi-line cross alignment를 구현합니다. | line break·align-content fixture 통과. | X.3.3 |
| X.3.5 | `[x]` | hidden child와 zero-size child 처리 규칙을 구현합니다. hidden은 flow에서 제외하고 zero-size는 gap 참여 정책을 명시합니다. | fixture 통과. | X.3.2 |
| X.3.6 | `[ ]` | baseline alignment hook을 추가하고 Text baseline이 없는 node는 bottom edge fallback을 사용합니다. | mixed text/image fixture 통과. | X.3.2, X.10.4 |

### X.4 Grid

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| X.4.1 | `[x]` | Grid track 타입 Fixed, Fraction, MinContent, MaxContent, MinMax를 정의합니다. | type validation. | X.1.1 |
| X.4.2 | `[x]` | explicit row/column track sizing을 구현합니다. | fixed/fraction fixture 통과. | X.4.1 |
| X.4.3 | `[x]` | grid item start/end, span과 gap을 구현합니다. invalid span은 오류입니다. | overlap 허용 정책과 invalid fixture 통과. | X.4.2 |
| X.4.4 | `[x]` | auto placement를 row-major deterministic order로 구현합니다. | 삽입 순서와 visual OrderKey fixture 통과. | X.4.3 |
| X.4.5 | `[x]` | content-based track sizing을 텍스트·이미지 intrinsic size hook으로 연결합니다. | mixed content fixture 통과. | X.4.2, X.7.2 |

### X.5 Overlay와 Anchor/Constraint

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| X.5.1 | `[x]` | Overlay layout에서 모든 child가 같은 containing block을 사용하고 z-order는 OrderKey를 따르게 합니다. | overlay fixture 통과. | X.1.1 |
| X.5.2 | `[x]` | Anchor target을 parent edge, sibling edge, center, baseline로 정의하고 참조는 StableId를 사용합니다. | missing sibling은 diagnostic. | III.2.3, X.1.1 |
| X.5.3 | `[x]` | constraint graph를 구축하고 cycle·underdetermined·overdetermined를 분류합니다. | 각 graph fixture 정확. | X.5.2 |
| X.5.4 | `[x]` | 초기 constraint solver는 equality와 priority 없는 필수 제약만 지원합니다. 지원하지 않는 priority 필드는 스키마에 넣지 않습니다. | deterministic solution fixture 통과. | X.5.3 |
| X.5.5 | `[x]` | constraint 변경 영향 범위를 connected component로 제한합니다. | 무관 component 재계산 0. | X.5.4 |

### X.6 콘텐츠 기반 크기·최소·최대·비율

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| X.6.1 | `[x]` | SizingMode에 Fixed, Fill, HugContent, Percentage를 정의합니다. | invalid mode combination 거부. | X.1.1 |
| X.6.2 | `[x]` | min/max clamp를 모든 layout mode의 최종 size에 동일 순서로 적용합니다. | min>max 오류, boundary fixture 통과. | X.6.1 |
| X.6.3 | `[x]` | aspect ratio constraint를 한 축이 definite일 때만 적용하고 양 축 indefinite는 intrinsic size로 해결합니다. | ratio matrix fixture 통과. | X.6.1 |
| X.6.4 | `[x]` | content intrinsic measurement trait를 Shape, Text, Image, ComponentInstance별로 정의합니다. | 각 node default intrinsic fixture 통과. | Phase VI, IX.2.2 |
| X.6.5 | `[x]` | HugContent cycle을 탐지하고 무한 상호 의존을 diagnostic으로 반환합니다. | parent↔child cycle fixture 실패. | X.6.4 |

### X.7 증분 레이아웃

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| X.7.1 | `[x]` | command invalidation을 layout dirty set으로 변환합니다. PaintOnly는 layout queue에 넣지 않습니다. | property별 expected dirty set 통과. | VII.1.4, X.1.3 |
| X.7.2 | `[x]` | bottom-up intrinsic measurement와 top-down placement를 분리하고 cache key를 정의합니다. | 같은 input에서 중복 측정 0. | X.6.4, X.1.4 |
| X.7.3 | `[x]` | dirty node의 ancestor propagation을 layout mode별로 최소화합니다. | leaf color 변경 0 layout, leaf width 변경 예상 ancestor만 방문. | X.7.1 |
| X.7.4 | `[x]` | 20만 node synthetic document에서 단일 leaf 변경 방문 node count를 기록하는 benchmark를 추가합니다. | result JSON에 visited/cache hit/duration 포함. | X.7.3 |
| X.7.5 | `[x]` | layout result가 DocumentRevision과 input hash에 맞지 않으면 폐기하는 worker result guard를 구현합니다. | stale result 적용 0. | II.3.4, X.1.4 |
| X.7.6 | `[x]` | 전체 layout fallback 경로를 테스트·진단용으로만 제공하고 일반 property 변경에서 호출되면 debug assertion과 metric을 남깁니다. | 단일 edit fixture에서 full layout count 0. | X.7.3 |

### X.8 폰트 레코드와 탐색

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| X.8.1 | `[x]` | `FontSource`를 Project, System, External로 정의하고 `FontFaceRef`에 file hash, face index, variation axes를 포함합니다. | round-trip. | III.3.5, V.1.5 |
| X.8.2 | `[x]` | `FontRegistry` 인터페이스를 정의하고 같은 face를 hash+index로 dedup합니다. 플랫폼 font discovery 구현은 Phase XIII에서 연결합니다. | duplicate register returns same handle. | X.8.1 |
| X.8.3 | `[x]` | explicit fallback chain 모델을 정의합니다. missing face를 임의 시스템 font로 바꾸지 않습니다. | fallback order fixture 통과. | X.8.2 |
| X.8.4 | `[x]` | missing font diagnostic에 requested face, source, affected Text IDs를 포함합니다. | fixture 정확, 텍스트 내용 미포함. | X.8.3, II.3.2 |

### X.9 공통 shaping·BiDi·줄바꿈

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| X.9.1 | `[x]` | 승인된 HarfBuzz binding을 dependencies policy에 등록하고 `text` crate에 안전 wrapper를 구현합니다. raw pointer는 wrapper 밖에 노출하지 않습니다. | simple Latin shaping fixture 통과, leak sanitizer clean. | I.3.2, X.8.2 |
| X.9.2 | `[x]` | script/language/direction itemization을 구현합니다. 입력 문자열과 style run을 shaping runs로 분할합니다. | Latin+Arabic+한글 mixed fixture 통과. | X.9.1, VI.7.2 |
| X.9.3 | `[x]` | Unicode BiDi 알고리즘 구현 또는 승인 dependency wrapper를 추가합니다. visual order와 logical cluster mapping을 모두 유지합니다. | 공식 또는 고정 BiDi corpus 통과. | X.9.2 |
| X.9.4 | `[x]` | line break opportunity 계산을 공통 엔진으로 구현하고 locale 입력을 명시합니다. | 한글·일본어·영문·공백 fixture 통과. | X.9.2 |
| X.9.5 | `[x]` | font fallback을 grapheme/script run 단위로 적용하고 missing glyph cluster 정보를 보존합니다. | emoji, CJK fallback fixture 통과. | X.8.3, X.9.2 |
| X.9.6 | `[x]` | OpenType feature와 variation axes를 shaping input에 적용합니다. | liga on/off, variable axis fixture 결과 차이 예상 일치. | X.9.1, VI.7.2 |

### X.10 텍스트 레이아웃

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| X.10.1 | `[x]` | shaped glyph runs를 width constraint에 맞춰 line으로 나누고 cluster를 쪼개지 않습니다. | narrow width, long word, CJK fixture 통과. | X.9.4, X.9.5 |
| X.10.2 | `[x]` | line height, paragraph spacing, alignment을 적용합니다. | top/middle/bottom은 text box alignment와 분리하여 fixture화. | X.10.1, VI.7.3 |
| X.10.3 | `[x]` | caret positions, selection rects, hit-test cluster mapping을 생성합니다. | BiDi caret fixture 통과. | X.10.1, X.9.3 |
| X.10.4 | `[!]` | Text intrinsic width/height와 baseline을 layout intrinsic hook에 연결합니다. | Stack baseline fixture 통과. | X.10.2, X.6.4 |
| X.10.5 | `[x]` | shaping·line layout cache key에 text hash, style hash, font face hash, width, locale, target profile을 포함합니다. | 한 입력 변경마다 예상 cache miss. | X.10.1, X.8.1 |
| X.10.6 | `[x]` | 텍스트 일부 편집 시 영향 paragraph와 이후 line만 재배치하는 incremental API를 구현합니다. | 첫 paragraph edit가 독립 두 번째 paragraph를 재shape하지 않습니다. | VI.7.5, X.10.5 |
| X.10.7 | `[~]` | macOS·Windows 동일 font files로 glyph IDs, advances, line breaks golden 비교를 CI에 추가합니다. | byte-identical logical layout. | X.10.6 |

### X.11 레이아웃 적합성 corpus

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| X.11.1 | `[x]` | Free, Stack, Grid, Overlay, Constraint 각각 최소 50개 fixture를 생성하는 schema와 runner를 작성합니다. | fixture count와 schema 검증 통과. | X.2~X.6 |
| X.11.2 | `[~]` | nested component, variant, token 조건이 포함된 layout fixture 20개를 추가합니다. | expected rect golden 통과. | IX.9, X.7 |
| X.11.3 | `[ ]` | 양 플랫폼에서 logical layout result를 비교하는 CI job을 추가합니다. | 모든 rect/baseline bytes 동일. | X.10.7, X.11.2 |

**Phase X 완료 조건:** 공통 레이아웃과 텍스트 논리 결과가 양 플랫폼에서 동일하고 증분 benchmark가 방문 범위를 보고합니다.

---

## Phase XI. Scene Snapshot, Hit Test와 Metal·Direct3D 12 렌더러

### XI.1 불변 Scene Snapshot

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XI.1.1 | `[x]` | `scene`에 `SceneNode`, `SceneResourceRef`, `SceneSnapshot`을 정의합니다. Document Node와 GPU 객체를 직접 포함하지 않습니다. | core snapshot round-trip 테스트용 serialization 통과. | Phase X |
| XI.1.2 | `[x]` | Scene node에 world transform, layout rect, clip stack ref, paint stack ref, z-order, hit-test metadata를 정의합니다. | fixture 생성 통과. | XI.1.1 |
| XI.1.3 | `[x]` | DocumentRevision→SceneRevision mapping과 snapshot generation을 정의합니다. | 동일 document revision에서 동일 scene semantic hash. | II.3.4, XI.1.1 |
| XI.1.4 | `[x]` | 이전 snapshot과 구조 공유하는 persistent collections를 도입합니다. 바뀐 subtree만 새 allocation을 갖게 합니다. | 단일 leaf 변경 시 shared node count 기준 통과. | XI.1.3 |
| XI.1.5 | `[x]` | render thread가 `Arc<SceneSnapshot>` 또는 epoch-safe handle로 lock 없이 읽게 합니다. mutable reference는 노출하지 않습니다. | thread sanitizer 또는 동등 동시성 테스트 clean. | XI.1.4 |
| XI.1.6 | `[x]` | 오래된 snapshot을 renderer가 제출 전 감지해 폐기할 수 있는 revision check를 추가합니다. | 빠른 100 revision fixture에서 최신 결과만 present candidate. | XI.1.3 |

### XI.2 Scene 증분 빌더

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XI.2.1 | `[ ]` | layout/binding/modifier invalidation을 scene dirty set으로 변환합니다. | paint-only, geometry, subtree dirty fixture 정확. | VII.1.4, X.7.1, VI.9.7 |
| XI.2.2 | `[x]` | Frame, Shape, Path, Text, Image를 scene primitives로 변환하는 builder를 각각 별도 함수로 구현합니다. | node kind별 golden scene snapshot. | XI.1.2, Phase VI |
| XI.2.3 | `[x]` | Fragment를 scene node로 생성하지 않고 자식을 부모 scene scope에 flatten합니다. | Fragment 유무 scene primitive 동일. | VI.4.1, XI.2.2 |
| XI.2.4 | `[ ]` | ComponentInstance virtual subtree와 View Variant 적용 결과를 scene으로 생성합니다. | IX.9 fixture scene snapshot 통과. | IX.2.2, IX.7.6, XI.2.2 |
| XI.2.5 | `[ ]` | dirty subtree만 재빌드하고 변경되지 않은 scene node/resource refs를 공유합니다. | 단일 property edit rebuild count 예상 일치. | XI.1.4, XI.2.1 |
| XI.2.6 | `[ ]` | scene build duration과 rebuilt node count를 diagnostics trace에 기록합니다. 내용·이름은 기록하지 않습니다. | trace schema 테스트 통과. | II.3.2, XI.2.5 |

### XI.3 공간 인덱스·가시성·Hit Test

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XI.3.1 | `[x]` | world bounds 기반 공간 인덱스 인터페이스를 정의하고 insert/update/remove/query를 구현합니다. | 20만 node random query correctness. | XI.1.2 |
| XI.3.2 | `[x]` | viewport frustum과 clip bounds로 visible candidate를 계산합니다. | 화면 밖 node draw list 제외. | XI.3.1 |
| XI.3.3 | `[x]` | z-order 역순, clip, visibility, pointer-events semantic을 적용한 broad-phase hit test를 구현합니다. | overlap fixture에서 topmost 결과. | XI.3.1, XI.1.2 |
| XI.3.4 | `[x]` | Shape/Path/Text/Image별 narrow-phase hit test를 구현합니다. alpha pixel hit test는 초기 제외하고 bounds 또는 geometry policy를 명시합니다. | node kind별 fixture 통과. | XI.3.3, VI.5.6, VI.6.4, X.10.3 |
| XI.3.5 | `[x]` | selection handle와 modifier control point를 별도 transient hit-test layer로 추가합니다. project 저장에 포함하지 않습니다. | handle 우선순위 fixture 통과. | XI.3.3, VII.3.1 |
| XI.3.6 | `[x]` | 2만 가시 node 기준 hit test p99 benchmark를 추가합니다. | 결과 JSON에 query count·candidate count·duration. | XI.3.4 |

### XI.4 Render Graph 공통 계층

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XI.4.1 | `[x]` | `renderer-core`에 backend 독립 `RenderGraph`, pass, attachment, resource lifetime 타입을 정의합니다. | cycle·missing resource validator 통과. | XI.1.1 |
| XI.4.2 | `[x]` | scene primitive를 opaque/transparent/effect pass로 분류하는 compiler를 구현합니다. | ordering fixture 통과. | XI.2.2, XI.4.1 |
| XI.4.3 | `[x]` | clip stack을 scissor, stencil, offscreen mask 중 하나로 계획하는 backend-neutral decision table을 구현합니다. | simple rect clip은 scissor 선택. | XI.4.1 |
| XI.4.4 | `[x]` | blend/opacity/filter dependency를 분석해 필요한 최소 offscreen subtree 경계를 계산합니다. | nested opacity fixture expected pass graph. | VI.9.2, XI.4.2 |
| XI.4.5 | `[x]` | draw item batching key를 pipeline, texture/atlas, clip state, blend state로 정의합니다. z-order를 바꾸는 batch는 금지합니다. | batch 후 render order 의미 동일. | XI.4.2 |
| XI.4.6 | `[x]` | RenderGraph compile 결과의 CPU 시간, pass count, draw count, batch count를 trace합니다. | trace fixture 통과. | XI.4.5, II.3.2 |

### XI.5 GPU backend interface

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XI.5.1 | `[x]` | `RendererBackend` trait에 device init, resize, resource create/update/destroy, encode, submit, present 예약, timestamp query를 정의합니다. | mock backend compile and call-order test. | XI.4.1 |
| XI.5.2 | `[x]` | GPU handle을 generational ID로 감싸고 backend native pointer를 공통 scene에 노출하지 않습니다. | stale handle fixture 실패. | III.1.3, XI.5.1 |
| XI.5.3 | `[x]` | resource upload queue와 frame-local allocator interface를 정의합니다. | 3 frames-in-flight simulation에서 premature free 없음. | XI.5.2 |
| XI.5.4 | `[x]` | device lost/out-of-memory/error 분류와 복구 가능 여부를 구조화합니다. | mock backend error matrix 통과. | XI.5.1, II.3.1 |
| XI.5.5 | `[x]` | present queue depth를 최대 1로 유지하는 frame scheduler를 구현합니다. 새 snapshot이 오면 아직 제출 전 오래된 frame을 drop합니다. | slow GPU simulation에서 queue depth≤1. | XI.5.1, XI.1.6 |

### XI.6 Geometry·Tessellation·Glyph cache

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XI.6.1 | `[x]` | Shape/Path fill tessellation interface와 cache key를 정의합니다. | 동일 path/style cache hit. | VI.5.6, VI.6.1 |
| XI.6.2 | `[x]` | stroke tessellation에 width, join, cap, dash를 적용합니다. | reference geometry bounds fixture 통과. | XI.6.1 |
| XI.6.3 | `[x]` | tessellation 작업을 worker pool에서 실행하고 stale scene revision 결과를 버립니다. | stale result 적용 0. | X.7.5, XI.6.1 |
| XI.6.4 | `[x]` | glyph outline/bitmap atlas interface와 key를 font hash, glyph id, size, variation, rendering mode로 정의합니다. | key collision fixture 없음. | X.10.5 |
| XI.6.5 | `[x]` | atlas page allocation·eviction을 구현하고 현재 frame 사용 glyph는 제거하지 않습니다. | memory pressure simulation 통과. | XI.6.4 |
| XI.6.6 | `[x]` | cache miss 시 placeholder가 아니라 outline rasterization 완료 전 이전 valid glyph 또는 빈 glyph 정책을 명시하고 테스트합니다. | 잘못된 glyph 표시 없음. | XI.6.5 |

### XI.7 이미지 decode·texture cache

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XI.7.1 | `[x]` | 이미지 decoder interface를 worker 전용으로 정의하고 decoded pixel format을 명시합니다. | malformed image corpus panic 없음. | VI.8.1 |
| XI.7.2 | `[x]` | mipmap 생성과 texture upload task를 구현합니다. | 크기별 expected mip count. | XI.7.1, XI.5.3 |
| XI.7.3 | `[x]` | texture cache key를 asset content hash, decode settings, color transform으로 정의합니다. | source path가 달라도 같은 content dedup. | V.1.5, XI.7.1 |
| XI.7.4 | `[~]` | missing/decoding asset placeholder를 deterministic checker 또는 neutral icon으로 정의하고 양 backend 동일 data를 사용합니다. | screenshot fixture 동일. | VI.8.2 |

### XI.8 Metal backend

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XI.8.1 | `[x]` | `renderer-metal`에서 MTLDevice, command queue, CAMetalLayer 연결과 pixel format을 초기화합니다. | macOS smoke frame clear 통과. | XI.5.1, II.5.4 |
| XI.8.2 | `[~]` | resize와 drawable acquisition을 구현하고 drawable 없음은 frame skip으로 처리합니다. | minimized/occluded window crash 없음. | XI.8.1 |
| XI.8.3 | `[x]` | 공통 vertex/index/uniform buffer abstraction을 Metal resource로 매핑합니다. | mock geometry triangle render fixture 통과. | XI.5.3, XI.8.1 |
| XI.8.4 | `[x]` | pipeline state와 shader library cache를 구현합니다. shader compile 실패는 구조화 오류로 반환합니다. | known shader fixture 렌더. | XI.8.3 |
| XI.8.5 | `[x]` | RenderGraph pass encoding, submit, `present` 예약, CPU timestamp를 구현합니다. | T0-T5 trace에 submit/present timestamp 존재. | XI.4.6, XI.8.4 |
| XI.8.6 | `[x]` | Metal GPU timestamp query를 연결해 pass별 GPU 시간을 측정합니다. | simple scene GPU duration report. | XI.8.5 |
| XI.8.7 | `[x]` | device error·memory pressure·drawable timeout 처리 테스트를 추가합니다. | 앱 crash 없이 error event 발생. | XI.5.4, XI.8.5 |

### XI.9 Direct3D 12 backend

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XI.9.1 | `[!]` | `renderer-d3d12`에서 adapter, device, command queue, swap chain을 Windows 11 x64에 초기화합니다. | Windows smoke clear frame 통과. | XI.5.1, II.6.4 |
| XI.9.2 | `[!]` | resize, back buffer, frame fence를 구현합니다. | resize/minimize/restore fixture crash 없음. | XI.9.1 |
| XI.9.3 | `[!]` | descriptor heap, upload/default buffer, texture resource mapping을 구현합니다. | triangle render fixture 통과. | XI.5.3, XI.9.1 |
| XI.9.4 | `[!]` | root signature, pipeline state, shader cache를 구현합니다. | known shader fixture 렌더. | XI.9.3 |
| XI.9.5 | `[!]` | RenderGraph command list encoding, execute, Present 예약을 구현합니다. | T0-T5 trace에 submit/present timestamp 존재. | XI.4.6, XI.9.4 |
| XI.9.6 | `[!]` | timestamp query heap을 연결해 pass별 GPU 시간을 측정합니다. | simple scene GPU duration report. | XI.9.5 |
| XI.9.7 | `[!]` | device removed, out-of-memory, swap chain failure 처리 테스트를 추가합니다. | 앱 crash 없이 error event 발생. | XI.5.4, XI.9.5 |

### XI.10 공통 색상·블렌드·효과

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XI.10.1 | `[ ]` | shader 입력 색을 linear working space로 변환하고 output target에 맞춰 encode하는 공통 수식을 문서화·구현합니다. | CPU reference와 GPU pixel 허용 오차 통과. | III.4.2, XI.8.4, XI.9.4 |
| XI.10.2 | `[ ]` | alpha compositing과 지원 blend mode의 수식을 고정합니다. 초기 지원 목록 밖 mode는 diagnostic입니다. | blend reference image 통과. | XI.10.1 |
| XI.10.3 | `[ ]` | gradient 렌더링을 구현하고 stop tie-order를 Phase III 규칙대로 적용합니다. | gradient pixel fixture 통과. | III.4.3, XI.10.1 |
| XI.10.4 | `[ ]` | clip, mask, opacity, shadow, blur를 순차적으로 구현하고 각 기능마다 독립 image fixture를 추가합니다. | 기능별 Metal/D3D12 비교 통과. | XI.4.4, XI.10.1 |
| XI.10.5 | `[x]` | Display P3 surface 지원 여부를 플랫폼 capability로 감지하고 unsupported display에서는 sRGB 변환과 gamut warning을 생성합니다. | mock capability fixture 통과. | III.4.2, XI.5.1 |
| XI.10.6 | `[x]` | HDR path는 명시적 unsupported 상태로 유지하고 extended value를 silent clamp하지 않습니다. | HDR fixture warning 발생. | XI.10.5 |

### XI.11 렌더 적합성·성능

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XI.11.1 | `[ ]` | common scene fixture를 offscreen render해 PNG-like raw RGBA reference를 생성하는 테스트 harness를 양 backend에 구현합니다. 제품 export 기능은 아닙니다. | 동일 fixture raw image 생성. | XI.8.5, XI.9.5 |
| XI.11.2 | `[ ]` | image comparison에 per-channel/structural 허용 오차를 config로 정의하고 초과 pixel 위치를 diff artifact로 남깁니다. | known equal/different fixture 판정 정확. | XI.11.1 |
| XI.11.3 | `[ ]` | Frame, Shape, Path, Text, Image, Modifier, clip, P3 fixture를 최소 100개 구성합니다. | fixture inventory 검사 통과. | XI.10.4, XI.7.4, XI.6.6 |
| XI.11.4 | `[ ]` | 2만 가시 node scene에서 scene build, render graph encode, submit까지 구간 benchmark를 추가합니다. | 단계별 p50/p95/p99 JSON 생성. | XI.2.6, XI.4.6, XI.8.5, XI.9.5 |
| XI.11.5 | `[x]` | Present queue depth와 dropped stale frame count를 benchmark 결과에 포함합니다. | queue depth≤1 assertion. | XI.5.5 |
| XI.11.6 | `[ ]` | Metal/D3D12 conformance CI에서 모든 fixture와 smoke benchmark를 실행합니다. | G6 통과. | XI.11.3~XI.11.5 |

**Phase XI 완료 조건:** G6 통과.

---

## Phase XII. 앱 프로세스 스레드 모델과 네이티브 셸 통합

### XII.1 런타임 스레드 구성

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XII.1.1 | `[x]` | `DocumentThread` event loop를 구현하고 모든 mutable document 접근을 이 thread로 제한합니다. | 다른 thread 직접 mutate compile/API test 실패. | Phase XI, Phase VII |
| XII.1.2 | `[x]` | UI→DocumentThread command queue를 bounded channel로 구현하고 입력 이벤트에 priority를 부여합니다. | queue overflow 정책 fixture 통과, blocking UI call 없음. | XII.1.1 |
| XII.1.3 | `[x]` | RenderThread를 구현하고 최신 SceneSnapshot을 lock-free handoff로 수신합니다. | document thread가 render submit을 기다리지 않습니다. | XI.1.5, XII.1.1 |
| XII.1.4 | `[x]` | worker pool을 text shaping, tessellation, image decode, save serialization class로 나누거나 priority tag로 구분합니다. | interactive 작업이 background GC보다 먼저 실행. | X.7.5, XI.6.3, XI.7.1 |
| XII.1.5 | `[x]` | worker result에 source revision/hash를 포함하고 stale result를 적용하지 않습니다. | rapid edit fixture에서 stale apply 0. | XII.1.4 |
| XII.1.6 | `[ ]` | thread별 shutdown 순서를 UI stop→command drain/cancel→render stop→worker stop→runtime destroy로 고정합니다. | 1,000 lifecycle stress에서 deadlock·누수 없음. | XII.1.1~XII.1.5 |

### XII.2 네이티브 창과 프로젝트 세션

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XII.2.1 | `[x]` | `ProjectWindowSession` core type에 ProjectIdentity, WindowSessionId, 독립 document state, view state, history, loaded generation을 정의합니다. | 두 session이 서로 state를 공유하지 않습니다. | IV.2.3, VII.5.1, V.4.4 |
| XII.2.2 | `[x]` | macOS `NSWindowController`가 한 ProjectWindowSession handle을 소유하도록 연결합니다. | 창 2개 open/close lifecycle 통과. | II.5.3, XII.2.1 |
| XII.2.3 | `[!]` | Windows `MainWindow`가 한 ProjectWindowSession handle을 소유하도록 연결합니다. | 창 2개 open/close lifecycle 통과. | II.6.3, XII.2.1 |
| XII.2.4 | `[ ]` | 같은 project path를 여러 창에서 열 수 있게 하되 각 창이 독립 base generation과 memory model을 로드하게 합니다. | 한 창 edit가 다른 창에 즉시 나타나지 않습니다. | XII.2.2, XII.2.3 |
| XII.2.5 | `[x]` | 창별 열린 tab, active view, viewport, selection을 WindowViewState에 연결합니다. | 창간 상태 격리. | VII.3.1, XII.2.1 |

### XII.3 Canvas 입력 파이프라인

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XII.3.1 | `[x]` | 공통 `InputEvent`에 pointer, wheel, key, text composition, focus, resize를 정의하고 native timestamp를 monotonic domain으로 변환합니다. | 양 플랫폼 event fixture schema 동일. | II.3.3 |
| XII.3.2 | `[x]` | macOS mouse/trackpad/keyboard 이벤트를 InputEvent로 변환합니다. | coordinate, modifiers, double click fixture 통과. | II.5.4, XII.3.1 |
| XII.3.3 | `[!]` | Windows pointer/mouse/keyboard 이벤트를 InputEvent로 변환합니다. | DPI scaling, modifiers, double click fixture 통과. | II.6.4, XII.3.1 |
| XII.3.4 | `[x]` | 연속 pointer move/wheel event coalescing을 구현하되 pointer down/up과 key event 순서를 바꾸지 않습니다. | 10,000 move→최신 1개, down/up 보존. | XII.3.1 |
| XII.3.5 | `[~]` | InputEvent 수신 T0 trace를 찍고 DocumentThread hit-test request까지 queue latency를 측정합니다. | trace에 T0/T1 후보 timestamp 존재. | XI.3.4, XII.1.2 |
| XII.3.6 | `[x]` | Pan, Zoom, Select, Drag, Resize, Rotate interaction state machine을 각 도구별 closed enum으로 구현합니다. | cancel/complete fixture 통과. | XI.3.5, Phase VII |
| XII.3.7 | `[!]` | interaction preview는 transient state로 Scene builder에 주입하고 pointer up에서만 CommandTransaction을 확정합니다. | drag 중 project model revision 불필요 증가 없음 또는 정의된 preview revision만 사용. | VII.4.2, XI.2.1 |

### XII.4 Canvas surface와 renderer 연결

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XII.4.1 | `[ ]` | macOS CanvasHost의 CAMetalLayer를 Metal backend에 연결하고 scale/size lifecycle을 전달합니다. | resize, display scale change 렌더 통과. | XI.8.2, XII.2.2 |
| XII.4.2 | `[!]` | Windows CanvasHost HWND/SwapChainPanel 또는 선택된 native surface를 D3D12 backend에 연결합니다. | resize, DPI change 렌더 통과. | XI.9.2, XII.2.3 |
| XII.4.3 | `[~]` | viewport pan/zoom matrix를 scene camera 입력으로 연결하고 document geometry는 변경하지 않습니다. | pan/zoom 후 save bytes 동일. | XII.2.5, XI.2.2 |
| XII.4.4 | `[~]` | selection overlay와 handles를 transient scene layer로 렌더합니다. | project snapshot에 overlay node 없음. | XI.3.5, XII.4.3 |

### XII.5 네이티브 UI 텍스트와 IME

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XII.5.1 | `[x]` | macOS 레이어명·검색·일반 필드에 NSTextField/NSTextView를 사용하고 core text engine을 호출하지 않습니다. | native undo/IME smoke test. | II.5.3 |
| XII.5.2 | `[!]` | Windows 레이어명·검색·일반 필드에 TextBox/RichEditBox를 사용합니다. | native undo/IME smoke test. | II.6.3 |
| XII.5.3 | `[ ]` | 캔버스 Text node 편집용 common editing session을 정의하고 native IME composition event만 받아 Phase X layout으로 표시합니다. | 한글 조합 중간·확정 fixture 통과. | X.10.3, XII.3.1 |
| XII.5.4 | `[ ]` | macOS text input client adapter를 구현해 marked text, selection range, candidate rect를 common session과 연결합니다. | 한글·일본어 IME manual automation fixture 통과. | XII.5.3 |
| XII.5.5 | `[!]` | Windows CoreText/TSF 또는 승인된 WinUI input adapter를 구현해 composition을 common session과 연결합니다. | 한글·일본어 IME fixture 통과. | XII.5.3 |
| XII.5.6 | `[ ]` | native overlay text control로 캔버스 Text를 렌더하지 않는 것을 검사하는 구조 테스트를 추가합니다. | editing 중에도 glyph layout source가 common text engine. | XII.5.4, XII.5.5 |

### XII.6 최소 애플리케이션 명령 연결

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XII.6.1 | `[ ]` | New Project native command가 ProjectCreateRequest를 호출하고 폴더 선택+이름 입력 결과만 core에 전달합니다. 상세 UX는 구현하지 않습니다. | create integration test 통과. | IV.4.7, XII.2.2, XII.2.3 |
| XII.6.2 | `[x]` | Open Project command가 folder를 선택하고 ProjectProbe 후 session을 엽니다. | invalid repo 안내 error mapping 통과. | IV.5.3 |
| XII.6.3 | `[ ]` | Clone Project command가 URL과 target folder를 받아 clone-open pipeline을 호출합니다. | local fixture remote clone 통과. | IV.6.4 |
| XII.6.4 | `[~]` | Undo/Redo/Save Now command를 active WindowSession에 연결합니다. | 창별 history와 save 호출 fixture 통과. | VII.5.3, V.5.4 |
| XII.6.5 | `[~]` | 앱 메뉴 단축키를 macOS·Windows native convention으로 연결하되 command ID는 core에서 공유합니다. | key mapping smoke test. | XII.6.4 |

### XII.7 동일 프로젝트 다중 창 저장 충돌

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XII.7.1 | `[ ]` | 각 WindowSession이 load 당시 base generation/hash를 유지하고 save 전에 disk current와 비교하게 합니다. | 두 창 동시 fixture에서 두 번째 save가 Stale 상태. | XII.2.4, V.5.2 |
| XII.7.2 | `[ ]` | stale save 시 선택지 두 개만 반환하는 core result를 정의합니다: UseCurrentWindow, UseLatestDisk. Semantic merge 옵션은 포함하지 않습니다. | enum golden test. | XII.7.1 |
| XII.7.3 | `[ ]` | macOS에서 단순 native alert로 두 선택지를 표시합니다. 시각적 diff를 열지 않습니다. | 선택별 core call smoke test. | XII.7.2 |
| XII.7.4 | `[!]` | Windows에서 ContentDialog 또는 native equivalent로 두 선택지를 표시합니다. | 선택별 core call smoke test. | XII.7.2 |
| XII.7.5 | `[ ]` | UseCurrentWindow는 새 save plan을 현재 disk generation 위에 전체 window snapshot으로 생성해 덮어씁니다. | 선택 후 disk가 window snapshot과 동일. | XII.7.2, V.5.4 |
| XII.7.6 | `[ ]` | UseLatestDisk는 로컬 미저장 state와 해당 창 history를 폐기하고 최신 generation을 다시 로드합니다. | reload 후 local changes/history 없음. | XII.7.2, VII.5.4 |
| XII.7.7 | `[ ]` | 자동 저장 중 stale가 감지되면 auto-save를 중지하고 같은 alert를 한 번만 요청합니다. | 반복 알림 없음, 선택 전 write 없음. | XII.7.1 |

### XII.8 앱 자체 접근성

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XII.8.1 | `[ ]` | macOS native UI controls에 label, role, keyboard focus order를 지정하고 CanvasHost는 선택 상태·도구 상태를 접근성으로 노출합니다. | VoiceOver automation smoke 통과. | XII.2.2, XII.6 |
| XII.8.2 | `[!]` | Windows UI Automation properties와 keyboard focus order를 지정합니다. | Narrator/UIA automation smoke 통과. | XII.2.3, XII.6 |
| XII.8.3 | `[ ]` | 디자인 문서 접근성 snapshot을 preview inspector가 읽을 수 있는 bridge를 추가합니다. 상세 inspector UX는 제외합니다. | IX.9 fixture의 역할 tree 읽기 가능. | IX.8.6, II.4.5 |

**Phase XII 완료 조건:** 네이티브 두 앱에서 프로젝트 생성·열기, 기본 편집, 렌더링, 창별 Undo, 다중 창 충돌 선택이 작동합니다.

---

### XII.9 운영체제 네이티브 셸 표현

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XII.9.1 | `[ ]` | macOS 앱 셸의 toolbar, sidebar, inspector, dialog가 macOS 26 AppKit/SwiftUI native material과 control을 사용하도록 theme capability layer를 구현합니다. 캔버스 문서 렌더링에는 적용하지 않습니다. | UI hierarchy에 승인되지 않은 WebView/커스텀 모사 control이 없고 native accessibility smoke 통과. | XII.2.2, XII.8.1 |
| XII.9.2 | `[!]` | Windows 앱 셸의 navigation, command bar, pane, dialog가 WinUI 3 Fluent control과 system theme를 사용하도록 theme capability layer를 구현합니다. 캔버스 문서 렌더링에는 적용하지 않습니다. | UI hierarchy와 accessibility smoke 통과. | XII.2.3, XII.8.2 |
| XII.9.3 | `[ ]` | ShellTheme event를 dark/light, high contrast, reduced motion, accent change로 normalize해 앱 UI와 canvas EvaluationEnvironment에 분리 전달합니다. | theme change 후 앱 UI와 document preview의 target environment event가 각각 한 번 발생. | VIII.7.2, XII.9.1, XII.9.2 |
| XII.9.4 | `[ ]` | macOS와 Windows 앱 셸의 visual snapshot은 동일할 필요가 없지만 native control usage, keyboard navigation, contrast를 platform별 test로 검증합니다. | platform UI conformance report 생성. | XII.9.1~XII.9.3 |

### XII.10 캔버스 편집 UX

> 기준: `docs/design/ui/canvas_editing.md`(이하 "캔버스 명세"). 전문 UI 디자인 도구 사용자가 기대하는 편집 관례를 개념 수준에서 따르되 화면·문구·고유 동작을 옮기지 않으며, 캔버스 조작은 `docs/design/property_model.md`의 CSS 기반 속성에 대한 편집 의도를 만듭니다. 모든 행은 한 제스처 한 history entry, 키보드·VoiceOver 경로, 아키텍처 §6.4 예산을 함께 지킵니다.

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XII.10.1 | `[ ]` | 캔버스 도구 11종(`docs/design/ui/tools_menus.md` 6.2)을 코어 closed enum `CanvasTool`로 정의하고 현재 도구를 WindowViewState에 둡니다. 누르는 동안 Hand(Space), 누르는 동안 깊은 선택(⌘), Escape 복귀 같은 일시 전환을 도구 상태 기계에 넣습니다. 도구 상태는 저장·history 대상이 아닙니다(캔버스 명세 3장). | 도구 전환·일시 전환·Escape 복귀 fixture 통과, 전환 전후 save bytes와 history 길이 동일. | XII.3.6, VII.3.1 |
| XII.10.2 | `[ ]` | 가이드 열린 결정 O10을 닫는 ADR을 작성합니다. 화살표·Shift-화살표 이동 단위, 키보드 크기 조절 키, 드래그 수식키(Shift·Option·⌘·Space) 의미, 재부모화 방지 수식키, 스냅 허용 오차(화면 pt), 줌 단계와 상하한, 핸들을 숨기는 화면 크기 기준을 캔버스 명세 3·5·7·16장 제안과 비교해 정하고, 확정 수치를 가이드 12장에 기록합니다. | `cargo xtask validate-adr` 통과, ADR Accepted, ADR 값과 캔버스 명세·가이드 12장 기록이 일치. | 없음 |
| XII.10.3 | `[ ]` | 클릭 선택을 구현합니다. hit test 결과를 현재 편집 범위(Editing Scope)의 직계 자식까지 올려 선택하고, Shift-클릭은 토글 추가, 빈 곳 클릭은 선택 해제, ⌘A는 편집 범위 형제 전체 선택입니다. Escape는 캔버스 명세 3.2절 순서(진행 중 제스처 취소, 그다음 Move가 아닌 도구에서 Move로 복귀, 그다음 편집 범위 한 단계 위, 최상위 편집 범위이면 선택 해제)를 따르며, 이 행은 최상위 편집 범위의 선택 해제를 맡고 편집 범위 한 단계 위는 XII.10.4가 맡습니다. 다중 선택에서 이미 선택된 node를 Option-클릭하면 primary로 지정합니다(캔버스 명세 3.3절, 8장). Fragment는 묶음 단위, ComponentInstance는 한 덩어리로 선택하고 결과는 SetSelection 하나입니다. | `canvas/select-scope-nested` fixture(Root Frame 아래 Frame, Fragment, Rectangle, ComponentInstance 중첩)에서 클릭 위치별 선택 결과와 history 1건 일치, 최상위 편집 범위 Escape의 선택 해제 fixture, Option-클릭 primary 지정 fixture. | XI.3.4, VII.3.2, XII.10.1 |
| XII.10.4 | `[ ]` | 깊은 선택과 편집 범위 이동을 구현합니다. ⌘-클릭은 가장 깊은 선택 가능 node, 더블클릭·Return은 한 단계 아래, Shift-Return은 부모, Tab·Shift-Tab은 OrderKey 순 형제 이동, Shift-더블클릭은 깊은 node 추가이며, 편집 범위가 최상위가 아니면 Escape는 편집 범위를 한 단계 위로 올립니다(캔버스 명세 3.2절). 편집 범위는 WindowViewState에 두고, 선택이 범위 밖으로 나가면 선택의 가장 가까운 공통 조상으로 다시 맞춥니다. | `canvas/select-scope-nested`에서 진입·이탈·형제 순환·사이드바 깊은 선택 후 범위 결과 일치, 범위 변경이 save bytes와 history에 없음. | XII.10.3 |
| XII.10.5 | `[ ]` | select-through 규칙을 구현합니다. pointer down이 현재 선택 bounds 안이면 위에 다른 node가 있어도 드래그는 선택을 옮기고, 움직이지 않고 놓으면 그 위치를 클릭 규칙으로 선택합니다. 잠금·숨김 node는 캔버스 hit에서 통과시키고, 커서 아래 겹친 node 목록(잠금 포함, z 역순)을 문맥 메뉴의 겹친 레이어 고르기 하위 메뉴(Select Overlapping, 캔버스 명세 4.3절)용으로 반환합니다. | 3단 겹침 fixture에서 drag·click·목록 결과 일치, 잠금 node가 캔버스 클릭으로 선택되지 않고 목록에는 존재. | XII.10.3, XI.3.3 |
| XII.10.6 | `[ ]` | 영역 선택(마퀴)을 구현합니다. 빈 캔버스나 Root Frame 빈 영역에서 시작하고, 기본은 영역에 걸친 편집 범위 node, Option은 완전히 포함된 node, ⌘는 가장 깊은 node, Shift는 기존 선택에 더하기(AreaSelectionMode Add)입니다. 진행 중 결과는 transient preview이고 pointer up에서 SetSelection 하나만 만듭니다. | 수식키 4종 matrix fixture 통과, 1,000 move event에서 history 1건, 2만 가시 node fixture에서 갱신 p99 8ms 이하. | VII.3.3, XII.10.3, XI.3.1 |
| XII.10.7 | `[ ]` | hover 미리보기를 구현합니다. 지금 클릭하면 선택될 node(누른 수식키 반영)의 윤곽을 transient overlay로 그리고, 사이드바 레이어 행 hover와 캔버스 hover를 같은 hover 상태로 동기화합니다. hover는 history·저장 대상이 아닙니다. | hover 대상과 클릭 선택 결과가 수식키 조합마다 같다는 property test, 2만 가시 node hover p99 8ms 이하(목표 5ms). | XII.10.3, XII.4.4 |
| XII.10.8 | `[ ]` | 변형 핸들 hit layer를 구현합니다. 모서리·변 크기 핸들, 모서리 바깥 회전 영역, 모서리 안쪽 반경 핸들, padding·gap 핸들 자리의 우선순위(반경, 크기, padding·gap, 회전, 본체 순)와 작은 선택에서 핸들을 숨기는 규칙을 구현하고, 회전된 node는 local 좌표로 계산합니다. | 우선순위·숨김 경계 fixture, 45도 회전 node 핸들 위치 fixture 통과, overlay가 project snapshot에 없음. | XI.3.5, XII.4.4 |
| XII.10.9 | `[ ]` | 크기 조절 제스처를 구현합니다. Shift는 비율 유지(비율 고정 속성이 켜져 있으면 일시 해제), Option은 중심 기준, 반대편을 넘으면 뒤집기(Text는 0에서 멈춤), 다중 선택은 선택 bounds 기준 비례 배치입니다. 뒤집기는 node local 축 기준(핸들이 local 좌표, XII.10.8)으로 Frame의 그 축 `scale` 부호를 바꾸고(캔버스 명세 5.6절), `scale` longhand가 없는 node 종류는 XII.10.44가 정한 반전 표현을 씁니다. 저장된 `transform-origin`은 바꾸지 않고, 고정된 반대편 변의 world 위치가 그대로 남도록 위치(위치 지정 자식은 inset, 흐름 자식은 `translate`)를 보정합니다. 흐름 컨테이너 자식은 조절한 축을 Fixed preset으로 바꾸고 변 핸들 더블클릭은 그 축을 HugContent preset(`width: fit-content` 등)으로 바꾸며, preset과 longhand 변환은 IX.11.7 함수를 씁니다. 크기 longhand는 IX.11.6 영속 record와 IX.11.3 Shape·Image·Text PropertyKey로 씁니다. 속성 이름과 저장 필드 대응은 `docs/design/property_model.md`를 따르며 pointer up에서 CommandTransaction 하나를 확정합니다. | 수식키·뒤집기·다중 선택·흐름 자식·더블클릭 matrix fixture 통과, migration 노드(`transform-origin: 0 0`) 뒤집기 fixture에서 저장된 origin 불변·고정 변 world 위치 불변·`scale` 부호만 반전, 흐름 자식 결과 longhand가 IX.11.7 preset 변환 결과와 일치, drag 500 update에서 history 1건, Escape 취소 시 history 0건. | XII.10.8, XII.3.7, VII.4.2, XII.10.2, IX.11.3, IX.11.6, IX.11.7, IX.11.9, XII.10.44 |
| XII.10.10 | `[ ]` | 회전 제스처를 구현합니다. pivot은 단일 선택이면 node border box 중심, 다중 선택이면 선택 bounds 중심이고 Shift는 15도 단위입니다. 결과는 `rotate` 편집(`docs/design/property_model.md`)이며 제스처는 저장된 `transform-origin`을 바꾸지 않습니다. 저장된 origin이 pivot과 다르면(IX.11.9 migration 노드의 `0 0` 등) pivot의 world 위치가 그대로 남도록 위치를 보정합니다. 위치 지정 자식은 현재 anchor의 inset, 흐름 자식은 `translate`를 바꿉니다. 다중 선택은 각 node의 위치와 `rotate`를 함께 바꿉니다. 회전 중심 이동 핸들 제공 여부(캔버스 명세 C3)를 이 행에서 정하고, 핸들을 두면 그 핸들만 `transform-origin`을 바꾸며 같은 보정으로 외형을 유지합니다. | 단일·다중·Shift 단위 fixture가 reference 행렬과 허용 오차 안에서 일치, migration 노드 fixture(`transform-origin: 0 0`, 선형 `transform` 행렬 보존, 이후 사용자가 `scale`을 더한 경우 포함)에서 저장된 `transform-origin` 불변·pivot world 위치 불변·0도로 되돌린 뒤 scene geometry가 회전 전과 같음, 흐름 자식 회전 fixture에서 형제 layout 불변, C3 결정과 근거가 보고서에 있고 고른 규칙의 fixture 통과, gesture당 history 1건. | XII.10.8, XII.10.2, IX.11.9 |
| XII.10.11 | `[ ]` | 모서리 반경 핸들을 Frame·Rectangle·Polygon에 구현합니다. Frame·Rectangle은 드래그가 네 모서리 동시, Option은 그 모서리만 바꾸고, bounds를 넘는 값은 VI.5.2 평가 규칙으로 표시만 줄이며 저장값은 입력값을 유지합니다. 반경은 4 모서리 `border-*-radius` longhand key로 씁니다. Frame key는 IX.11.22가 `corner_radius`(0x0106)를 나눠 만들고, Rectangle key는 IX.11.3이 만듭니다(ADR-0024 결정 14). Polygon은 모든 꼭짓점에 쓰는 반경 하나만 있으므로(`crates/document/src/shape.rs` `corner_radius: f32`, 속성 모델 6장 `stellar-only`) IX.11.3의 단일 stellar-only key를 바꾸며 Option 개별 편집이 없고, 표시는 Polygon path 평가(VI.5.6) 결과를 따릅니다. | Frame·Rectangle 동시·개별·oversize fixture 통과, Polygon 반경 드래그가 단일 key 하나만 바꾸고 Option에서도 같은 결과인 fixture, 핸들 수치가 저장값을 표시, 새 command가 묶음 key 0x0106을 쓰지 않음. | XII.10.8, VI.5.2, IX.11.3, IX.11.22 |
| XII.10.12 | `[ ]` | 비례 확대(Scale) 명령을 구현합니다. 크기 조절과 달리 선 두께, 반경, 그림자·블러 반경, 글자 크기(`font-size`, IX.11.11 Typography record)까지 같은 비율로 바꾸고 modifier 값 변경을 같은 CommandTransaction에 넣습니다. 선 두께는 modifier Stroke 두께와 Frame `border-*-width` longhand(IX.11.10)이며, 묶음 key 0x0105를 쓰지 않습니다. Object 메뉴와 인스펙터 비율 입력으로 실행하며, 메뉴 막대 Object 메뉴에 Scale 항목을 추가하고 `docs/design/ui/tools_menus.md` 3.2·6.1절 표와 `ShortcutPolicy`를 함께 갱신합니다. 단일 키 배정은 XII.10.2 ADR이 정합니다. | 200% 확대 fixture에서 modifier·text·Frame border 두께 값까지 기대값 일치, 새 command가 묶음 key 0x0105를 쓰지 않음, undo 1회로 원복, `ShortcutPolicy.audit()` 문제 0건. | XII.10.9, VII.2.6, IX.11.3, IX.11.10, IX.11.11, XII.6.5 |
| XII.10.13 | `[ ]` | 이동 드래그를 구현합니다. Shift는 축 고정, Option으로 시작하면 복제 후 복제본 이동(DuplicateSubtree와 이동을 한 transaction), 자유 배치 컨테이너 사이로 끌면 world 위치를 유지한 채 MoveNode로 부모를 바꿉니다. 결과는 위치 longhand 편집(위치 지정 자식은 현재 anchor의 inset, `docs/design/property_model.md` 4.5절·5장·12장)이며 ComponentInstance 안에는 공개 Slot에만 놓입니다. Frame의 위치 longhand는 IX.11.9가 transform에서 분리하고, Frame이 아닌 node의 위치 longhand는 IX.11.6 item record에 있습니다. Ellipse·Polygon `bounds` 원점과 Transform modifier 이동 성분은 바꾸지 않습니다. | 축 고정·복제·재부모화·Slot 밖 거부 fixture 통과, 이동 후 `transform`·`translate` 값 불변, Rectangle·Ellipse 이동 fixture에서 바뀐 값이 item record 위치 longhand뿐(bounds 원점·Transform modifier 불변), gesture당 history 1건. | XII.10.5, XII.3.7, VII.2.8, VII.2.3, IX.11.6, IX.11.9 |
| XII.10.14 | `[ ]` | 키보드 이동·크기 조절을 구현합니다. 화살표와 Shift-화살표는 XII.10.2 단위로 위치 longhand(Frame은 IX.11.9, 그 밖의 node는 IX.11.6 item record)를 바꿔 이동하고, 크기 조절 키는 오른쪽·아래 가장자리를 옮기며, 흐름 컨테이너 자식은 주축 화살표로 흐름 순서를 바꾸고, 선택이 없으면 화살표가 camera를 팬합니다. 키 반복은 VII.4.4 규칙으로 한 entry로 합칩니다. | key repeat 20회 history 1건, 흐름 자식 재정렬 fixture, Rectangle 화살표 이동이 item record 위치 longhand만 바꿈, 선택 없음에서 문서 revision 불변. | XII.10.2, VII.4.4, XII.10.3, IX.11.6, IX.11.9 |
| XII.10.15 | `[ ]` | 스냅 엔진을 코어 순수 함수로 구현합니다. 후보는 가이드, 레이아웃 그리드, 편집 범위 형제·부모의 가장자리와 중심, 같은 간격, 같은 크기, 픽셀 그리드이고 이 순서로 우선하며, 허용 오차는 XII.10.2의 화면 pt 값입니다. 후보 집합은 제스처 시작 때 가시 영역 공간 인덱스에서 한 번 만들고 update마다 재사용하며, 흐름 컨테이너 자식에는 객체 스냅을 적용하지 않습니다. | `canvas/snap-candidates-20k` fixture에서 결과 결정론, update당 계산 p99 0.35ms 이하, 줌 25%와 400%에서 화면 허용 오차 동일. | XI.3.1, XI.3.2, XII.10.2 |
| XII.10.16 | `[ ]` | 스마트 가이드와 측정 overlay를 transient scene layer로 그립니다. 정렬선, 가장자리 거리 수치, 같은 간격·같은 크기 표시, 선택 후 Option-hover 거리 측정을 포함하고, 대비 높이기·투명도 줄이기 ShellTheme 값(XII.9.3, 투명도 줄이기 포함 여부는 가이드 O6으로 XII.9.3이 정함)을 반영합니다. 측정 라벨 glyph 출처와 Text baseline 기준 측정 방법(캔버스 명세 C2, C5)을 이 행에서 정합니다. | overlay golden scene fixture, overlay가 project snapshot에 없음, 대비 높이기 on과 off의 서로 다른 golden 존재, C2·C5 결정과 근거가 보고서에 있고 고른 라벨 glyph 출처와 baseline 측정 규칙의 golden fixture 통과. | XII.10.15, XII.4.4, XII.9.3 |
| XII.10.17 | `[ ]` | 객체 스냅, 픽셀 그리드 스냅, 스마트 가이드 표시 토글을 View 메뉴 `ShellCommand`로 추가하고 드래그 중 스냅 일시 해제 수식키를 연결합니다. 토글 값은 앱 설정이며 문서·Git에 넣지 않습니다. | `ShortcutPolicy.audit()` 문제 0건, 토글 후 save bytes 불변, 일시 해제 중 snap 결과 없음. | XII.10.15, XII.6.5 |
| XII.10.18 | `[ ]` | 눈금자 가이드와 Frame 레이아웃 그리드의 저장 위치를 정하는 ADR을 작성합니다. 문서 데이터(ViewDefinition·Frame 소속, Git·Semantic Diff 대상)와 창별 view 상태를 비교하고 파일 포맷·migration·Semantic Diff·Merge 영향을 함께 정합니다. | `cargo xtask validate-adr` 통과, ADR Accepted. | 없음 |
| XII.10.19 | `[ ]` | 눈금자와 가이드를 구현합니다. ⌘R로 표시하고, 눈금자에서 끌어 가이드를 만들고 다시 눈금자로 끌어 지우며, 눈금 원점은 선택된 최상위 Frame 기준입니다. 저장은 XII.10.18 결정을 따르고 문서 데이터이면 가이드 추가·이동·삭제를 일반 Command로 history에 넣습니다. 눈금자 표현 방식(캔버스 명세 C1)을 이 행에서 정합니다. | 가이드 생성·이동·삭제·스냅 fixture, camera 이동 후 눈금 값 정확, C1 결정과 근거가 보고서에 있고 고른 눈금자 표현 방식의 camera 동기 fixture 통과. | XII.10.15, XII.10.18, XII.4.3 |
| XII.10.20 | `[ ]` | Frame 레이아웃 그리드(열·행·폭·gutter)와 픽셀 그리드 overlay를 구현합니다. 레이아웃 그리드는 자식 배치를 바꾸지 않는 설계 기준이고, 픽셀 그리드는 XII.10.2의 줌 기준 이상에서만 보이며 스냅은 표시 여부와 무관하게 설정을 따릅니다. | 12열 그리드 snap 결과 fixture, 줌 경계 표시 fixture, 그리드 유무에 따른 layout 결과 동일. | XII.10.15, XII.10.18 |
| XII.10.21 | `[ ]` | 정렬 6종, 분배 2종, 행·열 정리(Arrange in Rows and Columns, 선택을 같은 간격 행·열로 재배치하며 흐름 컨테이너를 만들지 않음)를 구현합니다. 단일 선택은 부모 기준, 다중 선택은 선택 bounds 기준이며 primary selection을 기준 객체로 쓸 수 있고, 분배는 가장 바깥 두 객체를 고정합니다. 흐름 밖 node는 위치 longhand(Frame은 IX.11.9, 그 밖의 node는 IX.11.6 item record)를 바꿉니다. 흐름 자식은 캔버스 명세 8장 규칙을 따릅니다. Stack 자식의 주축 정렬은 비활성화하고 컨테이너 `justify-content` 안내를 도움말로 보이며, 교차축 정렬은 자식 `align-self`를 바꿉니다. Grid 자식은 `justify-self`·`align-self`를 바꾸고, Overlay 자식의 정렬과 모든 흐름 자식의 분배·행·열 정리는 비활성화합니다. 결과는 CommandTransaction 하나입니다. 메뉴 막대에 없는 행·열 정리는 Object 메뉴에 항목을 추가하고 `docs/design/ui/tools_menus.md` 3.2·6.1절 표와 `ShortcutPolicy`를 함께 갱신합니다. | `canvas/align-distribute-matrix` fixture 통과, 회전 node는 world bounds 기준, Stack 교차축·Grid 정렬 fixture에서 바뀐 longhand가 `align-self`·`justify-self`뿐, 비활성 조합 fixture에서 문서 revision 불변, undo 1회 원복, `ShortcutPolicy.audit()` 문제 0건. | VII.1.6, VII.2.4, XII.10.3, IX.11.4, IX.11.5, IX.11.6, IX.11.9, XII.6.5 |
| XII.10.22 | `[ ]` | 같은 간격으로 놓인 다중 선택에 간격 핸들과 자리 바꾸기 핸들을 표시합니다. 간격 핸들 드래그는 모든 간격을 함께 바꾸고, 가운데 핸들 드래그는 이웃과 위치를 바꿉니다. | 등간격 판정 fixture, 간격 변경·자리 바꾸기 결과와 gesture당 history 1건. | XII.10.21, XII.10.8, XII.10.15, IX.11.6, IX.11.9 |
| XII.10.23 | `[ ]` | 순서와 묶음 명령을 구현합니다. Bring Forward, Send Backward, Bring to Front, Send to Back은 OrderKey를 바꾸는 MoveNode, Group은 Fragment 삽입과 자식 MoveNode, Frame Selection은 Frame 삽입과 자식 MoveNode, Ungroup은 자식을 부모로 옮기고 빈 Fragment를 지우는 CommandTransaction입니다. Frame을 풀 때는 world 위치를 보존하고, 흐름 컨테이너에서 순서 명령이 흐름 순서도 바꾼다는 사실을 도움말과 접근성 알림으로 알립니다. | group 후 ungroup의 semantic snapshot 동일, 순서 명령 4종 OrderKey fixture, 흐름 컨테이너 알림 fixture. | VII.2.1, VII.2.2, VII.2.3, VII.1.6, XII.10.3 |
| XII.10.24 | `[ ]` | 흐름 컨테이너(Stack·Grid) 안 드래그를 구현합니다. 끄는 동안 삽입 위치 표시를 그리고 형제를 증분 레이아웃 preview로 비키게 하며, 놓으면 OrderKey를 바꾸는 MoveNode 하나를 만듭니다. 밖으로 끌어내면 대상 부모 규칙으로 옮기고, 흐름에서 빠진 자식(`position: absolute`)은 inset 편집으로 움직이며, ComponentInstance 내부 재정렬은 거부합니다. 흐름 순서는 OrderKey로, inset은 IX.11.6 영속 record 값으로 씁니다. Grid 명시 배치 자식 처리(캔버스 명세 C6)를 이 행에서 정합니다. | Stack 가로·세로·wrap과 Grid 자동 배치 fixture에서 삽입 index 정확, 끌어내기·instance 거부 fixture, C6 결정과 근거가 보고서에 있고 Grid 명시 배치 자식 drag fixture가 고른 규칙과 일치, gesture당 history 1건. | X.1.2, X.3.2, X.4.4, XII.10.13, IX.11.6 |
| XII.10.25 | `[ ]` | padding·gap 캔버스 핸들을 구현합니다. 선택한 흐름 컨테이너에 hover하면 네 변 padding 핸들과 자식 사이 gap 핸들을 보입니다. padding 드래그는 `padding-top`·`padding-right`·`padding-bottom`·`padding-left`를 바꿉니다. gap 핸들은 자기 축의 longhand 하나만 바꿉니다. Stack 주축 핸들은 `flex-direction`이 `row` 계열이면 `column-gap`, `column` 계열이면 `row-gap`이고, 줄바꿈 Stack의 줄 사이 핸들은 다른 하나, Grid는 열 사이 `column-gap`·행 사이 `row-gap`입니다. `gap` shorthand는 쓰지 않습니다(속성 모델 11장, ADR-0024 결정 4). 값은 IX.11.6 영속 record에 씁니다. Shift는 큰 단위, Option은 마주 보는 padding 변을 함께 바꿉니다. gap은 음수가 되지 않고, 남는 공간 분배가 켜진 축의 gap 핸들은 표시만 하며, 핸들 클릭은 그 핸들 longhand의 `NSPopover`와 `NSTextField` 숫자 입력을 엽니다. | 드래그·Shift·Option·음수 거부·분배 축 fixture, `flex-direction` 4종·wrap·Grid fixture에서 바뀐 longhand가 핸들 축의 것 하나뿐이고 다른 축 gap 값 불변, 증분 layout 결과가 전체 layout 결과와 일치, gesture당 history 1건. | XII.10.8, X.3.1, X.1.2, IX.11.6 |
| XII.10.26 | `[ ]` | Frame·Rectangle·Ellipse·Line·Polygon 생성 도구를 구현합니다. 드래그는 영역 생성(Shift 정비례·45도, Option 중심 기준, 드래그 중 Space는 시작점 이동), 클릭은 기본 크기 생성이며, 흐름 컨테이너 위에서 만들면 pointer 위치의 흐름 순서에 넣습니다. InsertNode와 새 선택을 한 entry로 묶고 생성 후 도구는 Move로 돌아갑니다. | 도구별 드래그·클릭·수식키 fixture, 흐름 삽입 index fixture, 생성 1회 history 1건. | XII.10.1, VII.2.1, XII.10.9 |
| XII.10.27 | `[ ]` | Image 도구를 구현합니다. `NSOpenPanel`로 고른 파일을 XIII.5.2 import로 asset에 넣고 Image node를 삽입하며, 취소하면 이전 도구로 돌아갑니다. 캔버스로 파일을 끌어 놓는 경로도 같은 import를 씁니다. | import 성공·취소·지원하지 않는 형식 fixture, 같은 파일 두 번 import 시 asset dedup. | XII.10.26, XIII.5.2 |
| XII.10.28 | `[ ]` | Path point 편집 command를 추가합니다. point 삽입·이동·삭제, handle 이동, handle 대칭 모드 전환, subpath 닫기·열기를 closed enum으로 정의하고 point StableId를 유지합니다. CommandKind registry 변경이므로 전역 mutex 작업입니다. | command별 apply/inverse fixture, point 삽입 후 기존 point ID 유지, VII.6.1 property test 등록. | VI.6.3, VII.1.6, VII.6.1 |
| XII.10.29 | `[ ]` | 펜 도구와 벡터 편집 모드를 구현합니다. 클릭은 직선 point, 드래그는 대칭 handle, Option-드래그는 비대칭, Shift는 45도, 첫 point 클릭은 닫기, Return·Escape는 끝내기, Delete는 마지막 point 취소입니다. Path의 Return·더블클릭은 벡터 편집 모드이고 Shape는 명시적 Convert to Path(VII.2.7) 뒤에만 들어가며, 분기형 vector network는 지원하지 않습니다. | `canvas/pen-sequence` fixture가 reference Path command와 일치, Shape 더블클릭이 변환하지 않음, point drag gesture당 history 1건. | XII.10.28, XII.10.1, XII.10.8, VII.2.7 |
| XII.10.30 | `[ ]` | Boolean modifier geometry 평가를 결정론적 순수 함수로 구현합니다. 먼저 operand로 참조된 node의 단독 표시 여부와 operand 순서 규칙을 ADR로 정한 뒤, Union·Subtract·Intersect·Exclude 결과 Path를 scene builder에 연결합니다. | 4개 연산 analytic fixture 허용 오차 통과, 같은 입력 bit-stable 결과, ADR Accepted. | VI.9.4, VI.5.6, XI.2.2 |
| XII.10.31 | `[ ]` | Boolean 명령을 Object 메뉴와 문맥 메뉴에 연결합니다. 메뉴 막대 Object 메뉴에 Boolean 하위 메뉴(연산 4종)를 추가하고 `docs/design/ui/tools_menus.md` 3.2·6.1절 표와 `ShortcutPolicy`를 함께 갱신합니다. 같은 부모의 Shape·Path 둘 이상을 선택하면 맨 뒤 node에 Boolean modifier를 넣고 나머지를 operand로 연결하며, 연산 종류 변경과 operand 분리를 지원합니다. Path로 굽는 평탄화는 별도 명시 명령으로만 실행합니다. | 선택 조건 위반 거부 fixture, 연산 변경·분리 apply/inverse, 평탄화 전후 렌더 geometry 동일, `ShortcutPolicy.audit()` 문제 0건. | XII.10.30, VII.2.6, XII.6.5 |
| XII.10.32 | `[ ]` | Text 내용·style 범위 편집 command를 추가합니다. text range 교체와 범위 style 설정을 VI.7.5 helper로 구현하고 입력 종류와 caret 정보를 VII.4.3 grouping key로 넘깁니다. CommandKind registry 변경이므로 전역 mutex 작업입니다. | 삽입·삭제·교체·범위 style apply/inverse fixture, 한글 조합 확정 fixture, 연속 입력 merge fixture. | VI.7.5, VII.4.3, VII.1.6 |
| XII.10.33 | `[ ]` | 캔버스 텍스트 편집을 구현합니다. Text 도구 클릭은 자동 폭, 드래그는 고정 폭 Text를 만들고, 더블클릭·Return은 편집 모드, Escape와 바깥 클릭은 편집을 끝내고 Text node 선택으로 돌아갑니다. 조합 중 문자열은 transient로 그리고 확정 때만 XII.10.32 command를 만들며, 새로 만든 Text를 입력 없이 끝내면 생성을 취소합니다. 상자 변 드래그는 자동 폭을 고정 폭으로 바꿉니다(`docs/design/property_model.md`). | 한글·일본어 조합 중간·확정 fixture, 빈 생성 취소 시 history 0건, 편집 중 glyph source가 common text engine. | XII.10.32, XII.5.4, XII.5.6, X.10.3, XII.10.26 |
| XII.10.34 | `[ ]` | ComponentInstance 캔버스 편집을 구현합니다. 인스턴스 안 깊은 선택은 읽기 전용 윤곽으로 보이고 공개 Property에 연결된 값 편집만 PropertyOverrideCommand Set으로 보내며, 허용되지 않은 편집은 적용하지 않고 이유와 Detach 경로를 보입니다. 문맥 메뉴로 속성별·전체 Reset, Detach(IX.2.6), 정의로 이동을 제공하고 공개 Slot에는 끌어 놓기로 콘텐츠를 넣습니다. Reset Overrides·Detach Instance·정의로 이동은 메뉴 막대 Object 메뉴에도 추가하고 `docs/design/ui/tools_menus.md` 3.2·6.1절 표와 `ShortcutPolicy`를 함께 갱신합니다. | override·reset·detach·slot 삽입 fixture, 비공개·nested 직접 편집 거부 fixture, 동작마다 history 1건, `ShortcutPolicy.audit()` 문제 0건. | IX.2.4, IX.2.6, IX.3.3, IX.4.2, XII.10.4, XII.6.5 |
| XII.10.35 | `[ ]` | 인스턴스 교체(swap) command를 추가합니다. ComponentRef를 바꾸면서 이름과 TypeSpec이 같은 공개 Property의 override를 보존하고, 보존하지 못하는 override 목록을 적용 전 preview로 반환합니다. 크기는 새 정의 기본 크기를 쓰고 현재 크기 유지 선택지를 둡니다. 메뉴 막대 Object 메뉴에 Swap… 항목을 추가하고 `docs/design/ui/tools_menus.md` 3.2·6.1절 표와 `ShortcutPolicy`를 함께 갱신합니다. CommandKind registry 변경이므로 전역 mutex 작업입니다. | 보존·탈락 matrix fixture, undo 후 원래 ComponentRef와 override 복원, 같은 입력 같은 preview, `ShortcutPolicy.audit()` 문제 0건. | IX.2.4, IX.1.2, VII.1.6, XII.6.5 |
| XII.10.36 | `[ ]` | `InputEvent`에 확대 제스처(Magnify, phase 포함)와 smart magnify 종류를 추가합니다. `StellarMacEvent`에 `NSEvent.magnification` 값을 담는 필드를 더하고, 지금 변환하지 않는 magnify 이벤트(`crates/ffi/src/input_macos.rs` `event_type::MAGNIFY`)와 smart magnify 이벤트를 변환합니다. ADR-0014 Compatibility와 각 상수 문서 주석의 규칙에 따라 `STELLAR_MAC_EVENT_VERSION`(`MAC_EVENT_VERSION`)과 `INPUT_EVENT_SCHEMA_VERSION`을 올리고 이전 version은 거부합니다. `crates/ffi` 변경이므로 같은 diff에서 ADR-0014에 이 작업 절을 추가하고(I.3.5 보호 경로 정책), `cargo xtask generate-ffi-header`로 헤더를 다시 만듭니다. | 새 kind round-trip, 이전 version `StellarMacEvent`·입력 encoding 거부 fixture, 알 수 없는 kind 거부 유지, 같은 diff에 ADR-0014 갱신이 있어 `cargo xtask validate-adr` 통과, `cargo xtask generate-ffi-header --check` drift 없음. | XII.3.1, XII.3.2 |
| XII.10.37 | `[ ]` | 줌·팬 입력을 camera에 연결합니다. 핀치는 커서 기준 연속 줌, 두 손가락 스크롤은 관성 phase를 유지한 팬, ⌘+스크롤은 커서 기준 줌, 마우스 Shift+휠은 가로 팬, Space 드래그는 일시 Hand, Zoom 도구는 클릭 확대·Option-클릭 축소·영역 드래그 맞춤, smart magnify는 선택 맞춤과 이전 배율 전환입니다. 줌 단계와 상하한은 XII.10.2를 따르고 문서 geometry와 history를 바꾸지 않습니다. | 입력별 camera 결과 fixture, pan·zoom 후 save bytes와 history 불변, Pan·Zoom replay Present 예약 10ms 이하. | XII.10.36, XII.4.3, XII.10.2 |
| XII.10.38 | `[ ]` | 캔버스 클립보드를 구현합니다. Copy·Cut은 선택 subtree를 Stellar 전용 pasteboard type과 Text 일반 텍스트로 쓰고, Paste는 선택한 컨테이너 안, 아니면 원본 위치가 보이면 같은 위치, 아니면 화면 가운데에 IdRemapTable로 새 ID를 붙여 넣습니다. 다른 프로젝트의 Component·asset 참조 처리 규칙을 진단과 함께 고정합니다. | 같은 창·다른 창·다른 프로젝트 paste fixture, 중복 StableId 0, 대규모 paste 최초 시각 반응 10ms 이하. | VII.2.8, III.1.5, XII.6.5 |
| XII.10.39 | `[ ]` | 캔버스 문맥 메뉴를 구현합니다. 우클릭·Control-클릭 위치의 hit 결과와 현재 선택으로 `ShellCommand` 항목만 담은 `NSMenu`를 만들고, 겹친 레이어 고르기 하위 메뉴(Select Overlapping, XII.10.5), 순서·묶음·Boolean·변환·인스턴스 항목을 상황에 맞게 비활성화하며, VoiceOver AXShowMenu로도 같은 메뉴를 엽니다. 메뉴 막대에 아직 없는 Convert to Path·Convert to Frame(VII.2.7)은 Object 메뉴에 추가하고 `docs/design/ui/tools_menus.md` 3.2·6.1절 표와 `ShortcutPolicy`를 함께 갱신합니다. Scale, 행·열 정리, Boolean, 인스턴스 항목의 메뉴 막대 추가는 XII.10.12, XII.10.21, XII.10.31, XII.10.34, XII.10.35가 맡습니다. | hit 대상별 메뉴 구성 snapshot, Select Overlapping 하위 메뉴의 동적 레이어 목록 항목을 뺀 모든 항목 command ID가 메뉴 막대와 같음, `ShortcutPolicy.audit()` 문제 0건, AXShowMenu smoke. | XII.6.5, XII.10.5, XII.10.12, XII.10.21, XII.10.31, XII.10.34, XII.10.35 |
| XII.10.40 | `[ ]` | 한 제스처 한 history 적합성 검사를 추가합니다. 선택하지 않은 node를 눌러 바로 끄는 경우처럼 선택 변경과 편집이 한 제스처에서 일어나면 undo 한 번으로 둘 다 되돌리고, 취소한 제스처·hover·camera·도구 전환·편집 범위 이동은 기록하지 않습니다. `HistoryEntry` 타입은 바꾸지 않습니다. 문서 항목은 이미 `SelectionRestore`(편집 직전·직후 선택)를 담고(`crates/history/src/entry.rs`), `WindowHistory::begin_pointer_gesture`가 제스처 전 선택을 잡아 `end_preview`가 기록합니다(`crates/history/src/window.rs`). preview 중 `record_selection`은 `PreviewActive`로 거부되므로(`crates/history/src/history.rs`), `WindowHistory`에 pointer gesture 안에서 선택을 항목 없이 view에 반영하는 API를 추가합니다. 규칙: 제스처가 문서 항목을 만들면 그 항목의 `SelectionRestore`가 제스처 전·후 선택을 담고 Selection 항목은 만들지 않습니다. 편집 없이 끝나고 선택이 바뀌었으면 Selection 항목 하나를 기록합니다. 취소하면 제스처 전 선택으로 돌리고 기록하지 않습니다. | 캔버스 명세 19장 표의 제스처마다 history entry 수 기대값 fixture 통과, 선택하지 않은 node 즉시 drag fixture에서 undo 1회 후 문서와 선택이 모두 제스처 전과 같음, 움직이지 않은 click fixture에서 Selection 항목 1건, 취소 fixture에서 항목 0건과 선택 원복. | VII.4.2, VII.4.4, VII.4.6, XII.3.7, XII.10.9, XII.10.13 |
| XII.10.41 | `[ ]` | 키보드만으로 하는 캔버스 편집 경로를 완성합니다. 캔버스 포커스 표시, Tab·Return·Shift-Return 선택 이동, 화살표 이동, 키보드 크기 조절, Insert 메뉴로 화면 가운데 기본 크기 생성, 문맥 메뉴 열기를 제공하고, 핸들로만 할 수 있는 동작이 없도록 각 동작에 메뉴 명령 또는 인스펙터 필드를 연결합니다. | 캔버스 명세 20.1절 표의 모든 동작을 키보드만 쓰는 UI 자동화 시나리오로 완료, Full Keyboard Access smoke. | XII.10.4, XII.10.14, XII.10.39, XII.8.1 |
| XII.10.42 | `[ ]` | VoiceOver 캔버스 경로를 구현합니다. 선택 node를 접근성 요소(역할, 이름, 위치·크기 값)로 노출하고 이동·크기 조절·안으로·밖으로·메뉴 열기 custom action을 연결하며, 선택 변경과 제스처 결과를 접근성 알림으로 짧게 알리되 연속 조작은 끝날 때 한 번만 알립니다. | VoiceOver automation smoke에서 custom action이 마우스 제스처와 같은 command를 만듦, 연속 조작 알림 1회 fixture. | XII.8.1, XII.10.41 |
| XII.10.43 | `[ ]` | 캔버스 편집 성능 replay를 추가합니다. hover, 클릭 선택, 마퀴, 스냅 포함 이동, Stack 안 크기 조절, 회전, padding 핸들, 펜 point 드래그, IME 조합 입력 scenario를 2만 가시 node fixture(XVI.3.2)로 만들고, hover·선택·핸들 표시는 목표 5ms·상한 8ms, 나머지는 Present 예약 10ms 이하로 판정합니다. | scenario별 최소 1,000 sample 결과 JSON, analyzer 판정 통과. | XVI.2.3, XVI.4.2, XVI.3.2, XII.10.7, XII.10.16, XII.10.25, XII.10.33 |
| XII.10.44 | `[ ]` | Object ▸ Flip Horizontal·Flip Vertical(`object.flip_*`, `docs/design/ui/tools_menus.md` 3.2·6.1절) 명령을 구현합니다. 반전은 Frame `scale`의 해당 축 부호 반전(예: `scale: -1 1`, 캔버스 명세 5.6절, 속성 모델 5장)이고 저장된 `transform-origin`은 바꾸지 않습니다. 선택 bounds 중심을 지나는 축에 대해 반전되도록 각 node의 위치(위치 지정 자식은 현재 anchor의 inset, 흐름 자식은 `translate`)를 보정합니다. 단일 선택은 그 node bounds 중심 기준이므로 origin이 `50% 50%`인 새 노드는 보정이 없고, migration 노드(`transform-origin: 0 0`)는 bounds 중심 world 위치가 그대로 남도록 보정합니다. 회전된 node(또는 migration 노드의 선형 `transform`)를 node local 축과 world 축 중 어느 축 기준으로 반전할지, 그리고 `scale` longhand가 없는 node 종류(속성 모델 12장 Transform 대상 밖)의 반전 표현(예: Path·Polygon geometry 반전, 표현이 없으면 메뉴 항목 비활성화)을 이 행에서 정해 결정과 근거를 보고서에 기록합니다. 명령 하나는 CommandTransaction 하나입니다. | 단일·다중 선택 반전 fixture에서 선택 bounds 중심 불변, migration 노드 fixture에서 저장된 `transform-origin` 불변·bounds 중심 world 위치 불변, 같은 축 두 번 반전하면 scene geometry가 원래와 같음, 축 기준·비Frame 반전 표현 결정과 근거가 보고서에 있고 45도 회전 node와 각 node 종류의 fixture가 고른 규칙과 일치, undo 1회로 원복. | VII.1.6, VII.2.4, XII.10.3, IX.11.6, IX.11.9, XII.6.5 |

## Phase XIII. 자동 저장, 외부 파일, 에셋·폰트와 복구

### XIII.1 지연 자동 저장 스케줄러

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIII.1.1 | `[~]` | `AutosavePolicy`에 debounce, 최대 지연, 앱 비활성화 flush, 창 닫기 flush를 정의하고 값은 `config/performance-budgets.yaml`에서 읽습니다. | config 누락·범위 밖 값 거부. | Phase XII, V.5.4 |
| XIII.1.2 | `[x]` | 완료된 document CommandTransaction만 autosave dirty state를 만들고 Selection·viewport·preview event는 만들지 않게 합니다. | 선택 1,000회 후 save plan 없음. | VII.3.2, V.5.1 |
| XIII.1.3 | `[ ]` | 연속 command 후 debounce timer를 재설정하고 최대 지연에 도달하면 입력이 이어져도 snapshot을 캡처합니다. 캡처는 document thread에서 짧게 수행합니다. | ManualClock fixture 통과. | XIII.1.1, XII.1.1 |
| XIII.1.4 | `[ ]` | serialization과 chunk write는 worker에서 수행하고 base revision이 오래되면 결과를 current pointer로 전환하지 않습니다. | stale save result discard. | XII.1.4, V.5.2 |
| XIII.1.5 | `[ ]` | Save Now는 대기 autosave를 취소·합치고 최신 snapshot 하나만 저장합니다. | double write 없음. | XIII.1.4 |
| XIII.1.6 | `[ ]` | 창 닫기·앱 비활성화 flush는 timeout을 갖고 실패 시 창 닫기 결정 상태를 shell에 반환합니다. 자동으로 변경을 버리지 않습니다. | timeout·failure fixture 통과. | XIII.1.5 |
| XIII.1.7 | `[ ]` | autosave trace에 dirty revision, capture duration, serialization duration, write duration, result를 기록합니다. 경로·이름은 기록하지 않습니다. | trace schema 통과. | II.3.2, XIII.1.4 |

### XIII.2 파일 감시기

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIII.2.1 | `[x]` | 플랫폼 독립 `ProjectFileWatcher` event에 create, modify, remove, rename, overflow를 정의합니다. | event model round-trip. | Phase XII |
| XIII.2.2 | `[x]` | macOS watcher adapter를 구현하고 프로젝트 root, 외부 symlink target allowlist를 감시합니다. | create/modify/delete/rename fixture 통과. | XIII.2.1 |
| XIII.2.3 | `[!]` | Windows watcher adapter를 구현하고 동일 의미 event로 normalize합니다. | create/modify/delete/rename fixture 통과. | XIII.2.1 |
| XIII.2.4 | `[~]` | Stellar 자신의 save가 만든 filesystem event를 generation/hash로 식별해 외부 변경으로 재처리하지 않게 합니다. | autosave 후 reload loop 0. | V.5.4, XIII.2.2, XIII.2.3 |
| XIII.2.5 | `[x]` | 짧은 시간의 다중 파일 event를 root manifest current pointer 변경 단위로 coalesce합니다. 개별 chunk event만으로 reload하지 않습니다. | 한 save가 한 external change batch. | V.4.3, XIII.2.4 |
| XIII.2.6 | `[x]` | watcher overflow 시 전체 project current pointer와 tracked files rescan을 요청합니다. 임의로 상태를 clean 처리하지 않습니다. | overflow fixture에서 rescan 실행. | XIII.2.1 |

### XIII.3 외부 변경 반영

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIII.3.1 | `[x]` | 창에 로컬 미저장 변경이 없으면 외부의 새 정상 generation을 자동 reload하고 history를 폐기합니다. | clean window reload fixture 통과. | XIII.2.5, VII.5.4 |
| XIII.3.2 | `[x]` | 로컬 변경이 있으면 LoadedBase, CurrentMemory, ExternalDisk snapshot을 보존한 `ExternalChangePending` 상태를 생성합니다. | 세 snapshot hash 기록. | XIII.2.5, V.4.4 |
| XIII.3.3 | `[x]` | Semantic Reconciliation 구현 전에는 pending 상태에서 autosave를 중지하고 read-only alert만 반환합니다. 자동 overwrite를 금지합니다. | pending 중 current pointer 미변경. | XIII.3.2 |
| XIII.3.4 | `[ ]` | Phase XIV Semantic Merge 완료 후 세 snapshot reconciliation을 연결하고 겹치지 않는 변경만 자동 적용합니다. | non-overlap fixture 자동, overlap conflict. | XIII.3.2, XIV.10.9 |
| XIII.3.5 | `[x]` | 잠금된 agent subtree와 외부 변경 충돌 정책은 로드맵 Phase R3에서 연결하며 초기 릴리스에는 agent lock 코드가 비활성임을 검증합니다. | feature off build에 agent state 없음. | XIII.3.2 |

### XIII.4 심볼릭 링크

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIII.4.1 | `[x]` | 파일·디렉터리 symlink를 그대로 허용하는 path resolver를 구현합니다. 내부·외부 target 모두 허용합니다. | internal/external link read fixture 통과. | IV.2.3 |
| XIII.4.2 | `[x]` | symlink cycle과 최대 hop을 검사해 무한 순회를 차단합니다. | self/two-node/deep cycle fixture 오류. | XIII.4.1 |
| XIII.4.3 | `[x]` | broken symlink는 삭제하지 않고 diagnostic으로 표시합니다. | broken link open 시 project의 다른 데이터 정상. | XIII.4.1 |
| XIII.4.4 | `[~]` | 외부 symlink target 변경을 watcher에 동적으로 등록·해제합니다. | target replace 후 event 수신. | XIII.2.2, XIII.2.3, XIII.4.1 |
| XIII.4.5 | `[x]` | 신뢰되지 않은 프로젝트에서는 외부 symlink target을 읽지 않고 target path metadata만 제한적으로 표시합니다. | target content access syscall 없음. | IV.5.3, XIII.4.1 |

### XIII.5 에셋 저장소

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIII.5.1 | `[x]` | `AssetId` StableId newtype과 `AssetRecord { content_hash, media_type, size, source_metadata }`를 정의합니다. | round-trip. | III.1.1, V.1.5 |
| XIII.5.2 | `[x]` | 외부 파일 import 시 streaming hash를 계산하고 `assets/<prefix>/<hash>.<safe-ext>`로 복사합니다. 원본 경로는 optional metadata입니다. | 대형 파일에서 전체 메모리 로드 없음. | XIII.5.1 |
| XIII.5.3 | `[x]` | 동일 content hash는 파일을 재복사하지 않고 새 AssetId가 같은 content를 참조할 수 있게 합니다. | dedup fixture 통과. | XIII.5.2 |
| XIII.5.4 | `[x]` | asset write도 temp+fsync+atomic rename을 사용합니다. | 중단 복사 후 partial final file 없음. | XIII.5.2, V.2.5 |
| XIII.5.5 | `[~]` | `UpdateFromSource`는 source metadata 경로를 다시 읽고 새 hash asset을 만든 뒤 AssetRecord 참조를 command로 변경합니다. | source 없으면 기존 asset 유지. | XIII.5.2, Phase VII |
| XIII.5.6 | `[x]` | project에서 참조되지 않는 asset content GC를 generation/chunk GC와 별도 mark-sweep으로 구현합니다. Git tracked history는 고려하지 않고 current project refs만 다룹니다. | live shared content 미삭제. | XIII.5.3, V.7.4 |
| XIII.5.7 | `[x]` | import 허용 media type과 최대 단일 파일 크기 정책을 config에 정의합니다. unknown type은 binary asset으로 저장 가능 여부를 명시합니다. | boundary fixture 통과. | XIII.5.1 |

### XIII.6 프로젝트·시스템·외부 폰트

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIII.6.1 | `[x]` | Project Font import를 AssetStore와 연결하고 font file hash, face count, licensing metadata 입력 필드를 기록합니다. | multi-face font fixture 등록. | XIII.5.2, X.8.1 |
| XIII.6.2 | `[x]` | macOS system font discovery adapter를 구현하고 실제 file/face를 FontRegistry에 등록합니다. 프로젝트에 복사하지 않습니다. | 알려진 system face 조회 smoke. | X.8.2, II.5.3 |
| XIII.6.3 | `[!]` | Windows DirectWrite 또는 승인 API로 system font discovery adapter를 구현합니다. | 알려진 system face 조회 smoke. | X.8.2, II.6.3 |
| XIII.6.4 | `[x]` | External Font는 관리형 패키지의 고정 hash/face를 참조하는 인터페이스만 구현하고 다운로드 서비스는 제공하지 않습니다. | missing external package diagnostic. | X.8.1 |
| XIII.6.5 | `[~]` | font file hash가 예상과 다르면 같은 family name이어도 다른 face로 처리하고 재현성 경고를 생성합니다. | same name/different hash fixture 통과. | XIII.6.1~XIII.6.4 |
| XIII.6.6 | `[x]` | 라이선스·임베딩 상태는 Unknown, Allowed, Restricted, UserConfirmed로 저장하고 Stellar가 법적 판정을 자동 확정하지 않게 합니다. | 상태 transition fixture 통과. | XIII.6.1 |
| XIII.6.7 | `[x]` | missing font에서 명시 fallback chain만 적용하고 줄바꿈 변경 진단을 생성합니다. | missing face fixture expected fallback. | X.8.3, XIII.6.2, XIII.6.3 |

### XIII.7 Git LFS 감지와 권고

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIII.7.1 | `[x]` | `git lfs version`을 시스템 Git 환경에서 실행해 설치 상태와 버전을 읽습니다. | installed/not installed fixture. | IV.3.4 |
| XIII.7.2 | `[x]` | 에셋 크기·확장자 기반 LFS 권고 정책을 config에서 읽고 import 후 경고만 생성합니다. 자동 `.gitattributes` 수정은 하지 않습니다. | threshold boundary fixture. | XIII.5.7, XIII.7.1 |
| XIII.7.3 | `[x]` | 사용자 승인 API가 호출된 경우에만 `git lfs track` 또는 동등 시스템 Git 명령으로 `.gitattributes`를 수정합니다. 직접 파일 문법을 재구현하지 않습니다. | 승인 없음 변경 0, 승인 후 Git 결과 검증. | XIII.7.2 |
| XIII.7.4 | `[x]` | 기존 history migration은 지원하지 않고 요청 시 UnsupportedOperation을 반환합니다. | 이력 변경 명령 실행 0. | XIII.7.3 |
| XIII.7.5 | `[x]` | LFS pointer만 있고 object가 없는 asset을 missing LFS diagnostic으로 구분합니다. | 일반 missing file과 다른 오류 코드. | XIII.7.1, XIII.5.1 |

### XIII.8 복구 스냅샷 실행

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIII.8.1 | `[ ]` | dirty document revision이 바뀔 때 recovery snapshot write를 debounce해 OS app data에 기록합니다. project Git 폴더에는 기록하지 않습니다. | `git status` 변화 없음. | V.9.2, XIII.1.3 |
| XIII.8.2 | `[ ]` | recovery snapshot write는 autosave와 독립적이지만 같은 immutable memory snapshot을 공유합니다. | duplicate document serialization count≤1. | XIII.1.4, V.9.2 |
| XIII.8.3 | `[ ]` | 앱 시작 시 최신 snapshot probe 결과를 shell에 반환하고 사용자가 복구 또는 폐기를 선택하게 합니다. | 두 선택 fixture 통과. | V.9.3, XII.2.2, XII.2.3 |
| XIII.8.4 | `[ ]` | 복구 선택 시 새 in-memory WindowSession을 만들고 disk current를 즉시 덮어쓰지 않습니다. 다음 autosave/save에서 일반 working tree 변경으로 기록합니다. | open 직후 disk bytes 미변경. | XIII.8.3 |
| XIII.8.5 | `[ ]` | macOS state restoration과 앱 종료 통지를 연결해 창·탭 metadata 복원을 지원합니다. 실제 document state는 Stellar snapshot에서 읽습니다. | crash/relaunch automation 통과. | XIII.8.3, II.5.3 |
| XIII.8.6 | `[!]` | Windows Application Recovery/Restart callback을 연결해 준비된 snapshot flush를 시도합니다. callback에서 무거운 full serialization은 금지합니다. | forced crash integration test에서 snapshot 발견. | XIII.8.2, II.6.3 |
| XIII.8.7 | `[ ]` | 정상 종료 후 관련 recovery snapshot을 삭제합니다. 삭제 실패는 다음 시작에서 stale 판정으로 무시합니다. | normal close 후 prompt 없음. | V.9.4, XIII.8.3 |

**Phase XIII 완료 조건:** autosave, external change 감지, asset/font 재현성, LFS 경고, 비정상 종료 복구가 통합 테스트를 통과합니다.

---

## Phase XIV. 시스템 Git, Semantic Diff·Staging·Conflict

### XIV.1 Git 실행과 저장소 위치

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIV.1.1 | `[x]` | trusted project에서만 GitRunner를 활성화하고 untrusted 상태에서는 모든 Git command request를 `ProjectNotTrusted`로 거부합니다. | 악성 repo fixture에서 process spawn count 0. | IV.5.3, IV.3.4 |
| XIV.1.2 | `[x]` | `git rev-parse --show-toplevel`, `--git-dir`, `--git-common-dir`, `--is-inside-work-tree`를 실행해 worktree root와 gitfile 구조를 읽습니다. 경로를 직접 추측하지 않습니다. | 일반 repo와 linked worktree fixture 통과. | IV.3.4 |
| XIV.1.3 | `[x]` | Git capability probe에 porcelain v2, pathspec-from-file, update-index --index-info, worktree, LFS availability를 기록합니다. 초기 UI가 쓰지 않는 기능도 capability로만 저장합니다. | 서로 다른 Git fixture output parse. | IV.3.2 |
| XIV.1.4 | `[x]` | Git command cancellation을 구현합니다. child process와 자식 프로세스 tree를 플랫폼 안전 방식으로 종료하고 partial app state를 성공으로 처리하지 않습니다. | long-running fixture cancel 후 process 잔존 0. | IV.3.3 |
| XIV.1.5 | `[x]` | stdout/stderr decoding은 UTF-8 우선, raw bytes 보존, invalid sequence replacement 표시를 구분합니다. machine parse는 `-z` 또는 stable format만 사용합니다. | non-UTF8 filename fixture 처리. | IV.3.4 |
| XIV.1.6 | `[x]` | Git command trace에는 command kind, duration, exit code만 기록하고 URL, branch, path, commit hash는 telemetry에서 제외합니다. 로컬 debug log도 redaction 규칙을 적용합니다. | secret fixture redaction 통과. | II.3.2, XIV.1.5 |

### XIV.2 Git 상태와 SnapshotProvider

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIV.2.1 | `[x]` | `git status --porcelain=v2 -z --branch` parser를 구현합니다. ordinary, rename/copy, unmerged, untracked, ignored를 구분합니다. | fixture corpus 전부 parse. | XIV.1.5 |
| XIV.2.2 | `[x]` | `RepositoryStatus`에 HEAD, upstream 상태, staged/unstaged/untracked counts, conflict state를 정의합니다. | detached HEAD·unborn branch fixture 통과. | XIV.2.1 |
| XIV.2.3 | `[x]` | `SnapshotSource`를 Head(commit), Index, WorkingTree, Commit(hash), Tree(hash)로 정의합니다. | type round-trip. | XIV.2.2 |
| XIV.2.4 | `[x]` | HEAD/commit tree의 파일 목록과 mode/blob ID를 `git ls-tree -rz`로 읽는 provider를 구현합니다. | symlink·executable·unicode path fixture 통과. | XIV.2.3 |
| XIV.2.5 | `[x]` | index의 stage 0 파일 목록과 blob ID를 `git ls-files --stage -z`로 읽는 provider를 구현합니다. unmerged entry는 별도 conflict provider로 넘깁니다. | partial staged fixture 정확. | XIV.2.3 |
| XIV.2.6 | `[x]` | blob content를 `git cat-file --batch`로 읽고 temp virtual project filesystem에 materialize합니다. `.git`은 생성하지 않습니다. | binary chunk·symlink fixture bytes 동일. | XIV.2.4, XIV.2.5 |
| XIV.2.7 | `[x]` | WorkingTree provider는 disk project loader를 사용하고 저장 대기 memory state와 구분합니다. Git status는 저장된 file 기준임을 명시합니다. | unsaved memory edit가 Git status에 나타나지 않는 fixture. | V.4.4, XIV.2.3 |
| XIV.2.8 | `[ ]` | Git UI를 열기 전 pending autosave를 flush할지 호출자에게 상태를 반환합니다. 자동으로 사용자의 memory edit를 저장하지 않습니다. | save/continue-with-disk 선택 fixture 통과. | XIII.1.5, XIV.2.7 |

### XIV.3 Semantic Snapshot과 Diff 모델

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIV.3.1 | `[x]` | `SemanticSnapshot`을 프로젝트 metadata, documents, nodes, properties, tokens, components, assets refs의 StableId index로 정의합니다. runtime cache는 제외합니다. | HEAD/Index/WT snapshot build fixture 통과. | VI.10.4, XIV.2.6 |
| XIV.3.2 | `[x]` | `SemanticChange` kind에 AddObject, RemoveObject, MoveObject, ReorderObject, SetProperty, Add/Remove/ChangeToken, ComponentInterfaceChange, AssetRefChange를 정의합니다. | kind golden test. | XIV.3.1 |
| XIV.3.3 | `[x]` | object match는 StableId로만 수행하고 이름·geometry 유사도로 rename 추정하지 않습니다. | 같은 이름 다른 ID는 add/remove. | XIV.3.1 |
| XIV.3.4 | `[x]` | property diff는 `StableId + PropertyPath`와 canonical TypedValue를 비교합니다. float epsilon diff를 사용하지 않습니다. | bit-different finite float fixture change 탐지. | III.2.2, XIV.3.2 |
| XIV.3.5 | `[x]` | move/reorder는 parent ID와 OrderKey의 의미 순서를 비교해 분류합니다. 저장 chunk 이동은 semantic change로 표시하지 않습니다. | rechunk-only fixture semantic diff 0. | III.5.2, V.6.5 |
| XIV.3.6 | `[x]` | component definition과 instance override diff를 별도 provenance와 함께 생성합니다. | definition change와 instance override 구분. | IX.1, IX.2 |
| XIV.3.7 | `[x]` | variant·binding·expression AST는 StableId node 단위로 diff하고 전체 blob 변경으로 축약하지 않습니다. | AST 한 operator 변경 fixture 1 change. | VIII.2.1, IX.7.3 |
| XIV.3.8 | `[x]` | semantic changes를 deterministic sort key로 정렬합니다: document, object path, property path, change kind. | 입력 map 순서가 달라도 diff bytes 동일. | XIV.3.2 |
| XIV.3.9 | `[x]` | diff apply 함수를 구현해 base snapshot에 change list를 적용하고 target snapshot을 재현합니다. | `apply(diff(A,B), A)==B` property test. | XIV.3.8 |
| XIV.3.10 | `[x]` | diff inverse를 구현합니다. | apply inverse 후 A 복원. | XIV.3.9 |

### XIV.4 Status 분류

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIV.4.1 | `[x]` | HEAD→Index diff를 Staged, Index→WorkingTree diff를 Unstaged로 계산합니다. | 부분 staged fixture 분류 정확. | XIV.3.10, XIV.2.5, XIV.2.7 |
| XIV.4.2 | `[x]` | 같은 PropertyPath가 staged와 unstaged에서 연속 수정된 경우 base/head, staged, working 세 값을 모두 보존합니다. | H=1, I=2, W=3 fixture 표시 정확. | XIV.4.1 |
| XIV.4.3 | `[x]` | untracked Stellar files는 WorkingTree snapshot에 포함하되 index snapshot에는 없는 AddObject/File change로 표시합니다. | untracked chunk/asset fixture 통과. | XIV.4.1 |
| XIV.4.4 | `[x]` | non-Stellar files는 raw Git file status로만 표시하고 semantic parser로 해석하지 않습니다. | README change raw status 유지. | XIV.2.1 |
| XIV.4.5 | `[x]` | corrupt WorkingTree Stellar state는 semantic status를 실패시키되 raw Git status는 계속 제공하고 자동 수정하지 않습니다. | corrupt chunk fixture raw status 가능. | XIV.4.1, V.4.4 |

### XIV.5 선택 변경의 Staging 계획

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIV.5.1 | `[x]` | 선택한 SemanticChange ID 목록을 받는 `StageSelectionRequest`와 결과 dependency closure를 정의합니다. | round-trip. | XIV.4.1 |
| XIV.5.2 | `[x]` | change dependency graph를 작성합니다. 새 object의 property, binding이 참조하는 token/property, component interface와 instance override 관계를 포함합니다. | dependency fixture 정확. | XIV.3.2, VI.2.5, VIII.6.2 |
| XIV.5.3 | `[x]` | 선택 변경만으로 snapshot validator가 실패하면 필수 dependency를 `required_additions`로 반환하고 자동 stage하지 않습니다. | invalid partial fixture expected dependencies. | XIV.5.2, VI.10.1 |
| XIV.5.4 | `[x]` | caller가 dependencies 포함을 승인한 뒤 Index snapshot에 selected+required changes를 적용해 CandidateIndexSnapshot을 만듭니다. | candidate가 document validator 통과. | XIV.5.3, XIV.3.9 |
| XIV.5.5 | `[x]` | CandidateIndexSnapshot을 deterministic project files/chunks로 serialize하되 WorkingTree를 수정하지 않는 temp directory writer를 구현합니다. | write 전후 working tree file hash 동일. | XIV.5.4, VI.10.2, V.5.1 |
| XIV.5.6 | `[x]` | candidate와 기존 index tree의 file-level diff를 계산해 update할 path와 blob bytes만 산출합니다. | unchanged path update 0. | XIV.5.5, XIV.2.5 |

### XIV.6 Git index 기록

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIV.6.1 | `[ ]` | temp candidate file bytes를 `git hash-object -w --stdin`으로 blob object화하고 반환 OID를 검증합니다. | `git cat-file blob` bytes 동일. | XIV.5.6, XIV.1.3 |
| XIV.6.2 | `[ ]` | file mode와 blob OID를 `git update-index --cacheinfo` 또는 `--index-info`로 stage합니다. index 파일을 직접 수정하지 않습니다. | partial same-file stage fixture 통과. | XIV.6.1 |
| XIV.6.3 | `[ ]` | 삭제 path는 update-index remove semantics를 사용하고 symlink/executable mode를 보존합니다. | mode/delete fixture 통과. | XIV.6.2 |
| XIV.6.4 | `[ ]` | 여러 path index update를 하나의 operation으로 묶고 중간 실패 시 기존 index tree를 복원합니다. 시작 전 `git write-tree` 또는 안전한 index snapshot OID를 기록합니다. | fault injection 후 original index 동일. | XIV.6.2 |
| XIV.6.5 | `[ ]` | stage 후 Index provider를 다시 읽고 CandidateIndexSnapshot과 semantic snapshot이 같은지 검증합니다. | mismatch 시 operation 실패와 index 복원. | XIV.6.4, XIV.2.5 |
| XIV.6.6 | `[ ]` | stage operation이 WorkingTree bytes와 in-memory document를 바꾸지 않는 통합 테스트를 추가합니다. | 전후 hash/revision 동일. | XIV.6.5 |

### XIV.7 선택 Unstage

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIV.7.1 | `[x]` | 선택 staged changes를 Index snapshot에서 inverse 적용해 CandidateIndexSnapshot을 계산합니다. 기준은 HEAD가 아니라 현재 Index입니다. | staged A+B에서 A만 제거 fixture 통과. | XIV.4.1, XIV.3.10 |
| XIV.7.2 | `[x]` | unstage dependency graph를 검사하고 남은 index snapshot이 invalid하면 필수 함께-unstage 변경을 반환합니다. | component property/override fixture 통과. | XIV.5.2, XIV.7.1 |
| XIV.7.3 | `[ ]` | 승인된 candidate를 Phase XIV.6 index 기록 경로로 적용합니다. | WorkingTree 변경 없음, index semantic 결과 정확. | XIV.7.2, XIV.6.6 |

### XIV.8 Commit

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIV.8.1 | `[ ]` | staged snapshot이 valid Stellar project인지 commit 전에 재검증합니다. invalid하면 Git commit을 실행하지 않습니다. | invalid index fixture process spawn 0. | XIV.6.5, VI.10.1 |
| XIV.8.2 | `[x]` | commit message를 temp file 또는 stdin으로 시스템 Git에 전달하고 shell escaping에 의존하지 않습니다. | multiline/unicode message fixture 통과. | IV.3.3 |
| XIV.8.3 | `[x]` | user.name/email 미설정 오류를 구조화해 UI가 안내할 수 있게 합니다. Stellar가 전역 config를 자동 수정하지 않습니다. | missing identity fixture 통과. | IV.4.5 |
| XIV.8.4 | `[ ]` | commit 성공 후 HEAD, index, status를 다시 조회하고 자동 push하지 않습니다. | commit hash 변경, staged diff 0. | XIV.8.1~XIV.8.3 |
| XIV.8.5 | `[ ]` | commit hook/filter가 실행될 수 있으므로 trusted project에서만 commit을 허용하고 Git 결과를 그대로 반환합니다. hook 실패를 우회하지 않습니다. | failing hook fixture commit 없음. | XIV.1.1, XIV.8.4 |

### XIV.9 Remote 연결과 Push

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIV.9.1 | `[x]` | existing remote 목록을 `git remote -v`의 안정 parse 방식으로 읽고 fetch/push URL을 구분합니다. URL은 민감값으로 처리합니다. | multiple remote fixture 통과. | XIV.1.5 |
| XIV.9.2 | `[x]` | GitHub URL 형식 검증은 HTTPS/SSH canonical patterns만 확인하고 API 호출은 하지 않습니다. | github/non-github fixture 분류. | XIV.9.1 |
| XIV.9.3 | `[x]` | 새 GitHub remote 연결 전에 `git ls-remote`로 refs 존재 여부를 확인합니다. refs가 하나라도 있으면 최초 업로드 연결을 거부하고 Clone 안내 상태를 반환합니다. | empty/non-empty bare repo fixture 통과. | XIV.9.2 |
| XIV.9.4 | `[x]` | empty remote만 `git remote add <name> <url>`로 등록합니다. 기본 remote name은 `origin`, 이미 있으면 호출자가 다른 이름을 제공해야 합니다. | origin exists fixture overwrite 없음. | XIV.9.3 |
| XIV.9.5 | `[x]` | Push는 사용자가 선택한 existing remote/refspec을 시스템 Git으로 실행합니다. force push UI는 제공하지 않습니다. | normal push fixture 통과, force option API 없음. | XIV.9.1 |
| XIV.9.6 | `[x]` | credential helper/SSH agent prompt를 Git에 위임하고 Stellar는 token/key를 저장하지 않습니다. askpass bridge가 필요하면 입력 값은 메모리에서 즉시 폐기합니다. | credential log redaction·storage scan 통과. | XIV.9.5, II.3.2 |
| XIV.9.7 | `[x]` | push 성공 후 remote tracking status를 갱신합니다. 실패 시 local commit/index를 변경하지 않습니다. | rejected push fixture local state 동일. | XIV.9.5 |

### XIV.10 3-way Semantic Merge 엔진

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIV.10.1 | `[x]` | `MergeInput { base, ours, theirs }`, `MergeResult`, `SemanticConflict` 모델을 정의합니다. | round-trip. | XIV.3.1 |
| XIV.10.2 | `[x]` | 서로 다른 StableId 변경과 같은 object의 서로 다른 PropertyPath 변경을 자동 병합합니다. | non-overlap fixture conflict 0. | XIV.10.1, XIV.3.9 |
| XIV.10.3 | `[x]` | 같은 PropertyPath를 양쪽이 같은 값으로 바꾼 경우 conflict 없이 한 변경으로 합칩니다. | identical edit fixture 통과. | XIV.10.2 |
| XIV.10.4 | `[x]` | same property different value conflict를 생성하고 base/ours/theirs 값을 모두 보존합니다. | conflict record 정확. | XIV.10.2 |
| XIV.10.5 | `[x]` | delete-vs-modify, delete-vs-move, type-change-vs-value-change conflict를 각각 다른 kind로 분류합니다. | conflict matrix fixture 통과. | XIV.10.1 |
| XIV.10.6 | `[x]` | OrderKey independent insertion을 자동 병합하고 같은 child conflicting reorder를 conflict로 분류합니다. | Phase III fixture 통과. | III.5.4, XIV.10.2 |
| XIV.10.7 | `[ ]` | component property 삭제 vs instance override 추가, token type 변경 vs value 변경, drilling chain break를 전용 conflict로 분류합니다. | fixture corpus 통과. | IX.1.3, IX.4.6, VIII.5.5 |
| XIV.10.8 | `[ ]` | conflict 없는 merged snapshot을 full validator로 검증합니다. invalid result는 자동 merge 성공으로 반환하지 않습니다. | invalid cross-ref fixture conflict/error. | XIV.10.2~XIV.10.7, VI.10.1 |
| XIV.10.9 | `[ ]` | merge 결과와 conflict 순서를 deterministic sort합니다. | input map 순서 변경에도 동일 bytes. | XIV.10.8 |

### XIV.11 Git unmerged index materialization

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIV.11.1 | `[x]` | `git ls-files -u -z`를 읽어 stage 1(base), 2(ours), 3(theirs) blob과 mode를 path별로 수집합니다. | text/binary/rename conflict fixture parse. | XIV.2.5 |
| XIV.11.2 | `[x]` | conflict 관련 Stellar project 파일의 세 stage를 각각 virtual project snapshot으로 materialize합니다. conflict 없는 path는 current index/working 상태와 결합합니다. | base/ours/theirs project open. | XIV.2.6, XIV.11.1 |
| XIV.11.3 | `[ ]` | 세 snapshot을 Semantic Merge engine에 전달하고 raw file conflict와 semantic conflict mapping을 유지합니다. | conflict path→object mapping fixture 정확. | XIV.10.9, XIV.11.2 |
| XIV.11.4 | `[x]` | Stellar 파일이 아닌 conflict는 raw Git conflict로 표시하고 Semantic Merge로 해석하지 않습니다. | README conflict raw 유지. | XIV.11.1 |

### XIV.12 Conflict Resolution 모델

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIV.12.1 | `[ ]` | conflict별 resolution을 ChooseBase, ChooseOurs, ChooseTheirs, CustomValue, Delete, KeepModified로 정의하고 허용 action은 conflict kind별로 제한합니다. | invalid action 거부. | XIV.10.4~XIV.10.7 |
| XIV.12.2 | `[ ]` | resolution apply는 candidate merged snapshot에만 적용하고 WorkingTree/Index를 즉시 수정하지 않습니다. | preview 중 disk/index hash 동일. | XIV.12.1 |
| XIV.12.3 | `[ ]` | 모든 semantic conflict 해결 후 candidate snapshot full validation을 실행합니다. | unresolved count 0 + validator pass 필요. | XIV.12.2, VI.10.1 |
| XIV.12.4 | `[ ]` | validated snapshot을 WorkingTree project files로 generation save하고 관련 path를 시스템 Git `git add`로 resolved 처리합니다. | unmerged entries 제거, project open 성공. | XIV.12.3, V.5.4 |
| XIV.12.5 | `[ ]` | merge/rebase/cherry-pick 진행 상태를 자동 continue하거나 commit하지 않습니다. UI에는 Git 상태만 반환합니다. | continue command spawn 0. | XIV.12.4 |
| XIV.12.6 | `[ ]` | conflict resolution apply 전체에 fault rollback을 구현합니다. WorkingTree write 성공 후 git add 실패 시 unresolved 상태와 복구 안내를 반환하고 자동 commit하지 않습니다. | fault matrix 통과. | XIV.12.4 |

### XIV.13 최소 Git UI bridge

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIV.13.1 | `[ ]` | FFI에 repository status, staged/unstaged semantic change 목록을 page 단위로 조회하는 API를 추가합니다. | 100만 change synthetic pagination 통과. | XIV.4.5, II.4.5 |
| XIV.13.2 | `[ ]` | FFI에 stage/unstage selected change IDs API를 추가합니다. operation progress와 결과를 event queue로 반환합니다. | cancel/failure/success fixture 통과. | XIV.6.6, XIV.7.3 |
| XIV.13.3 | `[ ]` | FFI에 commit, existing remote connect, push API를 추가합니다. raw credentials를 FFI 구조체에 저장하지 않습니다. | smoke fixture 통과. | XIV.8.5, XIV.9.7 |
| XIV.13.4 | `[ ]` | FFI에 semantic conflicts, base/ours/theirs values, resolution apply API를 추가합니다. | conflict corpus bridge round-trip. | XIV.12.6 |
| XIV.13.5 | `[ ]` | macOS 최소 Git panel은 Status/Diff, Stage/Unstage, Commit, Push, Conflict resolution 기능만 core API에 연결합니다. | UI automation smoke. | XIV.13.1~XIV.13.4 |
| XIV.13.6 | `[!]` | Windows 최소 Git panel을 동일 기능으로 연결합니다. | UI automation smoke. | XIV.13.1~XIV.13.4 |
| XIV.13.7 | `[ ]` | Branch, Pull, Fetch, Merge, Rebase, Stash, Worktree, Remote 관리 전용 UI가 초기 빌드에 노출되지 않는 feature inventory test를 추가합니다. 시스템 Git 자체 사용은 차단하지 않습니다. | 초기 메뉴/command registry에 미포함. | XIV.13.5, XIV.13.6 |

### XIV.14 Git 통합 corpus

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIV.14.1 | `[~]` | 임시 local bare repo와 working repo를 만드는 Git fixture harness를 작성합니다. 외부 네트워크를 사용하지 않습니다. | macOS/Windows 동일 scenario 생성. | XIV.1.2 |
| XIV.14.2 | `[ ]` | partial same-chunk property staging, unstage, commit scenario를 fixture로 만듭니다. | HEAD/Index/WT expected values 일치. | XIV.6.6, XIV.7.3, XIV.8.4 |
| XIV.14.3 | `[ ]` | component/token dependency staging scenario를 만듭니다. | invalid partial 차단, dependency 승인 후 성공. | XIV.5.4 |
| XIV.14.4 | `[ ]` | branch merge conflict를 시스템 Git으로 만든 뒤 Semantic Conflict 해결·git add까지 통합 실행합니다. | unmerged entries 0, merge state 유지. | XIV.12.5 |
| XIV.14.5 | `[ ]` | linked worktree에서 open/status/stage/commit을 실행해 common git dir과 per-worktree index가 올바른지 검증합니다. | 다른 worktree index 변경 없음. | XIV.1.2, XIV.6.6 |
| XIV.14.6 | `[x]` | non-UTF8 path, symlink, executable, LFS pointer, corrupt project scenario를 추가합니다. | 각 정책 결과 정확. | XIV.1.5, XIII.4, XIII.7.5 |

**Phase XIV 완료 조건:** G7 통과.

---

## Phase XV. Git 기반 외부 Component·Token Library

### XV.1 의존성 선언 모델

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XV.1.1 | `[x]` | `stellar.yaml` dependency entry에 logical name, library id, source kind Git, repository URL, version selector(tag/commit/branch), optional local override를 정의합니다. | schema round-trip. | IV.1.1, Phase XIV |
| XV.1.2 | `[x]` | branch selector는 개발 편의 선언으로 허용하되 lockfile에는 항상 exact commit을 기록하게 합니다. | branch resolve 후 lock exact hash. | XV.1.1 |
| XV.1.3 | `[x]` | 같은 logical name 중복, 같은 LibraryId 다른 selector, 다른 LibraryId 같은 URL을 구분해 validation합니다. | conflict matrix fixture 통과. | XV.1.1 |
| XV.1.4 | `[x]` | dependency URL과 local override path를 telemetry 민감값으로 분류합니다. | diagnostic/telemetry redaction 통과. | II.3.2, XV.1.1 |

### XV.2 Resolver와 전이 그래프

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XV.2.1 | `[x]` | `DependencyNodeKey`를 LibraryId+ResolvedCommit으로 정의해 동일 library 여러 버전을 별도 node로 허용합니다. | v1/v2 동시 key fixture 통과. | XV.1.1 |
| XV.2.2 | `[x]` | dependency graph edge에 parent dependency path를 기록합니다. 같은 resolved node는 그래프에서 dedup합니다. | diamond dependency fixture 정확. | XV.2.1 |
| XV.2.3 | `[x]` | transitive cycle을 탐지하고 dependency path 전체를 오류로 반환합니다. | A→B→A fixture 실패. | XV.2.2 |
| XV.2.4 | `[x]` | version selector resolution은 시스템 Git의 `ls-remote`, fetch된 refs, commit verification을 사용합니다. Git 객체를 직접 파싱하지 않습니다. | tag/branch/commit fixture resolve. | XIV.1.3, XV.1.2 |
| XV.2.5 | `[x]` | 같은 LibraryId의 여러 resolved commit이 그래프에 존재하면 오류가 아니라 `DuplicateLibraryVersionWarning`을 생성합니다. | v1/v2 fixture warning 1건. | XV.2.1 |
| XV.2.6 | `[x]` | resolver 결과를 LibraryId, commit, dependency path로 deterministic sort합니다. | 선언 순서 변경에도 lock output 동일. | XV.2.2 |

### XV.3 공용 콘텐츠 캐시

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XV.3.1 | `[x]` | 사용자별 OS app data에 library cache root와 cache index를 정의합니다. project folder 안에는 의존성 checkout을 만들지 않습니다. | Git status 변화 없음. | V.1.5, XV.2.1 |
| XV.3.2 | `[x]` | repository URL별 bare mirror 또는 동등 system Git cache를 생성하고 fetch를 GitRunner로 수행합니다. | 같은 repo 두 dependency fetch 중복 네트워크 1회. | XIV.1.3, XV.3.1 |
| XV.3.3 | `[x]` | exact commit의 Stellar project files를 content-addressed immutable cache entry로 materialize합니다. | 같은 commit 재요청 bytes 재사용. | XV.3.2, XIV.2.6 |
| XV.3.4 | `[~]` | cache entry를 ProjectProbe와 storage validator로 검증하고 일반 Git repo 또는 손상 library를 거부합니다. | invalid library fixture 실패. | IV.5.1, V.4.4 |
| XV.3.5 | `[x]` | cache index에 repository URL 원문 대신 keyed identifier와 commit/content hash만 저장하는 privacy mode를 구현합니다. 원본 URL은 local secure config에만 둡니다. | exported diagnostic에 URL 없음. | XV.1.4, XV.3.1 |
| XV.3.6 | `[x]` | cache read lock과 fetch/write lock을 분리해 여러 Stellar project가 같은 entry를 동시에 읽을 수 있게 합니다. | two-process concurrency fixture 통과. | XV.3.3 |

### XV.4 Lockfile 생성

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XV.4.1 | `[x]` | resolver graph를 `stellar.lock.yaml`의 resolved entries와 dependency paths로 변환합니다. | canonical lock golden. | IV.1.6, XV.2.6 |
| XV.4.2 | `[~]` | 각 entry에 exact commit, cached content hash, library schema version, transitive dependencies를 기록합니다. | 누락 field validator 실패. | XV.3.4, XV.4.1 |
| XV.4.3 | `[!]` | lockfile가 존재하면 선언 selector가 바뀌지 않은 dependency는 network 없이 lock commit을 우선 사용합니다. | offline cache fixture 통과. | XV.4.2, XV.3.3 |
| XV.4.4 | `[~]` | lockfile content hash와 cache entry가 다르면 corruption으로 처리하고 자동으로 다른 commit을 선택하지 않습니다. | tampered cache fixture 실패. | XV.4.2 |
| XV.4.5 | `[x]` | resolve 성공 후 lockfile을 canonical write하고 autosave와 별도 project file transaction으로 기록합니다. 자동 commit하지 않습니다. | Git status에 lockfile만 예상대로 변경. | XV.4.1, IV.1.7 |

### XV.5 읽기 전용 Library 로드

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XV.5.1 | `[x]` | library project에서 export된 ComponentDefinition·TokenSet index를 읽는 `LibrarySnapshot`을 정의합니다. 모든 resource key에 LibraryId+commit+StableId를 포함합니다. | v1/v2 same StableId 충돌 없음. | XV.2.1, IX.1.1, VIII.5.1 |
| XV.5.2 | `[x]` | 사용 project document가 library resource를 참조할 수 있게 `ExternalComponentRef`, `ExternalTokenRef`를 구현합니다. | round-trip. | XV.5.1 |
| XV.5.3 | `[~]` | library resource를 수정하는 command를 사용 project에서 거부합니다. instance 공개 override만 허용합니다. | edit definition fixture 실패, override 성공. | XV.5.2, IX.2.4 |
| XV.5.4 | `[x]` | library의 transitive resource ref를 동일 dependency graph instance 안에서 resolve합니다. | v1 parent가 v1 child, v2 parent가 v2 child 참조. | XV.2.2, XV.5.1 |
| XV.5.5 | `[~]` | missing cache/offline entry는 library resource unavailable diagnostic을 생성하고 project 기본 데이터는 읽을 수 있게 합니다. | offline missing fixture partial open. | XV.3.3, III.2.4 |

### XV.6 로컬 경로·Worktree Override

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XV.6.1 | `[x]` | `stellar.yaml` local override는 특정 dependency logical name을 local Stellar project root로 매핑합니다. | schema validation. | XV.1.1 |
| XV.6.2 | `[~]` | local path가 일반 repo 또는 잘못된 library id이면 거부합니다. | fixture 실패. | XV.6.1, IV.5.1 |
| XV.6.3 | `[!]` | local override의 current commit과 working tree dirty 상태를 진단에 표시하되 lockfile exact remote commit은 유지합니다. | dirty override warning. | XV.6.2, XIV.2.2 |
| XV.6.4 | `[!]` | linked worktree path도 ProjectProbe로 허용합니다. | worktree library fixture load. | XIV.1.2, XV.6.2 |
| XV.6.5 | `[!]` | local override는 개발 세션에서만 사용하고 release validation에서 존재하면 경고 또는 정책 오류를 config로 제어합니다. | release gate fixture 통과. | XV.6.3 |

### XV.7 Library 업데이트

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XV.7.1 | `[x]` | dependency update request가 selector 재해석 후 CurrentLibrarySnapshot과 CandidateLibrarySnapshot을 생성합니다. | current/candidate commit 정확. | XV.2.4, XV.5.1 |
| XV.7.2 | `[x]` | library resource Semantic Diff를 ComponentDefinition·Property·Token 단위로 계산합니다. | resource 변경 fixture 정확. | XIV.3.7, XV.5.1 |
| XV.7.3 | `[ ]` | candidate library를 적용하기 전 사용 project의 external refs와 instance overrides를 검증합니다. | removed property override가 orphan diagnostic. | IX.4.6, XV.5.2 |
| XV.7.4 | `[!]` | 사용자 승인 후 lockfile과 in-memory library snapshot을 원자적으로 갱신합니다. 사용 project document override는 StableId로 재적용합니다. | update 후 render/semantic snapshot expected. | XV.7.3, XV.4.5 |
| XV.7.5 | `[!]` | update 실패 시 이전 lock/cache snapshot을 유지합니다. | network/cache/schema fault fixture 통과. | XV.7.4 |

### XV.8 캐시 정리와 진단

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XV.8.1 | `[!]` | 최근 사용 시각과 lockfile reference set을 기준으로 cache GC 후보를 계산합니다. 삭제 임계치는 config에 둡니다. | active lock entry 후보 제외. | XV.3.6, XV.4.2 |
| XV.8.2 | `[!]` | 여러 project가 참조하는 cache entry는 마지막 참조가 사라질 때까지 삭제하지 않습니다. | two-project fixture 통과. | XV.8.1 |
| XV.8.3 | `[x]` | duplicate library version warning에 dependency paths와 예상 추가 용량을 포함합니다. repository URL은 표시 UI에만 local source로 전달하고 telemetry에는 제외합니다. | warning fixture 정확. | XV.2.5, XV.3.5 |
| XV.8.4 | `[!]` | 외부 library 전체 integration fixture를 생성합니다: A→C v1, B→C v2, local override, offline cache, update. | semantic snapshot·lockfile golden 통과. | XV.1~XV.7 |

**Phase XV 완료 조건:** 외부 library exact version 재현, 다중 버전 격리, 읽기 전용 정책, update 검토가 통과합니다.

---

## Phase XVI. 성능 게이트, 보안, 진단, 패키징과 초기 릴리스

### XVI.1 T0~T5 성능 계측

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XVI.1.1 | `[ ]` | `InteractionTraceId`를 pointer/key event마다 생성하고 T0 입력 수신 시각을 기록합니다. | 한 event당 trace ID 하나. | XII.3.5 |
| XVI.1.2 | `[ ]` | T1 hit test 완료, T2 command commit, T3 binding/layout 완료, T4 scene/render graph 완료, T5 submit/present 예약 시각을 같은 trace에 기록합니다. | 누락 stage를 오류로 표시하는 trace validator 통과. | VII.1.4, X.7.3, XI.2.6, XI.8.5, XI.9.5 |
| XVI.1.3 | `[ ]` | stage별 duration과 총 Logical Interaction Latency를 계산합니다. clock domain이 다른 GPU timestamp는 CPU T0~T5와 직접 합산하지 않습니다. | ManualClock reference fixture 통과. | XVI.1.2, II.3.3 |
| XVI.1.4 | `[ ]` | interaction kind, document size bucket, visible node bucket, cache state만 trace dimension으로 허용하고 object name/text/path를 금지합니다. | privacy schema test 통과. | II.3.2, XVI.1.3 |
| XVI.1.5 | `[ ]` | ring buffer 기반 로컬 trace store를 구현하고 용량 초과 시 가장 오래된 trace부터 삭제합니다. | memory bound 유지. | XVI.1.4 |

### XVI.2 결정적 입력 재생 Harness

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XVI.2.1 | `[ ]` | 입력 재생 파일에 초기 project fixture hash, viewport, InputEvent sequence, ManualClock timeline을 정의합니다. | round-trip. | XII.3.1, XVI.1.1 |
| XVI.2.2 | `[ ]` | core-only replay runner를 구현해 UI 없이 command/layout/scene 단계까지 재생합니다. | 동일 replay 두 번 semantic/trace event count 동일. | XVI.2.1 |
| XVI.2.3 | `[ ]` | macOS full replay runner를 구현해 native input injection 또는 app test bridge로 T5까지 측정합니다. | 결과 JSON 생성. | XVI.2.2, XII.4.1 |
| XVI.2.4 | `[!]` | Windows full replay runner를 구현합니다. | 결과 JSON 생성. | XVI.2.2, XII.4.2 |
| XVI.2.5 | `[ ]` | Pan, Zoom, Drag, Resize, Rotate, Hover, Select, Text Edit, Token Propagation, Component Update, Variant Change, Undo, Redo replay를 각각 만듭니다. | 모든 scenario 최소 1,000 sample 생성. | XVI.2.3, XVI.2.4 |
| XVI.2.6 | `[ ]` | warm cache와 cold cache setup을 분리하고 결과 metadata에 명시합니다. | cache setup이 실제 hit/miss count와 일치. | XI.6.5, XI.7.3, XVI.2.5 |

### XVI.3 기준 프로젝트 Fixture

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XVI.3.1 | `[x]` | 1만, 20만, 프로젝트 전체 100만 node synthetic fixture generator를 구현합니다. 파일을 직접 Git에 대량 저장하지 않고 seed+generator version으로 재현합니다. | 같은 seed content hash 동일. | VI.10.5, V.6.4 |
| XVI.3.2 | `[x]` | 2만 visible node, 5만 component instance, 중첩 64 fixture를 각각 생성합니다. | validator pass. | IX.2.2, VI.10.1 |
| XVI.3.3 | `[~]` | text-heavy, image-heavy, effect-heavy, token-heavy, constraint-heavy fixture를 분리합니다. | 각 fixture가 목표 병목 특성을 계측값으로 확인. | Phase X, Phase XI |
| XVI.3.4 | `[x]` | project asset 20GB는 CI에 실제 포함하지 않고 sparse synthetic asset provider와 별도 reference device test data manifest로 구성합니다. | provider가 size/stream behavior 재현. | XIII.5.2 |

### XVI.4 성능 판정기

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XVI.4.1 | `[ ]` | `config/performance-budgets.yaml`에 T0~T5 단계 목표·상한, p50/p95/p99, GPU p99, queue depth를 기록합니다. | 문서 6장 수치와 자동 비교해 불일치 실패. | XVI.1.3 |
| XVI.4.2 | `[ ]` | sample에서 p50/p95/p99와 통제 가능 10ms 초과 count를 계산하는 analyzer를 구현합니다. | 통계 reference fixture 통과. | XVI.4.1 |
| XVI.4.3 | `[ ]` | OS preemption, device lost, sleep transition 등 excluded sample은 사전 정의 오류 코드가 있을 때만 제외합니다. 임의 outlier 제거는 금지합니다. | arbitrary high sample이 제외되지 않습니다. | XVI.4.2, XI.5.4 |
| XVI.4.4 | `[ ]` | release gate는 모든 required scenario가 p50≤5, p95≤7, p99≤9, controllable max≤10ms를 통과해야 성공하게 합니다. | threshold boundary fixture 통과. | XVI.4.2 |
| XVI.4.5 | `[ ]` | 결과를 commit, hardware profile, OS build, app build와 함께 보존하되 사용자 project 정보는 포함하지 않습니다. | result schema privacy 검사. | XVI.4.4 |
| XVI.4.6 | `[ ]` | 기준 하드웨어 전용 CI 또는 lab runner에서 macOS M1 16GB와 Windows 기준 기기 결과를 required status로 설정합니다. | 해당 status 없이는 release tag 차단. | XVI.2.5, XVI.4.5 |
| XVI.4.7 | `[ ]` | command 검증이 호출마다 다시 만드는 문서 전체 index(`crates/commands/src/tree.rs`의 `id_owners`·`ChildIndex`·`reference_index`, 각 O(N))를 `DocumentModel`이 편집과 함께 증분 유지하는 id 종류·자식·inbound 참조 index로 바꿉니다. RestoreNodeRecords(ConvertShapeToPath·ConvertFragmentToFrame의 undo·redo, VII.2.7)부터 옮기고, 같은 index를 쓰는 다른 command 검증도 옮깁니다. 검사 규칙과 오류 값은 바꾸지 않습니다. | release에서 node 1,000·10,000·50,000 문서의 이름만 바꾸는 RestoreNodeRecords validate 측정값을 보고서에 기록하고 50,000 node 값이 1 ms 이하(아키텍처 6.3 단일 점유 2 ms, 6.4 Undo/Redo 10 ms 안), random command sequence 뒤 증분 index가 전체 재구성 index와 같은 property test 통과, 범위 검사와 전체 validator oracle을 비교하는 기존 property test 통과. | VII.2.7, VI.2.5 |
| XVI.4.8 | `[ ]` | binding 저장소 조회의 선형 검색을 index로 바꿉니다. `BindingStore`에 token → binding(live·orphan) 역참조 index를 두어 토큰 Delete의 inbound 검사(VIII.5.5 `TokenOrphanPlan::current`)가 모든 binding source를 훑지 않게 하고, orphan target index로 `StoreTypes::reference_type`(VIII.8.2)이 Property 참조마다 하는 orphan 선형 검색을 없앱니다. 판정 결과는 바꾸지 않습니다. | release에서 binding 1,000·10,000 fixture의 토큰 Delete validate와 orphan 1,000개가 있는 open 경로 측정값을 보고서에 기록하고 binding 10배에서 시간 2배 이하, random 편집 뒤 index 조회가 전체 순회 결과와 같은 property test 통과, 기존 VIII.5.5·VIII.8.2·VIII.8.3 test 통과. | VIII.5.5, VIII.8.2 |
| XVI.4.9 | `[ ]` | 토큰·binding·orphan command 검증이 호출마다 다시 만드는 토큰 dependency graph와 통합 binding graph(VIII.5.5·VIII.6.6·VIII.8.4)를 편집과 함께 증분 갱신하도록 바꿉니다. cycle·참조 판정은 바꾸지 않습니다. | release에서 토큰 1,000·binding 1,000·10,000 fixture의 command 하나 validate 측정값을 보고서에 기록하고 binding 10배에서 시간 2배 이하, random command sequence 뒤 증분 graph가 전체 재구성 graph와 같은 property test 통과, 기존 VIII.5.5·VIII.6.6·VIII.8.4 test 통과. | VIII.5.5, VIII.6.6, VIII.8.4 |

### XVI.5 메모리·캐시 예산

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XVI.5.1 | `[x]` | CPU/GPU memory budget provider interface를 정의하고 OS별 available memory 정보를 normalized bucket으로 반환합니다. | mock low/medium/high fixture. | Phase XI, Phase XII |
| XVI.5.2 | `[ ]` | cache priority를 Document/CurrentScene(비축출), VisibleGlyph/Geometry, NearViewport, BackgroundThumbnail 순으로 정의합니다. | eviction order fixture 통과. | XI.6.5, XI.7.3 |
| XVI.5.3 | `[ ]` | glyph, tessellation, image mipmap, thumbnail cache가 공통 budget manager에 usage를 등록하게 합니다. | total usage 일치. | XVI.5.1, XVI.5.2 |
| XVI.5.4 | `[ ]` | memory pressure event에서 낮은 priority부터 증분 eviction하고 단일 UI thread 구간 2ms를 넘지 않게 chunking합니다. | simulated pressure에서 latency/usage 기준 통과. | XVI.5.3 |
| XVI.5.5 | `[x]` | 비활성 document chunk unload와 재로드를 구현합니다. document index와 unsaved dirty chunk는 unload하지 않습니다. | inactive reload semantic snapshot 동일. | V.6.6, VI.1.4 |
| XVI.5.6 | `[ ]` | GPU OOM 직전 캐시 purge 후 재시도를 한 번만 수행하고 반복 loop를 금지합니다. | mock OOM retry count=1. | XI.5.4, XVI.5.4 |

### XVI.6 파서·저장·Command Fuzzing

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XVI.6.1 | `[x]` | restricted YAML parser fuzz target을 추가합니다. | panic, hang, excessive allocation corpus 0. | IV.1.4 |
| XVI.6.2 | `[x]` | chunk envelope/canonical decoder fuzz target을 추가합니다. | malformed input panic 0. | V.2.6 |
| XVI.6.3 | `[x]` | document loader/validator fuzz target을 추가합니다. | invalid refs/cycles 처리, panic 0. | VI.10.3 |
| XVI.6.4 | `[x]` | expression AST/typechecker/evaluator fuzz target을 추가합니다. | instruction budget 작동, panic 0. | VIII.4.5 |
| XVI.6.5 | `[x]` | Semantic Diff/apply/inverse fuzz target을 추가합니다. | valid A/B에서 apply(diff)==B. | XIV.3.10 |
| XVI.6.6 | `[ ]` | Semantic Merge fuzz target을 추가해 결과 valid 또는 명시 conflict/error만 허용합니다. | invalid silent merge 0. | XIV.10.8 |
| XVI.6.7 | `[~]` | command sequence fuzz target을 장기 nightly job으로 추가합니다. | crash seed 자동 artifact 저장. | VII.6.3 |

### XVI.7 프로젝트 신뢰와 외부 실행 보안

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XVI.7.1 | `[x]` | untrusted mode capability matrix를 코드로 정의합니다: 문서 제한 읽기 허용, Git/terminal/future MCP/external filter/external symlink read 금지. | capability table test 통과. | IV.5.3, XIII.4.5, XIV.1.1 |
| XVI.7.2 | `[x]` | trust grant/revoke API를 구현하고 OS user data store에 기록합니다. project file/Git config는 변경하지 않습니다. | grant/revoke/reopen fixture 통과. | IV.5.2 |
| XVI.7.3 | `[ ]` | Git hooks, filters, diff/merge driver는 시스템 Git이 실행하되 trusted mode에서만 Git command 자체를 허용합니다. Stellar가 hook 파일을 직접 실행하지 않습니다. | untrusted commit process spawn 0. | XIV.8.5, XVI.7.1 |
| XVI.7.4 | `[ ]` | file parser worker process 격리 interface를 정의하고 이미지·font parser를 선택적으로 worker에서 실행할 수 있게 합니다. 초기에는 feature flag로 활성화합니다. | parser crash가 app process를 종료하지 않는 integration test. | XI.7.1, XIII.6.1, XII.1.4 |
| XVI.7.5 | `[x]` | temp directory permissions와 cleanup을 강화하고 다른 사용자 접근을 금지합니다. | platform permission test 통과. | XIV.2.6, IV.6.2 |

### XVI.8 로컬 진단·선택적 텔레메트리

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XVI.8.1 | `[ ]` | 로컬 diagnostic store에 app crash, renderer error, performance trace, feature failure count schema를 정의합니다. | schema round-trip. | II.3.2, XVI.1.5 |
| XVI.8.2 | `[ ]` | 금지 필드 scanner를 구현해 project text/name/path, Git URL/branch/commit, terminal/MCP payload를 진단 번들에서 탐지합니다. | secret corpus 탐지 100%. | XVI.8.1 |
| XVI.8.3 | `[ ]` | telemetry consent 상태를 NotAsked, Declined, Granted로 정의하고 기본은 NotAsked입니다. 자동 전송은 Granted에서만 가능합니다. | state transition fixture. | XVI.8.1 |
| XVI.8.4 | `[ ]` | 전송 전 payload preview API와 수동 diagnostic bundle export를 구현합니다. | preview bytes와 실제 upload bytes 동일. | XVI.8.2, XVI.8.3 |
| XVI.8.5 | `[ ]` | consent revoke 시 queued upload를 폐기하고 이후 전송을 차단합니다. 로컬 diagnostics는 정책에 따라 유지합니다. | revoke fixture network request 0. | XVI.8.4 |
| XVI.8.6 | `[ ]` | 터미널·MCP가 초기 출시 feature off인 빌드에서도 해당 payload 필드가 schema에 존재하지 않음을 검사합니다. | feature inventory 통과. | XVI.8.1 |

### XVI.9 macOS 패키징·서명·업데이트

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XVI.9.1 | `[x]` | Release configuration, bundle identifier, minimum macOS 26, arm64-only 설정을 고정합니다. | built binary architecture arm64 only. | Phase XII |
| XVI.9.2 | `[~]` | Rust core library와 resources를 app bundle에 포함하고 rpath/code signature 구조를 검증합니다. | clean machine launch smoke. | XVI.9.1 |
| XVI.9.3 | `[~]` | Developer ID signing pipeline을 구성합니다. 로컬 개발 key를 repo에 저장하지 않습니다. | `codesign --verify --deep --strict` 통과. | XVI.9.2 |
| XVI.9.4 | `[~]` | notarization submission과 ticket stapling pipeline을 구성합니다. | Gatekeeper clean machine 검증 통과. | XVI.9.3 |
| XVI.9.5 | `[~]` | Stable/Beta update feed를 분리하고 앱은 background check만 수행합니다. 다운로드·설치는 사용자 승인 후입니다. | channel isolation fixture. | XVI.9.2 |
| XVI.9.6 | `[!]` | 업데이트 전 open project 상태를 저장·복구하고 강제 종료하지 않습니다. | dirty project update prompt fixture. | XIII.1.6, XVI.9.5 |

### XVI.10 Windows MSIX·서명·업데이트

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XVI.10.1 | `[!]` | Release x64 MSIX packaging, Windows 11 minimum, package identity를 고정합니다. | clean Windows install/uninstall smoke. | Phase XII |
| XVI.10.2 | `[!]` | Rust DLL/static linkage와 VC runtime 정책을 고정하고 missing dependency가 없게 합니다. | clean VM launch. | XVI.10.1 |
| XVI.10.3 | `[!]` | code signing pipeline을 구성하고 certificate secret을 CI secret store에만 둡니다. | signature verification 통과. | XVI.10.1 |
| XVI.10.4 | `[!]` | Stable/Beta package feed를 분리하고 사용자 승인형 update flow를 구현합니다. | channel isolation fixture. | XVI.10.3 |
| XVI.10.5 | `[!]` | update 전 dirty project save/recovery 상태를 처리하고 강제 종료하지 않습니다. | dirty project fixture 통과. | XIII.1.6, XVI.10.4 |

### XVI.11 초기 릴리스 End-to-End Gate

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XVI.11.1 | `[ ]` | 새 프로젝트 생성→Frame/Text/Image 추가→Component/Token/Variant 생성→Save→Close→Reopen E2E를 양 플랫폼에 작성합니다. | semantic snapshot 동일. | Phases 3~12 |
| XVI.11.2 | `[ ]` | object property partial stage→commit→empty GitHub fixture remote connect→push E2E를 작성합니다. 실제 GitHub 네트워크 대신 local bare remote와 URL validation unit을 분리합니다. | local push 성공, remote refs 정확. | Phase XIV |
| XVI.11.3 | `[ ]` | 두 branch conflict 생성→Semantic Conflict 해결→git add 상태까지 E2E를 작성합니다. | unresolved entries 0, merge state 유지. | XIV.14.4 |
| XVI.11.4 | `[ ]` | 외부 library v1/v2 동시 사용→update→override orphan E2E를 작성합니다. | expected warning/diagnostic. | XV.8.4 |
| XVI.11.5 | `[ ]` | autosave 중 crash→relaunch→recovery 선택→Git working tree 확인 E2E를 작성합니다. | 미저장 변경 복구, history는 비어 있음. | XIII.8.7 |
| XVI.11.6 | `[ ]` | 동일 프로젝트 두 창에서 다른 편집→첫 창 save→두 번째 창 save alert의 두 선택 경로 E2E를 작성합니다. | 각각 정책 결과 정확. | XII.7.7 |
| XVI.11.7 | `[ ]` | reference hardware performance gate를 실행하고 G8 release report를 생성합니다. | 모든 required threshold 통과. | XVI.4.6, XVI.11.1~XVI.11.6 |
| XVI.11.8 | `[ ]` | release feature inventory를 검사해 내장 터미널, MCP, agent, export, plugin, codegen, advanced Git UI가 초기 빌드에 노출되지 않음을 확인합니다. | roadmap feature command/menu/API disabled. | XIV.13.7, XVI.8.6 |
| XVI.11.9 | `[ ]` | 초기 릴리스 수용 기준 체크리스트를 자동 생성하고 각 항목을 테스트 artifact에 링크합니다. 수동 확인만 있는 항목은 owner와 증거를 명시합니다. | 미증빙 항목 0. | XVI.11.7, XVI.11.8 |

**Phase XVI 완료 조건:** G8 통과 후에만 초기 릴리스 후보를 생성합니다.

---


---

## Phase XVII. 실제 착수 순서와 병렬 작업 규칙

### XVII.1 권장 실행 Wave

| Wave | 필수 작업 | 병렬 가능 작업 | 종료 조건 |
|---|---|---|---|
| W0 | Phase I | 없음 | 저장소·작업 추적·CI 기준선 확정 |
| W1 | Phase II | macOS II.5와 Windows II.6 병렬 | G1 |
| W2 | Phase III | III.3/III.4와 III.5/III.6 일부 병렬 | core primitive 안정화 |
| W3 | Phase IV, Phase V.1\~V.3, Phase VI.1\~VI.4 | 프로젝트 수명주기·저장 primitive·노드 모델 병렬 | 빈 프로젝트와 기본 노드 모델 준비 |
| W4 | Phase V 나머지, Phase VI 나머지, Phase VII | crate별 독립 작업만 병렬 | G3·G4 |
| W5 | Phase VIII, Phase IX.1~IX.4 | Expression/Token과 Component 기본 모델 병렬 | Binding·Component 기본 연결 |
| W6 | Phase IX 나머지, Phase X | 상태·Variant·접근성과 layout/text 일부 병렬 | G5 |
| W7 | Phase XI.1~XI.7 | Scene·RenderGraph·cache 순차 중심 | backend 공통 계약 확정 |
| W8 | Phase XI.8과 XI.9, Phase XII macOS/Windows | Metal·D3D12, 양 셸 병렬 | G6 |
| W9 | Phase XIII, Phase XIV.1~XIV.4 | file/asset과 Git snapshot 병렬 | 저장된 프로젝트 운영 준비 |
| W10 | Phase XIV 나머지, Phase XV | Git semantic pipeline과 library 일부 병렬 | G7 |
| W11 | Phase XVI | platform packaging만 병렬 | G8 |

### XVII.2 병렬 실행 금지 규칙

1. 같은 crate를 수정하는 작업 두 개를 동시에 실행하지 않습니다.
2. 루트 `Cargo.toml`, `Cargo.lock`, Xcode project, Visual Studio solution, FFI header generator는 한 시점에 작업 하나만 수정합니다.
3. 파일 포맷 golden fixture를 수정하는 작업은 migration 작업과 병렬 실행하지 않습니다.
4. PropertyKey, NodeKind, CommandKind, ErrorCode 같은 numeric registry를 수정하는 작업은 전역 mutex 작업으로 취급합니다.
5. Metal과 D3D12 backend는 공통 `RendererBackend` interface가 `[x]`가 된 후에만 병렬 실행합니다.
6. macOS와 Windows 구현이 모두 필요한 작업은 공통 fixture를 먼저 확정하고 각 플랫폼 작업을 별도 WBS로 실행합니다.
7. Semantic Diff, Staging, Merge 작업은 같은 fixture branch를 공유하지 않습니다. fixture commit을 먼저 병합한 뒤 각 작업을 시작합니다.
8. 한 에이전트가 다른 작업의 테스트를 임의로 수정해 통과시키지 않습니다.
9. 실패한 작업을 우회하기 위해 선행 ID를 건너뛰지 않습니다.
10. 전체 리팩터링은 별도 WBS가 없는 한 금지합니다.

### XVII.3 첫 착수 Queue

현재 상태에서는 아래 순서를 바꾸지 않습니다.

1. `I.1.6` 현재 계획·환경 계약을 기준으로 history reword/drop 수행
2. `I.1.7` Rust 도구체인 commit·보고서 ID 교정
3. `I.1.8` 최소 workspace commit·보고서 ID 교정
4. `I.1.9` xtask help commit·보고서 ID 교정
5. `I.1.10` macOS·Windows Bootstrap CI
6. `I.2.1` TaskStatus Rust 데이터 모델
7. `I.2.2` WBS ID 타입
8. `I.2.3` yaml-rust2 bootstrap 승인
9. `I.2.4` restricted YAML event loader
10. `I.2.5` TaskStatus YAML decoder
11. `I.2.6`~`I.2.9` record·collection·graph·evidence validator
12. `I.2.10`~`I.2.12` 계획 추출·ledger writer·ledger 생성
13. `I.2.13`~`I.2.17` 보고서·PR·commit validator
14. `I.3.1`~`I.3.5` 버전·dependency·ADR 정책
15. `I.4.1`~`I.4.4` 공통 명령과 G0 CI
16. `II.1.1` 제품 Rust baseline 승격
17. `II.1.2` 제품 workspace 확장
18. 이후 Wave W1

`I.1.10`과 `I.4.4`의 `CI-WIN` 증거가 없으면 Phase I 및 G0를 완료 처리하지 않습니다. 첫 Queue 완료 전에는 document model, renderer, Git 기능 구현을 시작하지 않습니다.


---

## Phase XVIII. 로드맵 — 내장 네이티브 PTY 터미널

### XVIII.1 터미널 세션 코어

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XVIII.1.1 | `[x]` | `TerminalSessionId`, profile, cwd, environment allowlist, process state를 정의합니다. project 저장·recovery snapshot에는 포함하지 않습니다. | round-trip은 세션 메모리 전용. | G8 |
| XVIII.1.2 | `[x]` | 프로젝트 창당 여러 terminal session registry를 구현합니다. | 20 session create/close 누수 없음. | XVIII.1.1 |
| XVIII.1.3 | `[x]` | terminal cwd는 project root 또는 사용자가 선택한 worktree root를 preset으로 제공하고 임의 cwd도 명시 입력으로 허용합니다. | cwd validation fixture 통과. | XIV.1.2, XVIII.1.1 |
| XVIII.1.4 | `[x]` | terminal은 trusted project에서만 시작되게 capability check를 연결합니다. | untrusted spawn 0. | XVI.7.1, XVIII.1.2 |

### XVIII.2 macOS POSIX PTY

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XVIII.2.1 | `[x]` | POSIX PTY create와 child process spawn wrapper를 구현합니다. | `/bin/echo` output fixture. | XVIII.1.2 |
| XVIII.2.2 | `[x]` | 사용자 기본 shell과 login/non-login profile을 설정에서 선택하게 합니다. | shell path 없음 오류 처리. | XVIII.2.1 |
| XVIII.2.3 | `[x]` | resize를 winsize ioctl로 전달합니다. | columns/rows 확인 fixture. | XVIII.2.1 |
| XVIII.2.4 | `[x]` | child exit, signal, cancellation, app close 시 process tree 정리를 구현합니다. | orphan process 0. | XVIII.2.1 |
| XVIII.2.5 | `[ ]` | 대화형 terminal session의 child 환경을 ADR-0023에 맞춥니다. 이 행은 완료 행 XVIII.1.1이 정의한 "environment allowlist"(구현 타입 `EnvironmentPolicy`, `crates/terminal/src/env.rs`)를 대화형 세션에 대해 대체하며, XVIII.1.1 문구와 상태는 그대로 둡니다(ADR-0023 Consequences). spawn 경로에서 `env_clear`와 상속 허용 목록(`DEFAULT_INHERITED_ENV`)을 제거하고 앱 프로세스 환경을 그대로 물려준 뒤, 터미널 식별 변수 `TERM`·`COLORTERM`·`TERM_PROGRAM`·`TERM_PROGRAM_VERSION`을 설정하고, 기본 셸 profile이면 `SHELL`을 실제로 실행하는 셸 경로로 설정하며(지금 동작 유지, argv profile은 상속값 유지), profile 명시 지정을 마지막에 적용합니다(ADR-0023 결정 4·5). 식별 변수 값은 spawn 입력으로 받으며 `TERM`·`COLORTERM` 값은 XVIII.4.1이 emulator에 맞춰 확정합니다. 그 밖의 변수는 추가하지 않고 사용자 셸 설정 파일을 만들거나 고치지 않습니다. 허용 목록 환경은 agent CLI 실행기(XXIX.2.4, ADR-0023 결정 9)에만 둡니다. | `[DEV-MAC]` 임시 HOME의 login 셸 fixture에서 rc·profile 파일이 더한 `PATH` 항목과 marker 변수가 child에 보임, 앱 환경에만 있는 기존 허용 목록 밖 변수가 child에 보임, 식별 변수 4개가 spawn 입력 값과 일치, 계정 로그인 셸과 다른 셸을 고른 기본 셸 profile fixture에서 `SHELL`이 실행한 셸 경로와 같음, argv profile fixture에서 `SHELL`이 상속값과 같음, Stellar가 더한 변수는 식별 변수 4개와 `SHELL` 외 0, profile 명시 지정 우선 fixture 통과, 세션 전후 rc·profile 파일 hash 동일, 대화형 spawn 경로의 `env_clear`·상속 허용 목록 사용 inventory 0. | XVIII.1.4, XVIII.2.2 |

### XVIII.3 Windows ConPTY

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XVIII.3.1 | `[!]` | ConPTY create와 child process spawn wrapper를 구현합니다. 기본 profile은 PowerShell입니다. | `Write-Output` fixture. | XVIII.1.2 |
| XVIII.3.2 | `[!]` | 사용자 지정 shell profile과 argv를 shell string이 아닌 배열로 저장합니다. | quoting fixture 통과. | XVIII.3.1 |
| XVIII.3.3 | `[!]` | resize API를 연결합니다. | columns/rows fixture. | XVIII.3.1 |
| XVIII.3.4 | `[!]` | job object 또는 동등 process tree 정리로 app close 시 child를 종료합니다. | orphan process 0. | XVIII.3.1 |

### XVIII.4 Terminal emulator와 UI

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XVIII.4.1 | `[x]` | 승인된 terminal parser/emulator dependency를 등록하고 VT sequence corpus를 테스트합니다. 고른 emulator가 지원하는 terminfo 항목으로 대화형 세션의 `TERM` 값을 정하고, emulator의 24-bit 색 지원 여부로 `COLORTERM`을 `truecolor`로 설정할지 상속값을 제거할지 정해 근거와 함께 보고서에 기록합니다(ADR-0023 결정 4). 두 값은 terminal crate 상수로 두고 XVIII.2.5의 spawn 입력이 받습니다. | 기본 cursor/color/erase fixture 통과, 정한 `TERM` 항목이 선언하는 색 수·cursor 이동·지우기 capability를 VT corpus fixture로 확인, 24-bit 색 sequence fixture 결과가 `COLORTERM` 결정과 일치, 보고서에 `TERM`·`COLORTERM` 값과 근거 존재. | I.3.2, XVIII.2.1, XVIII.3.1 |
| XVIII.4.2 | `[x]` | terminal screen buffer, scrollback, selection, clipboard 모델을 구현합니다. | 100만 line memory cap 유지. | XVIII.4.1 |
| XVIII.4.3 | `[ ]` | macOS terminal view가 screen buffer를 표시하고 keyboard/IME input을 PTY로 전달합니다. | interactive smoke. | XVIII.2.3, XVIII.4.2 |
| XVIII.4.4 | `[!]` | Windows terminal view를 연결합니다. | interactive smoke. | XVIII.3.3, XVIII.4.2 |
| XVIII.4.5 | `[x]` | 일반 terminal과 특정 worktree terminal의 header metadata를 분리 표시할 data model을 추가합니다. | wrong cwd session 경고. | XVIII.1.3 |
| XVIII.4.6 | `[ ]` | 창 종료 시 terminal을 복원하지 않고 종료하며 recovery snapshot에 output/process를 넣지 않는 검사를 추가합니다. | crash/relaunch 후 process 재연결 없음. | XIII.8.2, XVIII.4.3, XVIII.4.4 |
| XVIII.4.7 | `[ ]` | macOS terminal view에 Stellar 테마를 적용합니다(ADR-0023 결정 7). ANSI 16색, 기본 전경·배경, 선택 영역, cursor 색·모양·깜박임, 글꼴 family·크기를 asset catalog의 Light·Dark·대비 높이기 변형으로 정의하고 ShellTheme 이벤트로 실행 중에 바꿉니다. 동작 줄이기가 켜져 있으면 cursor 깜박임을 끕니다. 기본 전경·배경 색 질의에는 현재 테마 값으로 응답합니다. 테마 변경은 표시만 바꾸며 셸 프로세스 재시작, 환경 변수 변경, 사용자 셸 설정 파일 쓰기를 하지 않습니다. 테마·글꼴 설정은 앱 사용자 설정이며 프로젝트 파일에 넣지 않습니다. | `[DEV-MAC]` Light·Dark·대비 높이기 snapshot fixture 3종 golden 일치, 기본 전경·배경 대비가 Light·Dark에서 4.5:1 이상·대비 높이기에서 7:1 이상, 테마 전환 전후 셸 pid와 child 환경 동일, 세션 전후 사용자 rc·profile 파일 hash 동일, 색 질의 응답이 현재 테마 값과 일치, 프로젝트 파일 쓰기 0. | XVIII.4.3, XII.9.3 |

### XVIII.5 CLI 에이전트 실행

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XVIII.5.1 | `[x]` | Claude Code, Codex 등을 특별 파서 없이 일반 terminal profile command로 실행할 수 있게 합니다. | arbitrary CLI command smoke. | XVIII.4.5 |
| XVIII.5.3 | `[x]` | 일반 Git worktree 생성·사용은 시스템 Git 기능으로 남기고 terminal cwd preset만 제공합니다. agent 전용 ShadowWorktree 타입을 만들지 않습니다. | type inventory 검사 통과. | XVIII.1.3, XIV.1.2 |

**Phase XVIII 완료 조건:** 다중 PTY 세션이 양 플랫폼에서 동작하며 프로젝트 신뢰·종료 정책을 준수합니다.

---

## Phase XIX. 로드맵 — MCP 서버와 외부 에이전트 편집

### XIX.1 `stellar-mcp` stdio 브리지

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIX.1.1 | `[x]` | 별도 `tools/stellar-mcp` 실행 파일을 생성하고 stdin/stdout JSON-RPC 프레이밍을 구현합니다. stdout에는 프로토콜 메시지 외 로그를 쓰지 않습니다. | protocol smoke fixture. | G8 |
| XIX.1.2 | `[x]` | 브리지가 실행 중 Stellar 앱을 찾는 로컬 IPC endpoint registry를 구현합니다. network port는 열지 않습니다. | app 없음/한 개/여러 개 fixture 통과. | XIX.1.1 |
| XIX.1.3 | `[x]` | bridge↔app IPC에 session nonce와 OS 사용자 identity 검증을 적용합니다. | 다른 사용자·재사용 nonce 거부. | XIX.1.2 |
| XIX.1.4 | `[x]` | 한 bridge process는 한 MCP 연결 세션만 담당합니다. bridge가 끝나면 앱은 연결 종료 event를 한 번 기록합니다. 외부 클라이언트 세션은 그 시점에 XIX.6.4 종료 절차(권한·lease 해제, 자동 rollback 없음)를 시작하고, 앱이 시작한 세션의 권한·lease 수명은 AgentSession(XXIX.2.8)이 정합니다. | `[DEV-MAC]` 외부 bridge disconnect 후 session permission 0, 연결 종료 event 1회, 문서 revision 변화 0. | XIX.1.3 |
| XIX.1.5 | `[x]` | MCP protocol version을 config에 고정하고, 지원하지 않는 client version 요청에는 MCP 수명주기 명세대로 지원하는 version으로 응답합니다(client가 계속할지 정함). 형식이 틀린 요청은 명확한 오류를 반환합니다. | version matrix fixture. | XIX.1.1 |
| XIX.1.6 | `[x]` | 앱 쪽 IPC가 데이터 교환 전에 연결 상대를 인증합니다(ADR-0005 §5, 아키텍처 37 신뢰 경계). 인증 기준과 방법은 Apple 공식 문서를 근거로 정해 보고서에 기록하고, 인증은 UI·Document 스레드 밖에서 합니다. | `[DEV-MAC]` 인증 실패 fixture에서 데이터 교환 0, UI·Document 스레드 호출 0. | XIX.1.3 |
| XIX.1.7 | `[x]` | `stellar-mcp` 쪽 IPC가 데이터 교환 전에 연결 상대를 XIX.1.6과 같은 기준으로 인증합니다(ADR-0005 §5). 실패 시 종료 코드는 XXIV.1.2 표를 따릅니다. | `[DEV-MAC]` 인증 실패 fixture에서 데이터 교환 0, 종료 코드가 XXIV.1.2 표와 일치. | XIX.1.6, XXIV.1.2 |
| XIX.1.8 | `[~]` | 앱이 시작한 bridge를 그 세션에만 결합합니다(아키텍처 31.3, ADR-0005 §5). 세션이 끝나면 결합 수단은 무효가 되고, 결합은 권한을 부여하지 않으며 XIX.3 승인을 대체하지 않습니다. 결합 수단과 검증 방법은 Apple 공식 문서와 각 CLI 공식 문서를 근거로 정해 보고서에 기록합니다. | `[DEV-MAC]` 다른 세션·종료된 세션 결합 거부, 결합만으로 얻는 권한 0. | XIX.1.3, XVI.7.5 |
| XIX.1.9 | `[x]` | `stellar-mcp`가 표준 입력 EOF나 부모 프로세스 종료를 감지하면 스스로 종료합니다. 앱은 이를 연결 종료로 처리하고 XIX.1.4를 따릅니다. | `[DEV-MAC]` 부모 강제 종료 fixture에서 bridge 잔존 0. | XIX.1.1 |

### XIX.2 MCP Resources와 Tools

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIX.2.1 | `[x]` | read resources에 project structure, current document, selection, components, tokens, target profile, Git status를 정의합니다. | schema fixture. | XIX.1.4, G8 |
| XIX.2.2 | `[x]` | resource response에 revision과 pagination cursor를 포함하고 전체 대형 document를 한 응답에 보내지 않습니다. | 20만 node pagination 통과. | XIX.2.1 |
| XIX.2.3 | `[x]` | tools를 semantic diff 생성, render preview request, document validation, command proposal, command apply로 분리합니다. | tool inventory 정확. | XIX.2.1 |
| XIX.2.4 | `[x]` | MCP는 terminal, commit, push 권한을 문서 apply 권한과 별도 tool namespace로 구분합니다. 초기에는 commit/push tool을 비활성 기본값으로 둡니다. | permission matrix 통과. | XIX.2.3 |
| XIX.2.5 | `[~]` | 모든 MCP payload에서 project content가 agent에 전달되는 범위를 user-approved project/session으로 제한합니다. | cross-project access 거부. | XIX.1.4 |

### XIX.3 권한 모델

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIX.3.1 | `[ ]` | permission을 Read, Propose, Apply, Terminal, FileDelete, GitCommit, GitPush로 정의합니다. | enum golden test. | XIX.2.4 |
| XIX.3.2 | `[ ]` | 권한을 ProjectIdentity+AgentSessionId별로 저장하고 app 재시작 후 유지하지 않습니다. | restart 후 permission 없음. | XIX.3.1, IV.2.3 |
| XIX.3.3 | `[ ]` | 앱에 붙는 MCP 연결은 trusted project라도 첫 연결 승인 전에는 Read도 허용하지 않는 기본 정책을 구현합니다. 예외로 앱 없이 실행한 headless 읽기 backend는 실행 인자로 고정한 trusted project 하나에 한해 읽기·검증·dry-run을 승인 없이 허용하고(2026-11-14 제품 소유자 결정), 쓰기 권한을 주지 않으며, untrusted 프로젝트에서는 모든 tool을 거부합니다. 앱이 시작한 내장 세션의 첫 연결 승인 기록은 XXIX.3.5가 담당합니다. | `[DEV-MAC]` attached 첫 연결 denied/prompt state, headless untrusted fixture 전 tool 거부, headless 정책 결과에 쓰기 권한 0. | XVI.7.2, XIX.3.2 |
| XIX.3.4 | `[ ]` | Apply, Terminal, GitCommit, GitPush 요청은 각각 독립 확인 상태를 반환합니다. 한 번의 Apply 승인으로 Push를 허용하지 않습니다. | permission escalation fixture 실패. | XIX.3.1 |

### XIX.4 에이전트 편집 선언과 하위 트리 잠금

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIX.4.1 | `[ ]` | agent가 편집 시작 전 target root StableId 목록과 작업 설명을 제출하는 `BeginAgentEdit` tool을 정의합니다. | empty/missing node 거부. | XIX.2.3, XIX.3.4 |
| XIX.4.2 | `[ ]` | 승인 후 각 target node와 전체 descendant를 잠그는 `AgentEditLease`를 생성합니다. | 새로 추가된 descendant 자동 잠금. | XIX.4.1, VI.2.3 |
| XIX.4.3 | `[ ]` | 잠긴 subtree의 속성 변경, 자식 추가·삭제·이동, 상위 삭제·이동을 user command validator가 거부합니다. 조회·선택은 허용합니다. | operation matrix 통과. | XIX.4.2, Phase VII |
| XIX.4.4 | `[ ]` | 외부 token, component definition, asset 수정은 별도 root lock을 요청해야 합니다. 자동 연쇄 잠금은 하지 않습니다. | unrequested external edit 거부. | XIX.4.2 |
| XIX.4.5 | `[ ]` | 같은 project의 모든 창·프로세스가 local ProjectLockCoordinator로 lease를 공유합니다. 다른 worktree는 별도 project identity로 취급합니다. | multi-window/process fixture 통과. | IV.2.3, XII.2.4 |
| XIX.4.6 | `[ ]` | 여러 agent는 겹치지 않는 subtree만 동시에 lock할 수 있습니다. 동일 또는 ancestor/descendant request는 거부합니다. | overlap matrix 통과. | XIX.4.5 |
| XIX.4.7 | `[ ]` | user interaction preview 중인 node는 gesture 종료 후에만 agent lease를 획득합니다. | drag 중 lock 대기 fixture. | XII.3.7, XIX.4.2 |

### XIX.5 일반 Command와 동일한 적용

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIX.5.1 | `[ ]` | agent command proposal은 일반 CommandTransaction closed enum만 사용합니다. raw memory patch와 임의 script는 금지합니다. | unknown command 거부. | XIX.2.3, VII.1.6 |
| XIX.5.2 | `[ ]` | proposed transaction이 lease 범위를 벗어나는 StableId를 수정하면 거부합니다. | scope escape fixture 실패. | XIX.4.2, XIX.5.1 |
| XIX.5.3 | `[ ]` | user preview에는 semantic diff와 validation 결과를 제공하고 승인 전 document state를 변경하지 않습니다. | preview 전후 revision 동일. | XIV.3.10, XIX.5.1 |
| XIX.5.4 | `[ ]` | 승인된 command는 일반 DocumentThread queue, Binding/Layout/Scene invalidation, autosave 경로로 적용됩니다. | user command와 trace 경로 동일. | XII.1.2, XIX.5.3 |
| XIX.5.5 | `[ ]` | 한 agent edit lease 동안 여러 승인 batch를 임시 작업 그룹에 모으고 정상 CompleteAgentEdit에서 Undo entry 하나로 확정합니다. | 10 batch→history 1 entry. | VII.4.1, XIX.5.4 |

### XIX.6 완료·중단과 사용자 되돌리기

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIX.6.1 | `[ ]` | 승인 batch마다 inverse transaction을 보존해 변경 단위·세션 단위 되돌리기(XIX.6.8, XIX.6.9)에 씁니다. full document clone은 금지합니다. | `[DEV-MAC]` 1000 command batch의 inverse를 역순 적용하면 baseline semantic snapshot 복원. | XIX.5.5 |
| XIX.6.2 | `[ ]` | 정상 CompleteAgentEdit에서 변경 그룹을 history entry로 확정하고 lease를 해제합니다. | 완료 후 user edit 가능. | XIX.6.1 |
| XIX.6.3 | `[ ]` | user CancelAgentEdit(현재 세션 중지)는 rollback하지 않고 XIX.6.4 종료 절차를 따릅니다. 중지 후 세션 되돌리기는 종료 절차를 마친 뒤 XIX.6.9 세션 되돌리기 미리보기를 엽니다. 별도 rollback 경로는 두지 않습니다. | `[DEV-MAC]` 중지 fixture에서 적용 batch 유지·lease 0·중단 history entry 1, 중지하고 되돌리기 fixture에서 미리보기 승인 전 revision 변화 0. | XIX.6.4, XIX.6.9 |
| XIX.6.4 | `[ ]` | 세션 종료(외부 클라이언트의 MCP disconnect, agent process 종료, 사용자 중지, 재연결 유예 초과, 비정상 종료, 신뢰 철회, lease와 겹치는 외부 변경, 사용자의 lease 해제) 시 자동 rollback하지 않습니다. 앱이 시작한 세션은 bridge disconnect·crash 뒤에도 재연결 유예 동안 유지하고 유예를 넘기면 종료합니다(ADR-0005 §4.2). 적용 중인 transaction은 전부 적용 또는 전부 미적용으로 끝내고, 진행 중 작업 그룹을 "중단됨" history entry 하나로 확정하며, 적용된 batch의 출처 표시(XIX.6.7)를 유지하고, lease와 session permission을 즉시 해제하며, 미적용 제안은 폐기하고 대기 승인 요청은 거부로 닫습니다. | `[DEV-MAC]` kill fixture에서 적용 batch 유지, 출처 표시 존재, lease 0, permission 0, 대기 요청 0, history entry 1(중단 표시). 앱 시작 세션 bridge disconnect fixture에서 유예 안 세션 유지, 유예 초과 후 같은 종료 결과. | XIX.1.4, XIX.6.2, XIX.6.7 |
| XIX.6.6 | `[ ]` | Interrupted Agent Task에는 내용 없이 session id, duration, error code만 local diagnostic으로 남깁니다. | project content 미포함. | XVI.8.1, XIX.6.4 |
| XIX.6.7 | `[ ]` | 에이전트 변경 출처 색인 `AgentChangeProvenance`를 창 세션 메모리에 정의합니다. 적용된 batch마다 AgentSessionId, CLI가 보고한 세션 표시 이름, batch 순번, history sequence, VII.1.4 결과의 변경 StableId·PropertyPath 집합, 생성·삭제 노드, StableId+PropertyPath별 적용 직후 값 지문, 상태(완료·중단)를 기록합니다. batch command의 origin은 VII.1.1의 FutureAgent를 쓰고 VII 타입은 바꾸지 않습니다. 프로젝트 파일, Git, generation metadata, recovery snapshot, 진단, 텔레메트리에는 넣지 않으며 창을 닫으면 WindowHistory와 함께 버립니다. | `[DEV-MAC]` batch→StableId·PropertyPath 매핑 fixture 정확, 저장 bytes·recovery snapshot·진단에 출처 필드 0, 창 닫기 후 색인 0. | XIX.5.5, VII.1.1, VII.1.4, VII.4.1 |
| XIX.6.8 | `[ ]` | 변경 단위 되돌리기를 구현합니다. 사용자가 고른 batch 하나의 inverse를 새 User origin CommandTransaction으로 만들어 DocumentThread에 적용하고 "에이전트 변경 되돌리기" history entry 하나를 남깁니다(Undo 가능). 전제조건은 StableId+PropertyPath별 현재 값과 적용 직후 값 지문의 일치입니다. 이후 변경된 속성, 에이전트가 만든 노드 아래 이후 추가된 자식·참조, 외부 inbound ref, 이미 삭제된 노드는 충돌로 분류해 기본 제외하고 목록으로 반환합니다. 충돌 항목은 사용자가 명시적으로 고른 경우에만 복원하며, 사용자 콘텐츠가 함께 사라지는 구조 충돌의 기본값은 현재 유지입니다. 승인 전 미리보기는 XIX.5.3 경로를 쓰고, 다른 활성 lease 범위와 겹치면 XIX.4.3에 따라 거부합니다. 결과는 일반 autosave 경로로 저장합니다. | `[DEV-MAC]` 비충돌 batch 되돌리기 후 해당 속성 baseline 일치, 이후 사용자 편집 속성 기본 보존, 되돌리기 Undo 후 재적용 일치, 미리보기 전후 revision 동일, 활성 lease 겹침 거부, 결과 autosave 예약. | XIX.6.7, XIX.6.1, XIX.4.3, XIX.5.3 |
| XIX.6.9 | `[ ]` | 세션 단위 되돌리기를 구현합니다. 한 AgentSession의 남은 batch를 역순으로 XIX.6.8 규칙에 따라 하나의 CommandTransaction으로 합쳐 되돌리고 history entry 하나를 남깁니다. 전부 성공하거나 전부 미적용입니다. | `[DEV-MAC]` 10 batch 세션 되돌리기 후 비충돌 속성 baseline 일치, history entry 1, 충돌 목록 정확, 중간 실패 fixture에서 효과 0. | XIX.6.8 |
| XIX.6.10 | `[ ]` | 출처 표시 상태 전이를 구현합니다. 사용자가 변경·세션 단위로 "유지"를 고르면 문서를 바꾸지 않고 표시만 지웁니다. 표시된 속성을 사용자·다른 세션·외부 변경이 다시 바꾸면 "이후 변경됨", 창 Undo로 에이전트 entry를 취소하면 "실행 취소됨", 되돌리기를 확정하면 "되돌림"이 되고, 그 Undo·Redo에 따라 이전 상태로 돌아갑니다. 이 동작은 사용자 UI 경로에서만 호출할 수 있습니다. | `[DEV-MAC]` 유지 전후 revision·저장 bytes·history 동일, Undo/Redo·되돌리기에 따른 상태 전이 fixture 통과. | XIX.6.8 |

### XIX.7 외부 파일 변경과 잠금

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIX.7.1 | `[ ]` | external change batch를 semantic diff로 계산하고 locked subtree overlap을 판정합니다. | overlap/non-overlap fixture 통과. | XIII.3.4, XIX.4.2 |
| XIX.7.2 | `[ ]` | overlap이면 agent 세션을 XIX.6.4 절차로 끝내고(자동 rollback 없음, lease 해제, 대기 제안 거부), 적용된 agent 변경은 출처 표시를 유지한 로컬 변경으로 XIII.3.4 reconciliation에 넣어 외부 변경을 반영합니다. 의미 충돌은 Semantic Conflict 경로로 보냅니다. | `[DEV-MAC]` overlap fixture에서 agent 변경 자동 삭제 0, 겹친 속성 Semantic Conflict 진입, lease 0. | XIX.7.1, XIX.6.4, XIII.3.4 |
| XIX.7.3 | `[ ]` | non-overlap external change는 agent 작업을 유지한 채 적용합니다. | independent subtree fixture 통과. | XIX.7.1 |

### XIX.8 Worktree 관계 검증

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIX.8.1 | `[ ]` | MCP/agent model에 ShadowWorktree 전용 타입이 없는지 architecture inventory test를 추가합니다. | 일반 ProjectIdentity/Worktree만 존재. | XIX.5.1 |
| XIX.8.2 | `[ ]` | agent가 terminal에서 직접 파일을 수정하면 일반 외부 파일 변경으로만 처리되고 MCP lease 보호를 우회할 수 있음을 UI 경고 상태로 표현합니다. | direct edit fixture external path 사용. | XVIII.5.1, XIII.3.4 |

### XIX.9 L2 경계와 backend

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XIX.9.1 | `[~]` | `stellar-local-ipc`(`crates/local-ipc`), `stellar-agent-protocol`(`crates/agent-protocol`), `stellar-mcp-core` library(`crates/mcp-core`)를 crate graph 정책과 crate 의존 그래프 문서에 등록합니다. agent-protocol과 local-ipc는 core-types만, mcp-core는 ops·agent-protocol·local-ipc·diagnostics만 의존합니다. `tools/stellar-mcp` package(실행 이름 `stellar-mcp`)의 정책 허용 의존에 `stellar-mcp-core`를 추가하고, 이 실행 파일이 library 진입점만 갖도록 XIX.1.1~XIX.1.3 구현을 이 경계로 옮깁니다. | `[DEV-MAC]` `cargo xtask validate-crate-graph` 통과, tools 진입점에 protocol 로직 0. | XIX.1.3, XXIV.4.2, XXIX.1.1 |
| XIX.9.2 | `[ ]` | resource 목록(XIX.2.1)과 tool 목록(XIX.2.3)을 `stellar-agent-protocol`의 Rust 정의에서 생성합니다. CommandTransaction 입력 schema는 xtask가 VII.1.6 closed enum에서 생성해 `schemas/`에 두고 agent-protocol은 그 산출물만 포함합니다. tool 이름은 MCP 명세 규칙을 따르고 읽기·파괴 계열에 annotation을 붙입니다. Stellar 사용 안내는 MCP 명세에 따른 instructions로 제공합니다. `schemas/`는 보호 경로이므로 같은 변경에 ADR을 갱신합니다. | `[DEV-MAC]` catalog golden 일치, 생성 schema와 enum variant 일치, agent-protocol의 commands 의존 0. | XIX.9.1, XIX.2.1, XIX.2.3, XIX.5.1 |
| XIX.9.3 | `[~]` | `stellar-mcp-core`에 headless 읽기 backend를 구현합니다(2026-11-14 제품 소유자 결정). 실행 인자로 고정한 프로젝트 루트 하나만 다루고 tool 인자로 다른 경로를 지정할 수 없습니다. XVI.7 trust store에서 trusted가 확인되지 않으면 모든 tool을 거부합니다. 노출하는 것은 `stellar-ops`의 읽기·검증·dry-run tool뿐이고 쓰기 tool은 tools/list에 넣지 않습니다. | `[DEV-MAC]` untrusted fixture 전 tool 거부, 다른 경로 요청 거부, headless tools/list의 쓰기 tool 0, 실행 전후 프로젝트 파일 hash 동일. | XIX.9.2, XXIV.4.3, XVI.7.2, XIX.3.3 |
| XIX.9.4 | `[ ]` | attached backend를 구현합니다. backend 선택은 앱이 기록한 목록이 아니라 endpoint 실시간 검증(XIX.1.7) 결과로 하고 세션 시작 시 고정합니다. 결합 단위는 WindowSessionId입니다. 외부 클라이언트는 첫 연결 승인 화면에서 창을 고르고, 앱이 시작한 세션은 시작한 창에 묶습니다. 모든 쓰기는 앱의 lease·승인·DocumentThread 경로로 보냅니다. | `[DEV-MAC]` backend 선택 matrix(앱 없음, 검증 실패, 검증 성공) fixture, 다중 창에서 결합 창 외 history 변화 0. | XIX.9.3, XIX.1.7, XIX.1.8, XIX.5.4, XII.2.1 |
| XIX.9.5 | `[ ]` | tool 결과 종류를 Stellar 승인·lease 모델(XIX.3, XIX.4, 아키텍처 31.3~31.5)의 판정으로 정의합니다(ADR-0005 §3 결과 종류). 결과 범주는 XIX.3·XIX.4 판정을 그대로 나타내도록 이 작업에서 정해 보고서에 기록하며, 거부 사유는 Stellar 판정에서만 옵니다. 응답이 없거나 세션이 끝나면 대기 요청은 거부로 닫습니다. 오래 걸리는 호출과 클라이언트 취소는 MCP 명세를 따르고, 대기 상한은 실측과 MCP 명세로 정해 보고서에 기록합니다. | `[DEV-MAC]` 결과 종류 golden, 대기 상한 초과 fixture에서 거부, 세션 종료 후 대기 요청 0. | XIX.9.4, XIX.4.1 |
| XIX.9.6 | `[ ]` | 프로젝트에서 온 문자열을 tool 결과에 넣을 때 아키텍처 37.1 신뢰 경계에 따라 데이터로만 다루고 지시로 해석되지 않게 합니다(ADR-0005 §3). 표현 방식은 MCP 명세를 근거로 정해 보고서에 기록합니다. | `[DEV-MAC]` 지시 형태 문자열을 담은 프로젝트 fixture에서 프로젝트 문자열이 지시로 해석되지 않음(결과 golden 일치). | XIX.9.2 |
| XIX.9.7 | `[ ]` | MCP tool과 IPC 메시지 중 XVI.7 프로젝트 신뢰 상태, XIX.3 권한 상태, 다른 세션의 lease, 에이전트 모드, 앱 설정, terminal profile, 에이전트 변경 출처 표시·되돌리기(XIX.6.7~XIX.6.10), 명령줄 설치 상태(XXIV.5)에 쓰기 수단을 가진 것이 없음을 검사하는 inventory test를 추가합니다. | `[DEV-MAC]` 변경 불가 대상 쓰기 경로 inventory 0. | XIX.9.2, XIX.3.4, XIX.6.10 |
| XIX.9.8 | `[x]` | MCP 클라이언트 등록에 필요한 정보를 제공하되 다른 도구의 설정은 바꾸지 않습니다(ADR-0005 §3). 제공 방식은 MCP 명세를 근거로 정해 보고서에 기록합니다. | `[DEV-MAC]` 연결 정보 golden 일치, 다른 도구 설정 변경 0. | XIX.1.1 |
| XIX.9.9 | `[ ]` | tool 결과 종류와 `stellar`·`stellar-mcp` 종료 코드의 대응을 XXIV.1.2 종료 코드 표에 맞춰 고정합니다. | `[DEV-MAC]` 대응표 golden 일치. | XIX.9.5, XXIV.1.2 |
| XIX.9.10 | `[ ]` | MCP 경로의 자원 사용(호출량, payload 크기, 대기 시간)에 상한을 둡니다. 상한은 아키텍처 6.2~6.4 성능 계약과 Stellar 실측으로 정하고, 구체 한도 목록과 값은 보고서에 기록합니다. 상한을 넘으면 거부하거나 busy로 응답합니다. | `[DEV-MAC]` 한도 초과 fixture별 응답 일치, 한도 목록과 값 보고서 기록. | XIX.9.4 |

**Phase XIX 완료 조건:** 외부 에이전트와 앱이 호스팅한 세션이 MCP를 통해 일반 Command를 사용하고 잠금·승인·완료 정책을 준수합니다. 세션이 끝나도 적용 변경은 출처 표시와 함께 남고 사용자가 변경·세션 단위로 되돌릴 수 있습니다. headless backend는 신뢰된 고정 프로젝트의 읽기만 제공하고 쓰기는 attached backend로만 일어납니다.

---

---

## Phase XX. 로드맵 — PNG·SVG 내보내기

### XX.1 Export Snapshot과 옵션

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XX.1.1 | `[x]` | `ExportRequest`에 View/selection source, TargetProfile, scale, background, output path, format PNG/SVG를 정의합니다. | schema validation. | G8, Phase XI |
| XX.1.2 | `[x]` | export는 immutable SceneSnapshot과 고정 font/assets revision을 캡처해 작업 중 document 변화와 분리합니다. | export 중 edit가 결과에 섞이지 않습니다. | XI.1.5 |
| XX.1.3 | `[ ]` | unsupported target capability와 missing resource를 export 전 진단하고 user confirmation 없이는 진행하지 않습니다. | diagnostic fixture. | IX.10.8, XIII.6.7 |

### XX.2 PNG

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XX.2.1 | `[ ]` | offscreen RenderGraph target을 구현해 지정 scale의 RGBA image를 생성합니다. | size/scale fixture 정확. | XX.1.2, XI.11.1 |
| XX.2.2 | `[ ]` | sRGB와 Display P3 output profile/metadata를 기록합니다. unsupported encoder는 오류로 반환합니다. | profile 검사 통과. | XI.10.5 |
| XX.2.3 | `[ ]` | PNG encoding을 worker에서 실행하고 temp+atomic rename으로 저장합니다. | cancel/failure 시 partial final file 없음. | V.2.5, XX.2.1 |
| XX.2.4 | `[ ]` | 동일 snapshot/options의 decoded pixel output이 양 플랫폼 허용 오차 내 일치하는 golden test를 추가합니다. | comparison 통과. | XX.2.3 |

### XX.3 SVG 벡터 변환

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XX.3.1 | `[x]` | Scene primitive→SVG element mapping table을 정의합니다. Frame/Shape/Path/Image/clip/gradient 기본 지원을 명시합니다. | mapping inventory 통과. | XX.1.2 |
| XX.3.2 | `[ ]` | 모든 Text glyph run을 outline Path로 변환합니다. `<text>` 요소와 font embedding은 생성하지 않습니다. | SVG에 `<text` 문자열 0, 외형 golden 통과. | X.9.6, XI.6.4 |
| XX.3.3 | `[x]` | SVG id는 StableId에서 deterministic safe id로 생성하고 collision을 검사합니다. | same snapshot byte-identical. | III.1.1 |
| XX.3.4 | `[ ]` | SVG로 정확히 표현 가능한 gradient, clip, mask, opacity를 vector로 기록합니다. | validator와 browser render fixture 통과. | XI.10.4 |

### XX.4 부분 래스터화

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XX.4.1 | `[x]` | unsupported effect를 가진 최소 scene subtree와 외부 blend dependency를 분석하는 raster boundary planner를 구현합니다. | nested blur/blend fixture boundary 정확. | XI.4.4, XX.3.1 |
| XX.4.2 | `[ ]` | boundary를 offscreen PNG data로 렌더하고 SVG image element로 삽입합니다. | 위치·scale·alpha fixture 통과. | XX.2.1, XX.4.1 |
| XX.4.3 | `[ ]` | rasterized subtree 목록과 이유를 export report에 기록합니다. | report fixture 정확. | XX.4.2 |
| XX.4.4 | `[ ]` | 한 unsupported effect가 전체 View를 불필요하게 rasterize하지 않는 회귀 테스트를 추가합니다. | vector node count 유지. | XX.4.2 |

### XX.5 Export UI와 성능

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XX.5.1 | `[ ]` | macOS native export command와 progress/cancel을 연결합니다. | UI smoke. | XX.2.3, XX.4.3 |
| XX.5.2 | `[!]` | Windows export command를 연결합니다. | UI smoke. | XX.2.3, XX.4.3 |
| XX.5.3 | `[ ]` | export가 interaction T0~T5 경로를 차단하지 않는 background workload benchmark를 추가합니다. | export 중 interaction p99 gate 유지 또는 명시 resource policy 통과. | XVI.4.4 |

**Phase XX 완료 조건:** PNG와 텍스트 outline·부분 래스터화 SVG가 결정론적으로 생성됩니다.

---

---

## Phase XXI. 로드맵 — HTML/JavaScript·Rust/WASM 플러그인

### XXI.1 Plugin package와 manifest

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXI.1.1 | `[ ]` | `.stellar-plugin` directory package와 `plugin.yaml` schema를 정의합니다. fields: id, version, entry HTML, permissions, host API version, optional WASM modules. | schema fixture. | G8, 공개 API ADR |
| XXI.1.2 | `[ ]` | package path traversal, symlink escape, duplicate id, unsupported version을 검증합니다. | malicious fixture 거부. | XXI.1.1 |
| XXI.1.3 | `[ ]` | plugin signature/registry는 별도 gate로 두고 unsigned local plugin은 개발 mode에서만 허용합니다. | release mode unsigned load 거부. | XXI.1.1 |

### XXI.2 WebView 격리

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXI.2.1 | `[ ]` | macOS WKWebView를 plugin별 별도 data store/process pool 정책으로 생성합니다. | cookie/storage isolation fixture. | XXI.1.2 |
| XXI.2.2 | `[!]` | Windows WebView2 environment/profile을 plugin별 격리합니다. | storage isolation fixture. | XXI.1.2 |
| XXI.2.3 | `[ ]` | plugin HTML은 local package origin만 기본 허용하고 network는 권한 없이는 차단합니다. | CSP/network denial fixture. | XXI.2.1, XXI.2.2 |
| XXI.2.4 | `[ ]` | native app UI와 canvas rendering path에 WebView를 사용하지 않는 inventory test를 유지합니다. | plugin panel 외 WebView 0. | XXI.2.1, XXI.2.2 |

### XXI.3 JS↔WASM 직접 경로

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXI.3.1 | `[ ]` | plugin package WASM module을 WebView JS runtime에서 직접 instantiate하는 SDK loader를 제공합니다. Host IPC를 거치지 않습니다. | JS→WASM add benchmark. | XXI.2.3 |
| XXI.3.2 | `[ ]` | TypedArray와 WebAssembly.Memory view helper를 제공하고 memory growth 후 view 갱신 API를 문서화합니다. | growth fixture stale view 방지. | XXI.3.1 |
| XXI.3.3 | `[ ]` | per-node JS↔WASM 호출 대신 batch buffer API 예제를 SDK에 포함합니다. | 10만 value batch benchmark 기준 통과. | XXI.3.2 |
| XXI.3.4 | `[ ]` | DOM 조작은 JS, 계산은 WASM이라는 책임 경계 lint/example test를 제공합니다. 강제 runtime restriction은 하지 않습니다. | sample plugin 통과. | XXI.3.1 |

### XXI.4 Host RPC와 권한

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXI.4.1 | `[ ]` | Host API를 versioned IDL/WIT 또는 동등 schema로 정의하고 TypeScript/Rust bindings를 생성합니다. | bindings drift 0. | XXI.1.1 |
| XXI.4.2 | `[ ]` | read snapshot, query projection, apply CommandTransaction, Git status, file/network 요청을 별도 RPC namespace로 분리합니다. | API inventory 통과. | XXI.4.1 |
| XXI.4.3 | `[ ]` | RPC는 async request/response와 batch를 지원하고 high-frequency pointer/render loop 호출을 금지합니다. | rate/batch fixture 통과. | XXI.4.2 |
| XXI.4.4 | `[ ]` | Windows 대용량 payload는 Shared Buffer, macOS는 chunked binary reply 또는 승인 방식으로 전송합니다. | 100MB payload memory/copy benchmark. | XXI.4.3 |
| XXI.4.5 | `[ ]` | 파일, network, Git, terminal, clipboard, document apply 권한을 plugin별로 승인·저장합니다. | permission matrix 통과. | XXI.4.2 |
| XXI.4.6 | `[ ]` | plugin command도 일반 Command/Undo/Git WorkingTree 경로를 사용합니다. | user edit와 동일 semantic diff. | Phase VII, XXI.4.2 |

### XXI.5 외부 프로세스 플러그인

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXI.5.1 | `[ ]` | 장기 실행·네이티브 플러그인을 별도 process로 실행하는 IPC contract를 정의합니다. app process에 native library를 dlopen하지 않습니다. | process crash isolation. | XXI.4.1 |
| XXI.5.2 | `[ ]` | process lifecycle, heartbeat, timeout, permission revocation을 구현합니다. | kill/hang fixture cleanup. | XXI.5.1 |
| XXI.5.3 | `[ ]` | 외부 process plugin UI는 동일 HTML/JS WebView panel을 사용하게 합니다. | UI/runtime 분리 fixture. | XXI.2.3, XXI.5.1 |

### XXI.6 실시간 협업 플러그인 확장점

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXI.6.1 | `[ ]` | document snapshot, semantic diff, command apply, session selection/cursor/viewport event API를 plugin SDK에 노출합니다. | sample provider compile. | XXI.4.6 |
| XXI.6.2 | `[ ]` | collaboration metadata는 plugin namespace directory에만 저장하고 core document schema에 넣지 않습니다. | plugin 제거 후 project core open. | XXI.1.1 |
| XXI.6.3 | `[ ]` | sample collaboration plugin은 검증용으로만 작성하고 Stellar 공식 service는 제공하지 않습니다. | two-instance smoke, core metadata 0. | XXI.6.1, XXI.6.2 |

**Phase XXI 완료 조건:** 플러그인이 격리된 HTML/JS UI와 직접 JS↔WASM 계산, 권한형 Host RPC를 사용합니다.

---

---

## Phase XXII. 로드맵 — 단방향 플랫폼 코드 생성

> **순서:** CSS 기반 속성·레이아웃 코드 생성 행(XXII.1.1\~XXII.1.4, XXII.2.1\~XXII.2.4, XXII.3.1\~XXII.3.3, XXII.4.1, XXII.4.2)은 Phase XXX를 기다리지 않습니다. 상태 머신·`MotionSpec`·Context를 다루는 행(XXII.1.5\~XXII.1.7, XXII.2.5, XXII.2.6, XXII.3.4, XXII.4.3)과 모든 adapter를 묶는 XXII.5는 XXX.1.1·XXX.7.4 뒤에 진행합니다(ADR-0025 결정 6).

### XXII.1 중립 Export IR

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXII.1.1 | `[~]` | resolved View를 CSS 기반 속성 record, layout, semantics, state, tokens, target capability가 포함된 `CodegenIR`로 변환합니다(ADR-0025 결정 4.1). CSS 문자열을 다시 parse하지 않고 타입 있는 속성 record(IX.11.6 layout record, IX.11.11 Typography record)를 그대로 쓰며, target capability는 IX.11.19 matrix에서 읽습니다. Context 선언·provision·소비는 XXII.1.6, 상태 머신과 `MotionSpec`은 XXII.1.5가 IR에 더하므로 이 행은 Phase XXX를 기다리지 않습니다. | semantic snapshot fixture, 같은 입력 IR bytes 동일. | G8, IX.10.8, IX.11.6, IX.11.11, IX.11.19 |
| XXII.1.2 | `[ ]` | unsupported/approximated/native mapping diagnostic을 IR node별로 보존합니다. | diagnostic fixture. | XXII.1.1 |
| XXII.1.3 | `[ ]` | CodegenIR은 one-way이며 generated source parser/import API를 만들지 않습니다. | API inventory에 import 없음. | XXII.1.1 |
| XXII.1.4 | `[ ]` | IX.11.19 capability matrix와 IX.11.18 modifier stack 선형화 계획을 코드 생성용으로 확장합니다. 이 행이 더하는 것은 CSS 기반 속성 registry 항목에 대한 Apple·Android profile의 상태와 lowering 규칙뿐입니다. Context·`MotionSpec` 항목은 XXII.1.7이 더하므로 이 행은 Phase XXX를 기다리지 않습니다. Web의 CSS 속성 매핑과 modifier 선형화는 IX.11.18·IX.11.19 결과를 그대로 쓰고 다시 정의하지 않습니다. 비Web 항목 중 1:1이 아닌 것(modifier 적용 순서, 블렌드 합성 색공간 차이, 자유 배치·constraint, Stellar와 대상 플랫폼의 기본값 차이)은 lowering 규칙과 진단을 한 쌍으로 정의하고, IR lowering은 이 matrix만 참조합니다. 상태 어휘는 IX.10.5의 Native·Approximated·Unsupported·Deprecated이며 Apple·Android 상태는 CSS 매핑 상태에서 유도하지 않고 이 행의 lowering 규칙이 정합니다(ADR-0025 결정 1.3, 4.2). | Apple·Android profile의 모든 registry 항목에 상태·규칙이 있는지 완전성 검사, Web 항목 상태가 IX.11.19 matrix와 같음, 규칙별 입력·출력·진단 fixture 통과. | XXII.1.2, IX.11.6, IX.11.18, IX.11.19 |
| XXII.1.5 | `[ ]` | 선언형 상태 머신과 `MotionSpec`을 CodegenIR에 포함합니다. timing function과 reduced motion 대체 동작을 IR에 그대로 보존합니다. | semantic snapshot fixture, 같은 입력 IR bytes 동일. | XXII.1.1, XXX.1.1, IX.5.3 |
| XXII.1.6 | `[ ]` | Context 선언·provision·소비를 CodegenIR에 포함합니다(ADR-0025 결정 4.1). 소비 지점마다 XXX.7.4 해석기가 정한 값의 출처(provision node 또는 key default)와 slot 경계 정보를 IR에 보존하고, 생성 코드가 다시 해석하지 않게 합니다. | Context 중첩·shadowing·slot fixture의 IR 해석이 XXX.7.4 결과와 같음, 같은 입력 IR bytes 동일. | XXII.1.1, XXX.7.4 |
| XXII.1.7 | `[ ]` | XXII.1.4 matrix에 Context·`MotionSpec` 항목의 Web·Apple·Android profile kind별 상태와 lowering 규칙을 더합니다(ADR-0025 결정 4.2). timing function 종류(spring 등)와 Context 전달 방식처럼 1:1이 아닌 항목은 lowering 규칙과 진단을 한 쌍으로 정의합니다. CSS 기반 속성 항목은 바꾸지 않습니다. | Context·MotionSpec 항목에 profile kind별 상태·규칙이 있는지 완전성 검사, XXII.1.4 항목 불변, 규칙별 입력·출력·진단 fixture 통과. | XXII.1.4, XXII.1.5, XXII.1.6 |

### XXII.2 Web Adapter

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXII.2.1 | `[ ]` | Frame/Fragment/Text/Image/Control contract를 HTML semantic structure로 매핑합니다. | golden DOM tree. | XXII.1.1 |
| XXII.2.2 | `[ ]` | IX.11.6 layout record의 CSS 기반 layout 속성을 같은 이름의 CSS 속성(flex, grid, position·inset)으로 출력합니다. 이름·값 형태가 다른 항목은 IX.11.19 matrix와 속성 모델 4장의 변환 규칙으로만 변환하고, 불가능한 constraint는 diagnostic 처리합니다. | fixture output, 변환 규칙 없는 속성의 이름·값이 IR과 같음. | Phase X, XXII.2.1, IX.11.6, IX.11.19 |
| XXII.2.3 | `[ ]` | token을 CSS custom property 또는 generated constants로 내보냅니다. CSS 이름은 IX.11.16 파생 규칙, 등록 방식은 속성 모델 10장(`inherits: true`, `initial-value`는 기본 환경에서 평가한 literal, 값은 `:root`와 `:root`를 감싼 `@media` 블록에서만 선언)을 따릅니다. 환경 `interaction_state`에 따라 달라지는 값은 토큰 선언에서 빼고 XXII.2.6이 Control 상태 selector로 냅니다. | deterministic CSS, 출력 이름이 IX.11.16 golden과 같음, 등록 토큰을 `var()`로 쓰는 자손 요소의 계산값이 `:root` 값과 같고 `@media` 조건 on·off에서 각 조건부 값과 같은 fixture(기대값은 IX.11.8과 같은 고정 버전 브라우저로 생성해 커밋하고 테스트는 브라우저를 실행하지 않음). | Phase VIII, IX.11.8, IX.11.16 |
| XXII.2.4 | `[x]` | accessibility tree를 HTML/ARIA로 매핑합니다. | 정적 접근성 검사 fixture. | IX.8.6 |
| XXII.2.5 | `[ ]` | Context를 Web 출력으로 낮춥니다. CSS로 표현 가능한 스타일 값은 등록된 CSS custom property 상속으로, 그 밖의 값은 생성 코드의 context 구조로 내보냅니다(ADR-0025 결정 4.3). 등록 속성은 토큰과 같은 속성 모델 10장 정책(`inherits: true`)을 씁니다. ContextKey의 CSS 이름은 IX.11.16 파생 규칙으로 만들고, 같은 export scope의 토큰 이름·아래 깊이 이름과 한 이름 공간에서 충돌을 검사·진단합니다. 같은 key를 다시 읽는 provision은 자기 참조 `var()`로 내보내지 않습니다(CSS에서는 cycle이 되어 guaranteed-invalid 값이 됨). 대신 그 key의 provision마다 펼친 트리의 provision 깊이 d(자기를 포함한 조상 provision 수, key default는 0)를 붙인 custom property에 값을 쓰고 key 이름으로 다시 공급하며, 값 식 안의 같은 key 읽기는 깊이 d-1 이름으로 바꿉니다. 같은 provision이 instance 위치마다 다른 깊이를 가져 규칙 하나로 낼 수 없으면 그 key를 생성 코드 context 구조로 내보내고 정보 진단을 냅니다. slot host에서는 Slot이 공개하지 않았고 컴포넌트 안에 provision이 있는 key를 instance 바깥 값으로 다시 공급합니다(ADR-0025 결정 3.11). | deterministic output, 중첩·shadowing·slot fixture의 출력 해석값이 XXX.7.4 결과와 같음, 같은 key 재읽기 2단 중첩 fixture 출력의 자기 참조 `var()` 0건과 해석값 일치, 깊이가 instance마다 다른 fixture의 context 구조 출력과 진단 1건, 토큰과 이름이 겹치는 key fixture의 충돌 진단. | XXII.2.3, XXII.1.6, XXII.1.7, IX.11.16, XXX.7.4 |
| XXII.2.6 | `[ ]` | 상태 머신과 `MotionSpec`을 Web 출력으로 낮춥니다. CSS transition·animation으로 표현 가능한 motion만 CSS로 내보내고, 상태 전환은 생성 코드로 내보냅니다. spring처럼 표현할 수 없는 항목은 근사 또는 미지원 진단을 냅니다. 환경 `interaction_state`에 따라 달라지는 토큰·binding 값은 `:root` 토큰 선언이 아니라(속성 모델 10장) 그 값을 쓰는 Control 요소의 상태 selector(`:hover`·`:focus-visible`·`:active`·`:disabled`)에 해당 속성 선언으로 냅니다. 상태와 selector의 대응은 Control contract 상태(IX.6)로 정합니다. | fixture output, 근사·미지원 진단 fixture 정확, `interaction_state` 조건 토큰을 쓰는 Control의 상태별 계산값 fixture(기대값은 XXII.2.3과 같은 고정 버전 브라우저로 생성해 커밋)와 그 토큰이 `:root` 선언에 없음. | XXII.1.5, XXII.1.7, XXII.2.1, XXII.2.3 |

### XXII.3 Apple Adapter

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXII.3.1 | `[ ]` | Apple TargetProfile에 맞춰 SwiftUI/AppKit 중 지정된 output dialect로 Control contract를 매핑합니다. | golden source. | XXII.1.1 |
| XXII.3.2 | `[ ]` | common layout을 platform layout code로 변환하고 unsupported constraint를 diagnostic합니다. 변환과 진단은 XXII.1.4 matrix 규칙만 씁니다. | compile fixture. | Phase X, XXII.3.1, XXII.1.4 |
| XXII.3.3 | `[x]` | accessibility semantics와 color space를 Apple API 표현으로 내보냅니다. | compile/static fixture. | IX.8.6, III.4.2 |
| XXII.3.4 | `[ ]` | Context를 Apple dialect의 Environment 값으로, 상태 머신과 `MotionSpec`을 dialect의 상태·animation API로 내보냅니다. 표현할 수 없는 항목은 XXII.1.7 규칙에 따라 진단합니다. Environment가 렌더 트리로 전달되므로 slot 삽입 위치에서는 Slot이 공개하지 않은 key를 instance 바깥 값으로 다시 공급합니다(ADR-0025 결정 3.11). | compile/static fixture, Context shadowing·slot 결과가 XXX.7.4와 같음. | XXII.3.1, XXII.1.5, XXII.1.6, XXII.1.7 |

### XXII.4 Android Adapter

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXII.4.1 | `[ ]` | Android TargetProfile에 맞춰 Compose output을 생성합니다. | golden source/compile fixture. | XXII.1.1 |
| XXII.4.2 | `[~]` | layout, control semantics, accessibility를 Compose API로 매핑합니다. layout 변환과 진단은 XXII.1.4 matrix 규칙만 씁니다. | compile/static fixture. | Phase X, IX.8.6, XXII.1.4 |
| XXII.4.3 | `[ ]` | Context를 CompositionLocal로, 상태 머신과 `MotionSpec`을 Compose 상태·animation API로 내보냅니다. 값 변경 빈도에 따라 추적형과 비추적형 local을 고르는 규칙을 고정합니다. 표현할 수 없는 항목은 XXII.1.7 규칙에 따라 진단합니다. CompositionLocal이 렌더 트리로 전달되므로 slot 삽입 위치에서는 Slot이 공개하지 않은 key를 instance 바깥 값으로 다시 공급합니다(ADR-0025 결정 3.11). | compile/static fixture, local 선택 규칙 fixture 정확, Context slot fixture 결과가 XXX.7.4와 같음. | XXII.4.1, XXII.1.5, XXII.1.6, XXII.1.7 |

### XXII.5 결정론·출력 정책

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXII.5.1 | `[ ]` | 생성 파일 순서, 식별자 escape, formatting을 고정합니다. | same input byte-identical. | XXII.2~XXII.4 |
| XXII.5.2 | `[ ]` | output directory를 temp에 생성 후 atomic replace하거나 generated marker 아래만 수정합니다. 사용자 source를 임의 덮어쓰지 않습니다. | user file preservation fixture. | XXII.5.1 |
| XXII.5.3 | `[ ]` | generated code 수정분을 Stellar로 다시 가져오지 않는 문서·UI 경고를 제공합니다. | round-trip command 없음. | XXII.5.2 |

---

---

## Phase XXIII. 로드맵 — Merge Composer

### XXIII.1 Source 선택과 Snapshot

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXIII.1.1 | `[x]` | SourceRef에 branch, commit, tag, worktree, HEAD, Index, WorkingTree를 정의합니다. | source resolution fixture. | Phase XIV |
| XXIII.1.2 | `[x]` | 두 source의 공통 Git history가 있으면 시스템 Git `merge-base`로 base를 구합니다. 없으면 two-way comparison mode로 명시합니다. | related/unrelated fixture. | XXIII.1.1 |
| XXIII.1.3 | `[x]` | Source A/B/Base snapshot을 SnapshotProvider로 materialize하고 원본 worktree를 수정하지 않습니다. | source hash 보존. | XIV.2.6 |

### XXIII.2 Candidate C

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXIII.2.1 | `[ ]` | SemanticMergeEngine 결과를 독립 `CandidateSnapshot C`로 생성합니다. | A/B bytes 미변경. | XIV.10.9, XXIII.1.3 |
| XXIII.2.2 | `[ ]` | object/property별 A, B, auto, custom source provenance를 C에 기록합니다. | provenance fixture. | XXIII.2.1 |
| XXIII.2.3 | `[ ]` | C에서 A/B object·property를 선택하는 resolution command를 구현합니다. | apply/inverse. | XXIII.2.2 |
| XXIII.2.4 | `[ ]` | C full validation과 unresolved conflict count가 0일 때만 write 가능하게 합니다. | invalid candidate 차단. | XXIII.2.3, VI.10.1 |

### XXIII.3 결과 적용

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXIII.3.1 | `[ ]` | 결과 target을 current worktree, existing worktree, new branch+worktree로 정의합니다. | target validation. | XXIII.2.4 |
| XXIII.3.2 | `[ ]` | new branch/worktree 생성은 시스템 Git으로만 실행합니다. | `.git` 직접 수정 0. | XIV.1.3, XXIII.3.1 |
| XXIII.3.3 | `[ ]` | C를 target WorkingTree에 generation save하되 merge commit을 자동 생성하지 않습니다. | Git state expected. | V.5.4, XXIII.3.1 |
| XXIII.3.4 | `[ ]` | source가 변한 경우 apply 전에 revision/OID를 재검사하고 stale candidate를 차단합니다. | source update fixture 실패. | XXIII.1.3, XXIII.3.3 |
---

---

## Phase XXIV. 로드맵 — Headless CLI와 Git Driver

### XXIV.1 CLI 골격

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXIV.1.1 | `[x]` | `stellar-cli`를 app UI와 분리된 binary로 생성하고 core crate만 사용합니다. | headless machine에서 help 실행. | G8 |
| XXIV.1.2 | `[x]` | exit code와 machine-readable JSON output schema를 정의합니다. stdout/stderr 역할을 분리합니다. | success/error fixture. | XXIV.1.1 |
| XXIV.1.3 | `[~]` | `validate`, `semantic-diff`, `semantic-merge`, `migrate` 명령을 구현합니다. | fixture corpus 통과. | Phase XIV, V.8.6 |
| XXIV.1.4 | `[ ]` | Phase XX 이후 `render/export` 명령을 추가합니다. | golden output. | Phase XX |

### XXIV.2 Git custom driver

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXIV.2.1 | `[ ]` | Git custom merge driver가 base/ours/theirs/result path를 받아 Semantic Merge를 실행하는 noninteractive mode를 구현합니다. | conflict 없는 fixture exit 0. | XXIV.1.3 |
| XXIV.2.2 | `[ ]` | unresolved semantic conflict가 있으면 원본을 보존하고 nonzero exit로 앱에서 해결해야 함을 표시합니다. | unresolved fixture exit code 정확. | XXIV.2.1 |
| XXIV.2.3 | `[ ]` | `.gitattributes`/Git config 등록은 사용자 승인 명령으로만 수행합니다. 전역 config를 자동 변경하지 않습니다. | no-consent config unchanged. | XXIV.2.1 |

### XXIV.3 CI 도구

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXIV.3.1 | `[~]` | lockfile resolve check, project validation, migration-required check를 CI mode로 제공합니다. | invalid project CI failure. | XXIV.1.3 |
| XXIV.3.2 | `[x]` | semantic diff summary를 artifact로 생성하되 project text/name/path redaction option을 기본으로 합니다. | privacy fixture. | XIV.3.8, XXIV.1.2 |
| XXIV.3.3 | `[ ]` | performance replay CLI를 reference runner에서 실행할 수 있게 합니다. | result schema identical. | XVI.2.2 |

### XXIV.4 L1 확장과 실행 이름

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXIV.4.1 | `[x]` | 사용자가 부르는 명령 이름을 `stellar`로 고정합니다(2026-11-14 제품 소유자 결정). `stellar-cli` package에 `[[bin]] name = "stellar"`를 추가하고 package·library 이름 `stellar-cli`는 유지합니다. help·usage·stderr 진단 접두어를 `stellar`로 바꾸고, XXIV.1.2 JSON 결과 schema 식별자와 fixture는 바꾸지 않습니다. | `[DEV-MAC]` `cargo build -p stellar-cli` 산출 실행 파일 이름 `stellar`, help snapshot이 `stellar` 사용, XXIV.1.2 JSON fixture 무변경, `cargo test -p stellar-cli` 통과. | XXIV.1.2 |
| XXIV.4.2 | `[~]` | headless 조회·편집 공용 library crate `stellar-ops`(`crates/ops`)를 추가하고 crate graph 정책과 문서에 등록합니다. 의존은 project, document, commands, history, storage, semantic-diff, semantic-merge, diagnostics만 허용하고 local-ipc, agent-protocol, mcp-core, agent-host, ffi는 금지합니다. 앱 없이 프로젝트를 열어 revision을 포함한 읽기 전용 snapshot을 반환합니다. | `[DEV-MAC]` `cargo xtask validate-crate-graph` 통과, 앱 미실행 snapshot fixture revision 일치. | XXIV.1.2, VII.1.6 |
| XXIV.4.3 | `[~]` | `stellar inspect`를 확장하고 `stellar query`를 추가합니다. 프로젝트 구조, 문서 트리(cursor 분할), 노드 속성, 토큰, 컴포넌트를 revision과 함께 JSON으로 출력합니다. XXIV.1.2의 기존 inspect 결과 필드는 호환을 유지합니다. 앱 없이 동작하며 파일을 쓰지 않습니다. `stellar-cli`의 직접 의존을 `stellar-ops`·diagnostics로 줄이고(ADR-0005 §8), crate graph 정책의 `stellar-cli` 허용 의존도 ops·diagnostics로 갱신합니다. | `[DEV-MAC]` JSON golden 일치, 20만 node 문서 page 분할 통과, 실행 전후 프로젝트 파일 hash 동일, 기존 inspect fixture 통과, `cargo xtask validate-crate-graph` 통과, `stellar-cli` 직접 의존이 ops·diagnostics뿐. | XXIV.4.1, XXIV.4.2 |
| XXIV.4.4 | `[~]` | `stellar apply`를 추가합니다. 파일이나 표준 입력으로 CommandTransaction JSON과 base revision을 받아 closed enum으로 검증한 뒤 세대 전환 저장으로 원자적으로 기록하고, generation metadata에 writer 종류 headless CLI를 남깁니다. `--dry-run`은 semantic diff만 출력합니다. storage는 보호 경로이므로 같은 변경에 ADR을 갱신합니다. | `[DEV-MAC]` dry-run 전후 revision 동일, base revision 불일치와 unknown command 거부, 적용 후 reopen semantic snapshot 일치, writer 종류 기록 확인. | XXIV.4.2, VII.1.6, XIV.3.8 |
| XXIV.4.5 | `[~]` | `stellar-ops` 쓰기 경로에 교차 프로세스 저장 잠금을 추가합니다. 잠금은 프로젝트·Git 밖의 OS 사용자 데이터에 두고, 잠금을 잡은 뒤 generation을 비교하고 교체하며, 잠금 사용 중이면 일시적 실패 종료 코드로 끝냅니다. Git merge driver 모드는 잠금 대상에서 제외합니다. 사용 OS API는 Apple 공식 문서를 근거로 보고서에 기록합니다. | `[DEV-MAC]` 동시 쓰기 fixture 손실 0, 잠금 경합 시 exit 75, merge driver 모드 잠금 미사용. | XXIV.4.4, IV.2.3 |
| XXIV.4.6 | `[ ]` | 앱이 writer 종류가 앱 창이 아닌 generation을 감지하면 XII.7.5 두 선택지 대신 XIII.3.4 semantic reconciliation으로 처리합니다. 변경이 에이전트 lease와 겹치면 XIX.7.2 절차를 따릅니다. | `[DEV-MAC]` CLI writer generation fixture에서 두 선택지 alert 0, reconciliation 결과 snapshot 일치. | XXIV.4.5, XIII.3.4, XII.7.5 |
| XXIV.4.7 | `[ ]` | `stellar`에 AI·MCP·모델·에이전트 관련 명령과 해당 crate로의 직접·전이 의존이 없음을 검사하는 inventory test를 추가합니다. | `[DEV-MAC]` help 명령 inventory와 전이 도달 금지 검사 통과. | XXIV.4.3, XXIV.4.4, XXIX.1.1 |

### XXIV.5 macOS 번들 배포와 `stellar` 명령 설치

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXIV.5.1 | `[ ]` | macOS 앱 번들에 `stellar`를 helper로 넣습니다. 릴리스 서명 스크립트가 helper와 앱을 Apple 공식 코드 서명·공증 문서에 따라 서명하도록 바꾸고(helper에 앱 entitlement 없음), "중첩 서명 대상 없음" 전제를 고치며 helper에도 arm64·서명 검사를 적용합니다. 번들 안 배치와 구체 서명 옵션은 보고서에 기록합니다(ADR-0005 §7). | `[DEV-MAC]` `codesign --verify --strict` 통과, `[CI-MAC]` 공증 후 clean runner에서 helper `stellar --version` 실행. | XVI.9.4, XXIV.4.1 |
| XXIV.5.2 | `[ ]` | 앱 없는 macOS headless·CI 머신용 `stellar` 단독 서명·공증 배포물을 만듭니다. `stellar-mcp`는 넣지 않습니다. | `[CI-MAC]` 앱 미설치 runner에서 `stellar validate` fixture 실행 성공. | XXIV.5.1 |
| XXIV.5.3 | `[~]` | 명령줄 설치 위치 결정을 구현합니다. 위치와 근거는 §37 보안 경계(최소 권한)와 Apple·각 셸 공식 문서를 근거로 정해 보고서에 기록하고(ADR-0005 §7), 계산은 UI·Document 스레드 밖에서 합니다. | `[DEV-MAC]` 위치 결정 fixture가 보고서 정의와 일치, UI 스레드 호출 0. | XXIV.5.1 |
| XXIV.5.4 | `[~]` | `stellar` 명령 설치·제거 로직을 구현합니다(2026-11-14 제품 소유자 결정). 사용자 동의 뒤에만 파일을 바꾸고, PATH에서 `stellar`가 Stellar 자기 링크가 아닌 다른 프로그램으로 해석되면(설치 위치에 있거나 PATH 앞쪽에 있는 경우) 덮어쓰지 않고 `stellar-cli` 이름으로 설치하고, `stellar-cli`도 다른 프로그램이 쓰고 있으면 설치하지 않고 알립니다(ADR-0005 §2). 그 밖의 설치 방식과 안전 조건은 §37과 Apple·각 셸 공식 문서를 근거로 정해 보고서에 기록하며, 설치 상태는 OS 사용자 데이터에만 둡니다(ADR-0005 §7). | `[DEV-MAC]` 설치·제거 fixture 통과, 동의 없는 파일 변경 0, 다른 프로그램의 `stellar`가 설치 위치나 PATH 앞쪽에 있는 fixture에서 덮어쓰기 0·`stellar-cli` 설치, `stellar-cli`도 다른 프로그램인 fixture에서 설치 0·알림, Stellar 자기 링크만 있는 fixture에서 `stellar` 유지. | XXIV.5.3 |
| XXIV.5.5 | `[~]` | 설치 상태 확인을 구현합니다. 확인 시점·방법과 확인 결과의 처리는 Apple 공식 문서와 각 셸 공식 문서를 근거로 정해 보고서에 기록하며, 처리는 XXIV.5.4와 같은 조건을 지킵니다. 확인은 UI 스레드 밖에서 합니다. | `[DEV-MAC]` 상태 fixture 판정이 보고서 정의와 일치, 동의 없는 파일 변경 0. | XXIV.5.4 |
| XXIV.5.6 | `[~]` | macOS 첫 실행 명령줄 설치 화면을 구현합니다(2026-11-14 제품 소유자 결정). `stellar` helper를 포함한 빌드를 처음 실행하면 설치 화면을 제공하고, 사용자가 명시적으로 동의하기 전에는 파일을 만들거나 바꾸지 않습니다. 설치를 되돌리는 진입점도 제공하고, 실제로 설치한 명령 이름(`stellar` 또는 `stellar-cli`)을 보여 줍니다. 화면 구성과 진입점 배치는 HIG를 근거로 정해 보고서에 기록합니다. | `[DEV-MAC]` 첫 실행 설치 화면 표시, 동의 없는 파일 변경 0, 되돌리기 후 설치 항목 0, 키보드·VoiceOver smoke. | XXIV.5.5, XII.9.1, XII.6.5 |

---

## Phase XXV. 로드맵 — 세션 간 Undo 복원과 무결성 진단 패널

### XXV.1 영속 History Journal

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXV.1.1 | `[x]` | session-only HistoryEntry의 stable journal schema와 version을 정의합니다. | round-trip. | Phase VII, V.8.1 |
| XXV.1.2 | `[x]` | completed transaction과 inverse를 project-local ignored path 또는 OS app data 정책에 따라 기록합니다. Git 추적 금지 검사를 추가합니다. | `git status` 변화 없음. | XXV.1.1 |
| XXV.1.3 | `[x]` | 앱 재실행 후 current generation과 journal base가 같을 때만 history를 복원합니다. | mismatched base 봉인. | XXV.1.2 |
| XXV.1.4 | `[x]` | checkout/merge/rebase 등 기준점 변경 시 journal을 별도 sealed session으로 이동합니다. | history 혼합 없음. | XIV.2.2, XXV.1.3 |
| XXV.1.5 | `[x]` | retention size/age policy와 compaction을 구현합니다. | bounded storage. | XXV.1.2 |

### XXV.2 무결성 진단 수집

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXV.2.1 | `[ ]` | OrphanedBinding, missing token/font/asset/library, invalid accessibility, target unsupported, migration warning을 공통 DiagnosticIndex로 집계합니다. | fixture count 정확. | VIII.8.1, XIII.6.7, XV.5.5, IX.8.5 |
| XXV.2.2 | `[ ]` | diagnostic에 severity, source StableId, property path, remediation action ID를 포함합니다. | schema round-trip. | XXV.2.1 |
| XXV.2.3 | `[ ]` | document 변경 시 영향받는 diagnostic만 증분 재검사합니다. | unrelated edit recompute 0. | XXV.2.1, VII.1.4 |

### XXV.3 진단 UI

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXV.3.1 | `[ ]` | FFI pagination/filter API로 DiagnosticIndex를 노출합니다. | 100만 diagnostic pagination. | XXV.2.2 |
| XXV.3.2 | `[ ]` | macOS/Windows native panel에서 종류·severity·document로 필터하고 source로 이동합니다. | UI smoke. | XXV.3.1 |
| XXV.3.3 | `[ ]` | remediation action은 일반 CommandTransaction 또는 명시 navigation만 실행합니다. 임의 script는 금지합니다. | action inventory 통과. | XXV.2.2, Phase VII |

---

## Phase XXVI. 로드맵 — 고급 Git UI와 Worktree 관리

### XXVI.1 Branch·Fetch·Pull·Merge·Rebase

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXVI.1.1 | `[x]` | branch list/switch/create/delete UI API를 시스템 Git 명령에 연결합니다. | local repo fixture. | Phase XIV |
| XXVI.1.2 | `[x]` | fetch UI와 progress를 연결합니다. | local remote fixture. | XXVI.1.1 |
| XXVI.1.3 | `[x]` | pull strategy를 사용자가 merge/rebase/ff-only 중 선택하게 하고 시스템 config 기본을 임의 변경하지 않습니다. | strategy fixture. | XXVI.1.2 |
| XXVI.1.4 | `[ ]` | merge/rebase 시작과 abort/continue UI를 Git 상태 기반으로 연결합니다. continue는 Conflict UI 해결 후 사용자가 명시 실행합니다. | scenario fixture. | XIV.12.5 |

### XXVI.2 Stash·Cherry-pick·Revert·Reset·Reflog

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXVI.2.1 | `[x]` | stash list/create/apply/pop/drop UI를 시스템 Git으로 연결합니다. | fixture. | Phase XIV |
| XXVI.2.2 | `[ ]` | cherry-pick/revert UI와 conflict 상태 연결을 구현합니다. | fixture. | XIV.12.4 |
| XXVI.2.3 | `[x]` | reset/restore는 destructive level별 확인을 요구하고 command preview를 표시합니다. | hard reset confirm 없이는 실행 0. | Phase XIV |
| XXVI.2.4 | `[x]` | reflog 조회와 선택 ref 복구 command를 제공합니다. | fixture. | XIV.1.3 |

### XXVI.3 Worktree UI

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXVI.3.1 | `[x]` | `git worktree list --porcelain` parser를 구현합니다. | main/linked/locked/prunable fixture. | XIV.1.3 |
| XXVI.3.2 | `[x]` | add/remove/move/lock/unlock/prune/repair를 시스템 Git command로 연결합니다. | command matrix fixture. | XXVI.3.1 |
| XXVI.3.3 | `[ ]` | worktree별 Stellar project root 검증 후 별도 창에서 엽니다. | same common repo, independent index fixture. | XIV.14.5, XII.2.1 |
| XXVI.3.4 | `[x]` | untracked file이 있는 worktree remove를 Git이 거부하면 결과를 그대로 표시하고 force를 기본 제공하지 않습니다. | fixture. | XXVI.3.2 |

### XXVI.4 기타 Git 기능

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXVI.4.1 | `[x]` | tags, notes, bisect, sparse-checkout, submodule 상태의 read UI를 우선 구현합니다. | fixture. | Phase XIV |
| XXVI.4.2 | `[x]` | 각 기능의 write UI는 별도 task/승인 후 추가하고 시스템 Git에서 가능하다는 이유만으로 자동 노출하지 않습니다. | feature flags default off. | XXVI.4.1 |

---

## Phase XXVII. 로드맵 — Windows ARM64

### XXVII.1 빌드와 ABI

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXVII.1.1 | `[!]` | Rust target과 Windows solution에 ARM64 configuration을 추가합니다. | ARM64 compile. | G8 |
| XXVII.1.2 | `[!]` | FFI type size/alignment와 ABI version을 x64/ARM64에서 비교합니다. | public ABI layout 동일 또는 명시 version bump. | II.4.6 |
| XXVII.1.3 | `[!]` | D3D12 adapter/device code의 architecture assumption을 제거합니다. | ARM64 device smoke. | XI.9.7 |

### XXVII.2 의존성과 패키징

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXVII.2.1 | `[!]` | 모든 native dependency의 ARM64 artifact를 검증하고 미지원 dependency를 대체하지 말고 blocker report로 남깁니다. | dependency audit. | XXVII.1.1 |
| XXVII.2.2 | `[!]` | ARM64 MSIX package와 update channel artifact를 추가합니다. | clean device install. | XXVII.1.3, XVI.10.4 |
| XXVII.2.3 | `[!]` | ARM64 기준 성능 장치를 별도로 지정하고 x64 기준과 다른 보장 여부를 문서화합니다. | performance report. | XVI.4.6 |

---

## Phase XXVIII. 로드맵 — 파일 포맷 공개 명세

### XXVIII.1 명세 안정화 준비

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXVIII.1.1 | `[ ]` | 공개 전 최소 두 개 major app version에서 format/migration 사용 데이터를 검토하고 변경 빈도 보고서를 작성합니다. | stability report. | G8 이후 누적 |
| XXVIII.1.2 | `[x]` | canonical encoding, chunk envelope, generation manifest, YAML, StableId, unknown field, migration rules를 문서화합니다. | internal implementation과 spec fixture 일치. | Phase V |
| XXVIII.1.3 | `[x]` | semantic model과 Git diff/merge 규칙을 별도 normative section으로 작성합니다. | merge corpus와 spec mapping 완전. | Phase XIV |
| XXVIII.1.4 | `[x]` | 보안 한도, 최대 크기, parser 요구사항, malformed input behavior를 명시합니다. | fuzz corpus category 전부 문서 연결. | XVI.6 |

### XXVIII.2 Conformance Suite

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXVIII.2.1 | `[~]` | valid/invalid binary/YAML golden corpus를 라이선스와 함께 공개 가능한 형태로 정리합니다. project content는 synthetic만 사용합니다. | corpus manifest 완전. | XXVIII.1.2 |
| XXVIII.2.2 | `[ ]` | parser/writer conformance runner와 expected result schema를 제공합니다. | official implementation 통과. | XXVIII.2.1 |
| XXVIII.2.3 | `[ ]` | semantic diff/merge conformance corpus와 runner를 제공합니다. | official implementation 통과. | XXVIII.1.3 |
| XXVIII.2.4 | `[x]` | version compatibility matrix와 deprecation policy를 공개합니다. | historical fixture 검증. | V.8.6 |

### XXVIII.3 첫 공개 버전

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXVIII.3.1 | `[!]` | 공개 format version을 동결하고 이후 incompatible change는 major format bump와 migration을 요구하도록 governance를 적용합니다. IX.12.1이 예약한 Component Context 번호는 동결 명세에 예약 항목으로 표시합니다. | breaking fixture without bump CI 실패, 동결 명세의 예약 번호가 IX.12.1 golden과 같음. | XXVIII.2.4, IX.12.1 |
| XXVIII.3.2 | `[ ]` | 공개 문서, reference parser API 범위, issue process를 게시합니다. reference parser는 headless CLI와 분리할 수 있습니다. | clean-room sample parser가 최소 fixture 읽기 성공. | XXVIII.3.1 |
| XXVIII.3.3 | `[ ]` | 공개 명세가 있어도 Stellar 제품의 지원 보장은 minimum reader/writer version 규칙을 따른다는 호환성 문서를 제공합니다. | version matrix 일치. | XXVIII.3.1 |

---

## Phase XXIX. 로드맵 — 3계층 에이전트 도구와 내장 에이전트 패널

### XXIX.1 계획·정책 검증 도구

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXIX.1.1 | `[~]` | crate graph 검사기를 확장합니다. `[dependencies]` 외에 `[target.*.dependencies]`, `[build-dependencies]`, `[dev-dependencies]`의 내부 의존을 읽고, 지정한 crate 쌍 사이의 전이 도달을 금지하는 규칙(예: agent-host에서 commands·document 도달 금지)을 추가합니다. crate 의존 그래프 문서도 함께 갱신합니다. | `[DEV-MAC]` 전이 위반·target 의존 위반 fixture 거부, 현재 workspace 통과. | 없음 |
| XXIX.1.2 | `[x]` | XXIV.4, XIX.9, XXIX에 필요한 외부 crate(JSON 직렬화, JSON-RPC 프레이밍, IPC 상대 인증용 macOS binding, 필요하면 표시 서식 해석)를 `config/dependencies.yaml` 승인 항목으로 등록하고 `validate-dependencies` 검사 범위를 workspace 전체 직접 의존으로 넓힙니다. | `[DEV-MAC]` 미등록 의존 추가 fixture 실패, 현재 workspace 통과. | 없음 |
| XXIX.1.3 | `[x]` | `cargo xtask sync-plan`에 본문 ID 참조 검사를 추가합니다. 구현 지시·검증 셀과 Phase 완료 조건에 나온 WBS ID 형식 토큰이 계획서에 없으면 보고합니다. 범위 표기(`A~B`)는 양 끝 ID 존재만 검사하고, Roman Phase가 아닌 표기와 계획서 밖 문서는 검사하지 않습니다. | `[DEV-MAC]` 삭제된 ID를 참조하는 fixture 보고, 현재 계획서 종료 0. | 없음 |
| XXIX.1.4 | `[x]` | ledger 허용 상태에서 `cancelled`를 제거합니다(1.1 상태 표기, 2026-11-14 제품 소유자 결정). `TaskState`의 cancelled variant, collection 검증 허용 목록, evidence 검증 분기를 삭제하고 `cancelled` record는 unknown status로 거부합니다. 완료 행 I.2.1·I.2.7은 고치지 않습니다. | `[DEV-MAC]` cancelled record fixture 거부, 현재 ledger 통과, xtask fmt·clippy·test 통과. | 없음 |

### XXIX.2 구독 CLI 호스트 코어와 출력 디코드

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXIX.2.1 | `[ ]` | `stellar-agent-host` crate(`crates/agent-host`)를 추가하고 crate graph에 등록합니다. 의존은 core-types, agent-protocol, local-ipc, diagnostics만 허용하고 document, commands, history, storage, ops로의 전이 도달을 금지합니다. 모델 제공자 API client, API key 필드, 자격 증명 저장, 모델 라우팅, 사용량 계량 타입이 없음을 inventory test로 고정합니다. | `[DEV-MAC]` `cargo xtask validate-crate-graph` 통과, 금지 타입 inventory 0. | XIX.9.1, XXIX.1.1 |
| XXIX.2.2 | `[ ]` | 구독 CLI 실행 경로를 네트워크와 실제 계정 없이 테스트할 수 있는 테스트 대역을 만듭니다. XXIX.2.4~XXIX.2.13과 XXIX.6의 시나리오를 재현할 수 있어야 하며, 방식은 이 작업에서 정해 보고서에 기록합니다. | `[DEV-MAC]` 테스트 대역 self-test 통과, 네트워크와 실제 계정 사용 0. | XXIX.2.1 |
| XXIX.2.3 | `[ ]` | 구독 CLI 실행 대상 지정을 구현합니다. 사용자가 확인한, 수정하지 않은 공식 CLI만 실행 대상으로 삼습니다(ADR-0005 §4.3 실행 대상 확인). 신원 확인 방법은 Apple 공식 문서와 각 CLI 공식 문서를 근거로 정해 보고서에 기록합니다. Stellar는 공급자 CLI를 설치·업데이트·수정하지 않습니다. | `[DEV-MAC]` 신원 확인 실패 fixture에서 실행 0, 확인 중 네트워크 호출 0, CLI 실행 파일 쓰기 0. | XXIX.2.2 |
| XXIX.2.4 | `[ ]` | macOS 실행기를 구현합니다. 실행은 ADR-0005 §4.3 두 불변식(문서 변경은 Stellar MCP tool로만, 세션은 프로젝트 working tree와 사용자의 CLI·도구 설정에 흔적 없음)을 지키고, Stellar는 자격 증명을 다루거나 들여다보지 않습니다(아키텍처 31.1, 37.3). UI가 느려도 출력 수신을 멈추지 않습니다(ADR-0005 §6). CLI 프로세스 환경은 대화형 터미널과 달리 ADR-0023 결정 9의 허용 목록으로 만듭니다. 빈 환경에서 시작해 launch profile(XXIX.2.6)의 환경 변수 이름 목록에 있는 이름만 앱 프로세스 환경에서 전달합니다. 그 밖의 실행 세부와 timeout 값은 실측, Apple 공식 문서, 각 CLI 공식 문서를 근거로 정해 보고서에 기록합니다. | `[DEV-MAC]` 테스트 대역 세션에서 Stellar MCP tool 외 프로젝트 변경 0, 세션 전후 프로젝트·사용자 설정 hash 동일, 세션 종료 후 실행 흔적 0, timeout·취소 fixture 통과, UI 지연 fixture에서 출력 수신 정지 0, 허용 목록 밖 환경 변수 전달 0. | XXIX.2.2, XVIII.2.4, XVI.7.5 |
| XXIX.2.5 | `[ ]` | 구독 CLI 연결 상태를 각 CLI 공식 문서가 제공하는 방법으로만 판정합니다. CLI 자격 증명 저장 위치에는 접근하지 않습니다. 판정은 UI·Document 스레드 밖에서 하며, 판정할 수 없으면 사용 불가로 처리합니다(fail closed). CLI가 보고한 계정 식별 정보는 저장·로그·진단에 넣지 않습니다(XVI.8). 상태 정의와 판정 시점은 보고서에 기록합니다(ADR-0005 §4.1). | `[DEV-MAC]` 테스트 대역 상태 fixture 판정 일치, 판정 불가 fixture에서 사용 불가, CLI 자격 증명 저장 위치 접근 inventory 0, 진단·설정 저장소에 계정 식별 문자열 0. | XXIX.2.3, XXIX.2.4 |
| XXIX.2.6 | `[ ]` | CLI 종류와 지원 버전 범위별 launch profile을 Stellar 소유 fixture로 관리합니다. 각 profile은 각 CLI 공식 문서의 비대화형 인터페이스와 구조화 출력 형식만 쓰고, 실행 설정, decoder profile 버전, CLI 프로세스에 전달할 환경 변수 이름 목록(ADR-0023 결정 9), 공급자 약관·공식 문서 검토 일자와 결과를 담습니다. 환경 변수 목록에는 이름만 적고 값은 넣지 않으며, 각 이름을 넣은 근거(CLI 공식 문서의 해당 항목)를 profile별 보고서에 기록합니다. profile은 ADR-0005 §4.3 두 불변식을 지키며, 각 불변식을 지키는 공식 설정과 그 밖의 실행 세부는 각 CLI 공식 문서와 Apple 공식 문서를 근거로 profile별 보고서에 기록합니다. 약관 검토상 허용이 명확하지 않은 CLI와 지원 범위 밖 버전은 실행하지 않습니다. | `[DEV-MAC]` profile별 테스트 대역 세션에서 Stellar MCP tool 외 프로젝트 변경 0, 세션 전후 프로젝트·사용자 설정 hash 동일, 비활성 profile과 지원 범위 밖 버전 실행 거부, 모든 profile에 환경 변수 이름 목록 존재와 값 0, 테스트 대역 세션의 child 환경 변수 이름 집합이 그 profile 목록의 부분집합, 목록의 이름마다 보고서 근거 존재. | XXIX.2.4, XXIX.2.5, XIX.1.8 |
| XXIX.2.7 | `[ ]` | 세션 실행 전 확인을 구현합니다. ADR-0005 §4.3 실행 대상 확인과 XXIX.2.5 연결 상태를 확인하고, 확인되지 않으면 실행하지 않습니다(fail closed). | `[DEV-MAC]` 확인 실패 fixture에서 실행 0, 정상 fixture에서 실행 1. | XXIX.2.6 |
| XXIX.2.8 | `[ ]` | AgentSession 상태 기계를 구현합니다. 상태와 전이는 XIX.6.4 종료 절차와 아키텍처 31.5 수명 규칙을 표현하도록 이 작업에서 정해 보고서에 기록합니다. 앱이 시작한 세션의 lease와 권한은 AgentSession이 소유하고, XIX.1.8로 결합된 bridge가 다시 연결되면 같은 세션에 다시 묶으며, 재연결 유예를 둡니다. 세션은 사용자가 명시적으로 시작할 때만 만들고 사용자 동작 없이 시작·재개·재전송하지 않으며, CLI가 보고한 사용 한도·인증 오류는 그대로 전달하고 세션을 멈춥니다. 세션 중지, CLI 프로세스 종료, 유예 초과, 신뢰 철회 시 XIX.6.4 절차(자동 rollback 없음)로 끝내고 ADR-0005 §4.3 불변식에 따라 실행 흔적을 정리합니다. | `[DEV-MAC]` 상태 전이 matrix가 보고서 정의와 일치, 중지 후 적용 batch 유지·출처 표시 존재·lease 0·대기 제안 0, 재연결 fixture lease 유지, 유예 초과 fixture 중단됨 history entry 확정, 한도 오류 fixture 사용자 동작 없는 재전송 0. | XXIX.2.7, XIX.6.4 |
| XXIX.2.9 | `[ ]` | 출력 디코더 테스트 fixture를 launch profile마다 둡니다. 원문 입력과 기대 `AgentChatItem` 목록(golden)으로 네트워크와 실제 계정 없이 결정적으로 실행하며(아키텍처 38), fixture에는 XVI.8 금지 정보를 넣지 않습니다. 정상 출력, 해석할 수 없는 출력, 입력 경계에 걸친 데이터를 포함하고, profile별 사례는 각 CLI 공식 문서를 근거로 보고서에 기록합니다. | `[DEV-MAC]` fixture 목록 golden, 기대 결과 파일 존재, fixture privacy scan(XVI.8) 0건. | XXIX.2.2, XXIX.2.6 |
| XXIX.2.10 | `[ ]` | 채팅 보기 모델 `AgentChatItem` closed enum을 `stellar-agent-host`에 정의합니다. variant는 ADR-0005 §4.4 범위 안에서 이 작업에서 정해 보고서에 기록합니다. 이 타입은 CommandTransaction이나 document, commands, history, storage, ops 타입을 담지 않고 그 타입이나 승인 응답으로의 변환 함수를 갖지 않습니다. item당 표시 텍스트와 세션당 item 수에 상한을 둡니다. | `[DEV-MAC]` variant별 golden 직렬화 test 통과, 금지 타입·변환 함수 inventory 0. | XXIX.2.1 |
| XXIX.2.11 | `[ ]` | 표시 전용 출력 디코더를 `stellar-agent-host` 계층 안에 격리해 구현합니다(2026-11-14 제품 소유자 결정). launch profile마다 버전이 붙은 decoder profile로 각 CLI 공식 문서의 구조화 출력만 해석하고, 디코드는 실패하면 닫힙니다(fail closed, 2026-11-14 제품 소유자 결정). 모듈 구성, 입출력, 실패 판정 기준은 보고서에 기록합니다. 같은 원문은 항상 같은 item 목록으로 디코드됩니다. | `[DEV-MAC]` XXIX.2.9 fixture 디코드 golden 일치, 해석 불가 fixture별 실패 판정 1, 반복 실행 결과 동일, agent-host 밖의 공급자 출력 해석 코드 0. | XXIX.2.9, XXIX.2.10, XXIX.1.1 |
| XXIX.2.12 | `[ ]` | 원문 보기 버퍼를 구현합니다. CLI 출력을 수신한 그대로 담는 세션별 메모리 상한 버퍼이며, 상한을 넘으면 잘린 사실을 표시합니다. 제어 문자는 해석하지 않고 PTY·terminal emulation은 쓰지 않습니다. 디코더 성공·실패와 관계없이 보존하고 세션 메모리에만 둡니다. 상한 값은 실측으로 정합니다. | `[DEV-MAC]` 제어 문자 주입 fixture 무력화, 상한 초과 시 메모리 상한 유지와 잘림 표시, 진단·recovery snapshot에 내용 0. | XXIX.2.4 |
| XXIX.2.13 | `[ ]` | 디코드 실패 판정을 세션 처리에 연결합니다. 실패 뒤 처리는 2026-11-14 결정(fail closed)과 XIX.6.4 절차(자동 rollback 없음)를 지키도록 이 작업에서 정해 보고서에 기록합니다. 디코드 실패 판정은 권한을 판단하지 않으며, 문서 접근 허용 여부는 앱의 권한·승인·lease 판정만 정합니다. | `[DEV-MAC]` 해석 불가 출력 fixture에서 처리 결과가 보고서 정의와 일치, 적용 변경 자동 삭제 0, 권한·lease 확대 0. | XXIX.2.11, XXIX.2.8 |

### XXIX.3 FFI·승인·스레드·채팅 조립

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXIX.3.1 | `[ ]` | FFI를 확장합니다. 에이전트 세션 생성·메시지 전송·중지, 승인 응답, 채팅 item과 원문 버퍼 증분 event, 출처 색인 조회, 변경·세션 되돌리기와 표시 정리 함수, 버전이 매겨진 에이전트 event message를 추가하고 셸은 II.4.5 pull 방식 event 경로로 받습니다. stellar-ffi 허용 의존은 ADR-0013대로 core-types, runtime만 두고, 에이전트 기능은 runtime을 거쳐 도달합니다. crate graph 정책의 stellar-runtime 허용 의존에 agent-host, agent-protocol, ops와 에이전트 편집 구현 crate를 추가하고, runtime이 agent-protocol host trait를 구현합니다(ADR-0005 §8). FFI는 보호 경로이므로 같은 변경에 ADR을 갱신합니다. | `[DEV-MAC]` 생성 header drift 없음, FFI smoke에서 세션 handle 누수 0, `cargo xtask validate-crate-graph` 통과, stellar-ffi 직접 의존이 core-types·runtime뿐. | XXIX.2.8, XXIX.2.10, XXIX.2.12, XIX.6.10 |
| XXIX.3.2 | `[ ]` | 승인 요청을 ProjectIdentity 단위로 코어에 보관합니다. 첫 응답 하나만 유효하고, 창 닫힘, 응답 기한 초과, 세션 종료 시 거부가 기본입니다. 요청에는 Stellar 코어가 만든 semantic diff·validation·렌더 프리뷰를 승인 근거로 담습니다(XIX.5.3). | `[DEV-MAC]` 중복 응답 무시, 창 닫힘·기한 초과·세션 종료 거부 fixture, 동시 요청 순서 유지. | XXIX.3.1, XIX.4.1, XIX.3.4 |
| XXIX.3.3 | `[ ]` | 에이전트 모드를 XIX.3.1 Read·Propose·Apply 권한 조합의 상한으로 매핑합니다(ADR-0005 §4.5). 모드는 권한을 부여하지 않고 상한으로만 작동하며, ADR-0005 Security 결정에 따라 사용자 UI에서만 바꿉니다. 내장 세션 카탈로그에서 Terminal과 FileDelete 계열 tool을 제외하고 GitCommit·GitPush는 개별 확인으로만 허용합니다. | `[DEV-MAC]` 모드별 permission matrix 통과, 내장 세션 tools/list의 terminal·파일 삭제 계열 0, tool 호출로 모드 변경 시도 거부. | XXIX.3.2, XIX.9.7 |
| XXIX.3.4 | `[ ]` | 앱에 Agent Host·IPC I/O 스레드를 둡니다. 이 스레드는 문서를 쓰지 않습니다. 검증된 Command를 XII.1.2 queue에 사용자 입력보다 낮은 priority로 넣고 넘치면 busy를 반환합니다. lease 대상 계산과 batch 적용은 단일 점유 상한 안에서 나눠 처리하고, 디코드·원문 버퍼링·표시용 텍스트 준비는 이 스레드나 worker에서 합니다. | `[DEV-MAC]` 20만 node 문서 lease·batch trace에서 UI·Document 단일 점유와 사용자 입력 지연 증가가 예산 이내. | XXIX.3.1, XII.1.2, XIX.4.2 |
| XXIX.3.5 | `[ ]` | 내장 세션 시작 전에 XVI.7.1 capability 검사를 적용해 untrusted 프로젝트에서는 시작하지 않습니다. 사용자가 대상 프로젝트, 실행할 CLI, 모드를 확인하는 시작 확인 데이터를 만들고 확인을 첫 연결 승인으로 기록합니다(아키텍처 31.3). 각 CLI를 처음 쓰기 전과 profile 약관 검토 결과가 바뀐 뒤에는 약관 고지 확인을 요구하고, 그 기록은 OS 사용자 데이터에 둡니다. 세션 중 신뢰가 철회되면 즉시 세션을 중지하고 XIX.6.4 절차를 따릅니다. | `[DEV-MAC]` untrusted 시작 0, 확인 기록 존재, 고지 미확인 시 시작 0, 검토 결과 변경 후 재고지, 철회 fixture에서 lease 0·대기 승인 0·적용 변경 자동 삭제 0. | XXIX.3.2, XVI.7.1, XIX.3.3, XXIX.2.7 |
| XXIX.3.6 | `[ ]` | 채팅 item 조립을 구현합니다. 채팅 보기는 문서 상태나 승인 결과의 근거가 아니며, 문서 효과 표시는 Stellar 승인·출처 기록(XXIX.3.2, XIX.6.7)과 일치해야 합니다(아키텍처 31.3~31.5, 31.9). 원문과 채팅 기록은 세션 메모리에만 둡니다. | `[DEV-MAC]` 기록 fixture로 만든 item 목록 golden 일치, 승인·출처 기록과 어긋나는 문서 효과 표시 0. | XXIX.2.11, XXIX.3.2, XIX.9.5, XIX.6.7 |

### XXIX.4 macOS 에이전트 UI

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXIX.4.1 | `[ ]` | macOS 프로젝트 창에 에이전트 패널 자리를 추가합니다. 캔버스 렌더 경로 밖의 네이티브 chrome에 두고 캔버스 위에 겹쳐 그리지 않으며, 배치와 표시 전환 방식은 HIG를 근거로 정해 보고서에 기록합니다(ADR-0005 §4.5). 패널 열기·닫기·크기 변경 중 캔버스 drawable 크기 전달은 frame 예산 안에서 묶어 처리합니다. | `[DEV-MAC]` 열기·닫기 반복 중 frame time trace가 예산 이내, 접근성 smoke 통과. | XII.9.1, XXIX.3.1 |
| XXIX.4.2 | `[ ]` | macOS 에이전트 패널을 구현합니다. 창마다 하나이고 그 창에서 시작한 세션만 보여 줍니다(ADR-0005 §4.2). XXIX.2.8 상태와 출처 표시를 보여 주고 중지 동작을 제공하며, 배치는 HIG로 정해 보고서에 기록합니다. 입력은 여러 줄 입력과 IME 조합을 지원하는 네이티브 텍스트 컨트롤입니다. Stellar는 CLI 입력에 문서 내용을 넣지 않고, 노드 지정은 BeginAgentEdit target root 제안으로만 합니다(XIX.4 lease 모델). 패널을 숨겨도 세션과 승인 대기는 유지됩니다. | `[DEV-MAC]` 테스트 대역 세션 UI smoke, Stellar가 CLI 입력에 넣은 문서 내용 0, 노드 지정이 target root 제안으로만 전달, 패널 숨김 중 세션·승인 대기 유지. | XXIX.4.1, XXIX.3.3, XXIX.3.5 |
| XXIX.4.3 | `[ ]` | 채팅 보기를 구현합니다(기본 보기). XXIX.3.6 item을 네이티브 텍스트 컨트롤로 그리며, 에이전트 텍스트를 신뢰하지 않는 입력으로 다뤄 표시 과정에서 네트워크 요청, 스크립트 실행, WebView 생성이 없게 합니다(ADR-0005 §4.4 표시 안전). 서식 범위는 이 작업에서 정해 보고서에 기록합니다. 프로젝트 문자열과 에이전트 텍스트는 출처를 구분해 표시합니다. 스트리밍 중 갱신은 UI 스레드 단일 점유 상한 안에서 묶어 반영합니다. | `[DEV-MAC]` 새 세션 기본 보기 채팅, 원격 리소스 요청 0, WebView 인스턴스 0, 1만 item 세션 스크롤 trace 예산 이내. | XXIX.4.2, XXIX.3.6 |
| XXIX.4.4 | `[ ]` | 원문 보기와 보기 전환을 구현합니다. 원문 보기는 XXIX.2.12 버퍼를 읽기 전용 텍스트로 보여 주고 선택·복사를 지원합니다. 보기 전환은 세션마다 하며, 상태는 창 세션 메모리에만 두고 새 세션과 앱 재실행 후에는 채팅 보기로 시작합니다. 전환은 CLI 프로세스·권한·lease·승인 요청에 영향을 주지 않습니다. | `[DEV-MAC]` 보기 전환 후 item·원문 손실 0, 앱 재실행 후 기본 보기 채팅, 전환 전후 세션 상태 동일. | XXIX.4.2, XXIX.2.12 |
| XXIX.4.5 | `[ ]` | macOS 승인 화면을 네이티브 chrome(패널, 창 sheet)에만 표시하고 캔버스와 플러그인 WebView에는 그리지 않습니다. Stellar 코어가 만든 semantic diff·validation·렌더 프리뷰를 승인 근거로 보여 줍니다(XIX.5.3). Terminal·FileDelete·GitCommit·GitPush 확인은 HIG 파괴적 동작 패턴을 따르고 VoiceOver로도 확정할 수 있어야 합니다. 화면 구성은 HIG를 근거로 정해 보고서에 기록합니다. | `[DEV-MAC]` 승인 화면에 코어 semantic diff·validation·렌더 프리뷰 표시, 캔버스 영역 승인 UI 0, VoiceOver 승인 경로 smoke. | XXIX.4.3 |
| XXIX.4.6 | `[ ]` | 승인 전 제안 batch를 XII.3.7 transient preview로 캔버스에 겹쳐 보여 주고, 레이어 패널과 캔버스에 lease 잠금 표시를 모든 창에 그립니다. 미리보기는 revision을 바꾸지 않습니다. | `[DEV-MAC]` preview 전후 revision 동일, 다중 창 잠금 표시 일치. | XXIX.4.5, XII.3.7, XIX.5.3 |
| XXIX.4.7 | `[ ]` | 세션 중지, 중지 후 그 세션 변경 되돌리기, 에이전트 lease 해제 동작을 공유 command ID로 제공합니다(ADR-0005 §4.5, 아키텍처 31.5). 중지와 lease 해제는 XIX.6.4 절차(적용 변경 유지·출처 표시, lease 해제)를 따르고, 중지 후 되돌리기는 XIX.6.3을 따릅니다. 메뉴·명령 팔레트 항목과 이름, 배치는 HIG를 근거로 정해 보고서에 기록합니다. 기능 이름과 아이콘에 공급자 이름이나 로고를 넣지 않습니다. | `[DEV-MAC]` lease 해제 후 해당 lease 0·적용 변경 자동 삭제 0, 중지 후 대기 제안 0, 중지 후 되돌리기 미리보기 승인 후 비충돌 속성 baseline 일치. | XXIX.4.2, XII.6.5, XIX.6.9 |
| XXIX.4.8 | `[ ]` | macOS 설정에 내장 에이전트 설정 화면을 구현합니다. 구독 CLI 선택, 연결 상태(XXIX.2.5), 약관 고지와 확인 상태를 다루며, 화면 구성은 HIG와 각 CLI 공식 문서를 근거로 정해 보고서에 기록합니다. Stellar는 로그인 화면이나 로그인 흐름을 제공하지 않습니다(ADR-0005 §4.1). 비용과 사용량은 표시하지 않고, CLI 이름은 일반 텍스트로만 씁니다. | `[DEV-MAC]` 상태별 화면 snapshot, 사용량 표시 요소 0, 자격 증명 입력 요소 0, 화면 동작 중 CLI 자격 증명 저장 위치 접근 0. | XXIX.2.5, XXIX.4.1 |
| XXIX.4.9 | `[ ]` | 활성 lease, 실행 중인 세션, 검토 대기 출처 표시가 있으면 macOS 창 닫기와 앱 종료 전에 확인합니다(ADR-0005 §4.5). 출처 표시는 창과 함께 사라지고 변경은 남는다는 점을 알리고, 사용자가 되돌리기를 고르면 XIX.6.9 미리보기를 거칩니다. 되돌리기 확인은 HIG 파괴적 동작 패턴을 따릅니다. | `[DEV-MAC]` 닫기 확인 fixture 통과, 미리보기 승인 전 revision 변화 0. | XXIX.4.2, XIX.6.7, XIX.6.9 |
| XXIX.4.10 | `[ ]` | 에이전트 변경 출처 표시와 되돌리기 UI를 구현합니다. 레이어 패널 노드 행의 표지(색에만 의존하지 않는 기호와 접근성 레이블), 사용자가 켜고 끄는 캔버스 transient overlay(XII.4.4 방식, project snapshot·렌더 프리뷰·내보내기 산출물에 없음), 인스펙터의 속성 단위 출처 표시, 패널의 세션별 변경 목록(시각, semantic diff 요약, 상태, 대상 노드 이동)과 적용 변경 item 동작(캔버스에서 보기, 이 변경 되돌리기, 이 세션 변경 모두 되돌리기, 유지)을 둡니다. 되돌리기 전에 충돌 목록과 미리보기 sheet를 보여 주며 충돌 항목은 기본 제외하고 항목별로 고를 수 있습니다. | `[DEV-MAC]` 중단 세션 fixture에서 표시 노드 집합이 출처 색인과 일치, 되돌리기·유지 동작 smoke, overlay가 저장 bytes에 0, VoiceOver로 네 동작 접근 가능. | XXIX.4.3, XIX.6.10, XII.4.4 |
| XXIX.4.11 | `[ ]` | 에이전트 패널의 접근성과 키보드 조작을 구현합니다(아키텍처 18 애플리케이션 자체 UI). 모든 item에 AppKit 접근성 role과 label을 주고 상태 변화를 보조 기술에 알리며, 패널의 모든 조작을 키보드만으로 할 수 있게 합니다. 시스템 대비와 동작 줄이기 설정을 따릅니다. | `[DEV-MAC]` VoiceOver automation smoke 통과, 키보드 전용 시나리오 통과. | XXIX.4.3, XXIX.4.4, XXIX.4.5, XII.8.1, XII.9.3 |

### XXIX.5 `stellar-mcp` 번들 배포

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXIX.5.1 | `[ ]` | macOS 앱 번들에 `stellar-mcp`를 helper로 넣고 XXIV.5.1과 같은 규칙(Apple 공식 코드 서명·공증 문서, 앱 entitlement 없음)으로 서명합니다. 내장 에이전트 UI는 이 작업으로 노출하지 않습니다. | `[DEV-MAC]` `codesign --verify --strict` 통과, `[CI-MAC]` 공증 후 clean runner에서 helper `stellar-mcp`의 XIX.9.8 연결 정보 제공 동작 실행. | XXIV.5.1, XIX.9.4 |
| XXIX.5.2 | `[ ]` | 번들된 `stellar-mcp`와 앱의 내부 IPC protocol 버전을 연결 시 협상하고, 불일치하면 연결하지 않고 XXIV.1.2 종료 코드 표에 따라 끝냅니다. | `[DEV-MAC]` 버전 불일치 fixture에서 연결 거부, 종료 코드가 XXIV.1.2 표와 일치. | XXIX.5.1, XIX.1.7 |

### XXIX.6 E2E·불변식

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXIX.6.1 | `[ ]` | 테스트 대역(XXIX.2.2) 기반 E2E 시나리오 정의를 공통 fixture로 작성합니다. (1) 세션 시작, 범위 승인, batch 미리보기·적용, 완료, Undo 1회 복원. (2) 세션 중지 후 적용 변경 유지·출처 표시·lease 0·대기 제안 거부, 이어서 변경 단위 되돌리기와 세션 단위 되돌리기. (3) 재연결 중 lease 유지와 유예 초과 시 중단됨 history entry 확정(자동 rollback 없음). (4) 디코드 실패 후 적용 변경 유지와 권한·lease 확대 0. | `[DEV-MAC]` 시나리오 정의 golden, fixture 로더 test 통과. | XXIX.3.5, XXIX.2.8 |
| XXIX.6.2 | `[ ]` | XXIX.6.1 시나리오를 macOS 앱에서 실행하는 E2E를 작성합니다. | `[DEV-MAC]` 완료 시 history entry 1, 중지 후 적용 batch 유지와 표시 노드 일치, 되돌리기 후 비충돌 속성 baseline 일치, 재연결 시나리오 lease 유지, 기본 보기 채팅. | XXIX.6.1, XXIX.4.6, XXIX.4.10 |
| XXIX.6.3 | `[ ]` | macOS에서 앱이 프로젝트를 연 상태의 공존 E2E를 작성합니다. `stellar apply`가 semantic reconciliation으로 처리되는지(두 선택지 alert 없음), 앱이 열린 동안의 Git merge driver 실행, `stellar-mcp` attached·headless backend 선택, lease와 겹치는 외부 변경의 XIX.7.2 처리(자동 삭제 없음)를 다룹니다. | `[DEV-MAC]` 정책 결과 matrix 일치, merge driver 정상 fixture exit 0. | XXIX.6.2, XIX.9.4, XXIV.4.6, XIX.7.2, XXIV.2.1 |
| XXIX.6.4 | `[ ]` | 에이전트 관련 필드(사용자 입력 문장, 채팅 item, 원문 버퍼 내용, CLI 표준 오류, CLI 실행 파일 경로, CLI가 보고한 계정 식별 정보, 세션 결합 수단, IPC 상대 정보, 출처 색인)를 XVI.8.2 금지 필드 scanner corpus와 XVI.8.6 schema inventory에 추가합니다. 진단, recovery snapshot, 텔레메트리에 이 필드가 들어가지 않고 앱 재시작 후 세션과 출처 표시가 복원되지 않음을 검사합니다. | `[DEV-MAC]` privacy scan 0건, 재시작 후 세션 0·출처 표시 0. | XXIX.6.2, XVI.8.2, XVI.8.6, XIII.8.2 |
| XXIX.6.5 | `[ ]` | ADR-0005 §3 변경 불가 경로 inventory test를 추가합니다. 프로젝트 파일 편집이나 에이전트 경로로 XVI.7 프로젝트 신뢰 상태, XIX.3 권한 상태, 에이전트 모드, 에이전트 변경 출처 표시와 되돌리기를 바꾸는 경로가 없어야 합니다(ADR-0005 Security). | `[DEV-MAC]` 프로젝트 파일 변조 fixture에서 위 상태 변경 0, agent 경로의 출처 표시 정리·되돌리기 호출 0. | XIX.9.7, XXIX.3.3 |
| XXIX.6.6 | `[ ]` | 출력 표시 격리 inventory test를 추가합니다. 터미널 경로(XVIII crate)와 다른 crate·셸·FFI에 공급자별 출력 parser가 없고 터미널 출력이 제품 상태로 해석되지 않음, 디코더가 agent-host 안에만 있음, 디코드 결과 타입에서 CommandTransaction·문서 변경·승인 응답으로 가는 경로 0, agent-host에서 commands·document 전이 도달 0을 함께 검사합니다. | `[DEV-MAC]` 네 검사 모두 0건, 위반 fixture 거부. | XXIX.3.6, XVIII.5.1, XXIX.1.1 |
| XXIX.6.7 | `[ ]` | 스트리밍 부하 성능 게이트를 추가합니다. 테스트 대역이 최대 속도로 출력하는 동안 캔버스 인터랙션 지연이 6.2~6.4 계약을 지키고, UI 스레드 단일 점유가 2ms 이하이며, 원문 버퍼 상한으로 메모리가 제한되는지 측정합니다. | `[REF-MAC]` 스트리밍 중 인터랙션 trace가 예산 이내, 메모리 상한 유지, 출력 수신 정지 0. | XXIX.6.2, XVI.4.6 |
| XXIX.6.8 | `[ ]` | launch profile별 릴리스 전 수동 검증 절차를 작성합니다. ADR-0005 §4.3 두 불변식과 디코드 결과를 확인하고 결과와 약관 검토 일자를 보고서에 기록합니다. 보고서에는 XVI.8 금지 정보를 넣지 않습니다. | `[DEV-MAC]` profile별 수동 검증 항목 전부 통과 기록, 보고서 privacy scan 0건. | XXIX.6.2, XXIX.2.6 |

**Phase XXIX 완료 조건:** macOS에서 사용자가 시작한 구독 CLI 세션이 Stellar MCP tool만으로 lease·승인 정책을 지키며 편집하고, Stellar는 공급자 인증·자격 증명·사용량에 관여하지 않습니다. 세션이 끝나도 적용 변경은 출처 표시와 함께 남아 사용자가 변경·세션 단위로 직접 되돌릴 수 있고, 채팅 보기(기본)와 원문 보기가 동작하며, 공급자 출력 디코드는 표시 전용으로 격리됩니다.

---

## Phase XXX. 로드맵 — 노드 기반 프로토타이핑과 Component Context

> **범위:** ADR-0025 결정 2(XXX.1\~XXX.6)와 결정 3(XXX.7)을 구현합니다. 프로토타입 그래프는 CSS 기반 속성, Value Binding, Expression AST, 선언형 상태 머신, Control Contract, Context 위의 읽기 전용 projection이며 별도 원본을 만들지 않습니다. 새로 저장하는 것은 `MotionSpec`, 그래프 배치 hint, EventKind 확장, Component Context record(IX.12.1이 예약한 번호 사용)뿐입니다. 문서에는 실행 코드를 넣지 않고, 모든 편집은 CommandTransaction이며, 프리뷰는 문서를 쓰지 않습니다. 업계 프로토타이핑 도구는 개념 기준점으로만 쓰고 UI 문구와 시각 요소를 옮기지 않습니다.

### XXX.1 프로토타입 모델

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXX.1.1 | `[ ]` | `MotionSpec` record에 StableId, owner(ComponentDefinition 또는 ViewDefinition), 대상 PropertyPath 목록, 시작 조건(상태 변수 StableId와 값 조건), 지속 시간, 지연, timing function, reduced motion 대체 동작(기본 즉시 전환)을 정의합니다. timing function은 linear, cubic-bezier(4 parameter), steps(개수·위치), spring(질량·강성·감쇠·초기 속도)의 닫힌 집합입니다. Color 보간 공간은 ADR-0025 결정 1.2를 따릅니다. 저장된 색공간(sRGB 또는 Display P3)의 encoded 채널을 premultiplied로 선형 보간하고, 시작값과 끝값의 색공간이 다르면 거부합니다. 저장 형식은 착수 시 ADR로 고정합니다. | parameter 범위 위반·보간할 수 없는 대상 타입 fixture 거부, 색공간이 다른 Color 끝값 fixture 거부, round-trip. | IX.5.1, VIII.6.1 |
| XXX.1.2 | `[ ]` | 프로토타입 socket 종류 State와 Pulse를 정의하고 expression socket 연결 가능 API를 확장합니다. State와 Pulse의 직접 연결은 거부하고 등록된 변환 node(상태 변화→펄스, 펄스→상태 토글·카운터)만 허용합니다. 속성 입력 포트의 State socket 타입은 registry 항목의 CSS 값 문법을 ADR-0025 결정 1.2 대응으로 바꾼 `ExpressionType`이며, 합집합 문법(`width` 등)은 구성 타입별 `ExpressionType`의 집합입니다. 연결된 식의 정적 타입이 그 집합에 속해야 하고 저장 값 variant는 그 타입을 따릅니다. 대응이 없는 값 문법의 속성은 포트로 노출하지 않고 진단합니다. | 종류가 다른 직접 연결 fixture 거부, 변환 node 경유 연결 허용, 값 문법별 포트 타입 golden, 대응 없는 값 문법 진단 fixture, `width` 포트에 Length 식과 Percentage 식 연결 허용·Color 식 거부, 저장 값을 Length에서 Percentage로 바꿔도 연결 검사 결과 불변. | VIII.3.2, IX.5.2, IX.11.1 |
| XXX.1.3 | `[ ]` | 프로토타입 node 종류 registry를 닫힌 집합으로 정의합니다. 속성 출력(PropertyPath), expression block, 상태 변수, 이벤트 원천, Transition, semantic emit, Context 읽기, MotionSpec, 변환 node만 둡니다. 사용자 정의 코드, JavaScript, 파일·네트워크 node는 표현할 수 없게 합니다. | serialized unknown node kind 거부, registry 완전성 검사 통과. | XXX.1.1, XXX.1.2, XXX.7.3, IX.11.2, IX.11.3 |
| XXX.1.4 | `[ ]` | EventKind에 hover, pointer down·up, drag(변위), scroll, long press를 추가하는 확장을 ADR로 고정한 뒤 구현합니다. 현재 EventKind payload는 `TypeSpec` 하나(`crates/document/src/component/event.rs`의 `payload_type`)이고 `TypedValue`에는 2D 값이 없습니다(`crates/core-types/src/typed_value.rs`). 그래서 ADR은 drag·scroll 변위 payload를 2D 값 타입 추가(`TypedValue`·`ExpressionType` variant와 tag 추가)와 축별 이벤트(x·y 각각 `Length` payload, 같은 tick 안에서 x 다음 y) 중 하나로 정합니다. drag 끝 이벤트는 spring 초기 속도(XXX.3.2)에 쓸 축별 release velocity(px/tick)를 싣고, velocity는 registry에 고정한 개수만큼의 마지막 tick 변위를 IEEE 기본 연산으로 평균해 계산합니다. 제스처 판정 임계값은 논리 tick 단위로 registry에 고정하고 기존 kind 번호는 바꾸지 않습니다. | 기존 kind golden 불변, 새 kind golden test 통과, ADR에 변위 payload 방식과 release velocity 규칙 존재, 같은 입력 로그의 velocity 값 byte-identical. | IX.5.2 |
| XXX.1.5 | `[ ]` | 사용자가 고정한 그래프 node 위치만 희소 배치 hint(StableId→좌표)로 저장합니다. Semantic Diff에서는 표현 전용 change로 분류해 따로 stage할 수 있게 합니다. | hint 없는 그래프의 save bytes에 배치 정보 0, hint 변경의 diff kind 정확. | XXX.1.3, XIV.3.2 |
| XXX.1.6 | `[ ]` | `MotionSpec` 생성·변경·삭제 command를 구현합니다. 대상 속성이 삭제되면 MotionSpec을 지우지 않고 orphan 진단으로 남깁니다. | apply/inverse 통과, 대상 삭제 orphan fixture 정확. | XXX.1.1, Phase VII |

### XXX.2 그래프 projection과 편집

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXX.2.1 | `[ ]` | owner의 binding, expression AST, 상태 머신, Control Contract 이벤트, Context 읽기, MotionSpec에서 node·edge 목록을 만드는 읽기 전용 `PrototypeGraphView`를 구현합니다. node·edge 순서는 StableId 기준으로 결정합니다. | projection→편집 plan→원본 의미 동일, 같은 입력 projection bytes 동일. | XXX.1.3, VIII.3.1, IX.5.3 |
| XXX.2.2 | `[ ]` | 결정론적 자동 배치를 구현합니다. hint 없는 node 위치를 계산하고 hint 있는 node는 고정합니다. | 같은 입력 같은 좌표, hint node 좌표 불변. | XXX.2.1, XXX.1.5 |
| XXX.2.3 | `[ ]` | node 추가·연결·해제·삭제를 기존 binding·expression·transition command와 MotionSpec command로 낮춘 command plan을 생성하는 API를 구현합니다. 사용자 동작 하나는 CommandTransaction 하나입니다. 실제 UI는 XXX.5에서 연결합니다. | 각 plan 적용 후 valid 문서, apply/inverse 통과, Undo 1회로 동작 전 상태 복원. | XXX.2.1, XXX.1.6, VIII.3.3, VIII.6.6 |
| XXX.2.4 | `[ ]` | 포트 공개를 구현합니다. 그래프에서 컴포넌트 밖으로 포트를 공개하면 필요한 공개 ComponentProperty·Event·ContextRequirement 생성 plan을 계산하고, 미리보기 후 하나의 CommandTransaction으로 적용합니다. nested instance 내부 포트에 직접 연결하는 edge는 거부합니다. | nested 직접 edge fixture 거부, fault injection 시 생성된 Property 0. | XXX.2.3, IX.4.5, XXX.7.8 |
| XXX.2.5 | `[ ]` | expression, 상태 머신, Context, MotionSpec 진단을 그래프 node·socket ID로 매핑합니다. | 오류 위치 fixture 정확. | XXX.2.1, VIII.3.4, IX.5.7, XXX.7.5 |

### XXX.3 결정론적 평가

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXX.3.1 | `[ ]` | 프리뷰 세션에 고정 tick 논리 시계 `PreviewClock`을 정의합니다. 문서 평가 경로에는 시스템 시각 접근을 두지 않고, 셸은 디스플레이 프레임을 tick 번호로 대응시키기만 합니다. | 평가 경로 API inventory에 시스템 시각 호출 0, 같은 tick 열 같은 결과. | IX.5.4, XVI.2.1 |
| XXX.3.2 | `[ ]` | timing function evaluator를 구현합니다. cubic-bezier는 고정 반복 횟수 해법, spring은 고정 step 적분을 쓰고 결과는 tick의 순수 함수입니다. 진행률에 따른 값 보간(Color는 ADR-0025 결정 1.2의 premultiplied encoded 채널 선형 보간)도 같은 evaluator에 둡니다. 계산은 IEEE-754 기본 연산(사칙연산·`sqrt`)만 씁니다. `exp`·`sin`·`cos`·`pow` 같은 libm 초월함수가 필요한 닫힌 식은 ADR-0006 기준으로 플랫폼 의존 결과가 되므로 쓰지 않습니다. | reference corpus bytes 동일(Color 보간 포함), 경계 parameter fixture 통과, 결과 alpha 0 Color 보간 fixture, evaluator 경로의 libm 초월함수 호출 inventory 0. | XXX.1.1, VIII.4.6 |
| XXX.3.3 | `[ ]` | tick 처리 순서를 고정합니다. 입력 이벤트→펄스 dispatch(정해진 순서, 원자 commit)→상태 변화에 따른 MotionSpec 시작·현재 값에서 재지정→영향받는 binding 재평가 순서입니다. 펄스 연쇄 dispatch 깊이가 상한을 넘으면 진단하고 그 tick의 상태 변화는 버립니다. | 같은 이벤트 로그·tick 열의 값 trace byte-identical, 무한 펄스 fixture 상한 오류·상태 변화 0. | XXX.3.1, XXX.3.2, XXX.1.4, IX.5.5 |
| XXX.3.4 | `[ ]` | tick마다 활성 MotionSpec과 그 대상 binding만 평가하도록 binding scheduler에 연결합니다. | 무관 binding 평가 count 0. | XXX.3.3, VIII.6.4 |
| XXX.3.5 | `[ ]` | 환경 key `reduced_motion`이 켜지면 MotionSpec의 대체 동작을 씁니다. | 환경 on/off fixture 결과 정확. | XXX.3.3, VIII.7.3 |
| XXX.3.6 | `[ ]` | 프리뷰 상태, 진행 중 motion, 입력 로그를 세션 메모리에만 둡니다. 프로젝트 저장과 recovery snapshot에 넣지 않습니다. | 프리뷰 조작 후 save bytes 동일, recovery snapshot의 프리뷰 field 0. | XXX.3.3, XIII.8.2 |

### XXX.4 프리뷰

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXX.4.1 | `[ ]` | macOS 프리뷰 세션을 구현합니다. 캔버스 렌더러로 프리뷰를 그리고, InputEvent를 프로토타입 이벤트로 변환하며, 평가 결과는 transient scene으로 주입합니다. | `[DEV-MAC]` 토글·drag fixture 프리뷰 smoke, project revision 증가 0. | XXX.3.3, XII.3.7, XII.4.1 |
| XXX.4.2 | `[ ]` | 프리뷰 입력을 입력 재생 파일 형식으로 기록·재생해 같은 값 trace를 재현합니다. 기록 파일은 프로젝트 밖 사용자 위치에만 둡니다. | 기록→재생 trace byte-identical, 프로젝트 working tree 변경 0. | XXX.4.1, XVI.2.2 |
| XXX.4.3 | `[ ]` | 프리뷰를 View presentation 단위로 열고 View 변수 assignment와 TargetProfile 환경을 고정해 실행합니다. | 같은 View의 두 presentation 프리뷰가 서로 독립된 state를 가짐. | XXX.4.1, IX.7.9, IX.10.7 |

### XXX.5 macOS 그래프 편집기 UI

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXX.5.1 | `[ ]` | 편집기 영역을 분할해 캔버스 옆에 그래프 편집 면을 추가합니다. 편집 면은 코어 렌더러 scene으로 그리고(ADR-0025 결정 2.11), 포트 목록·속성 편집·메뉴·툴바·다이얼로그는 네이티브 컨트롤을 씁니다. | `[DEV-MAC]` UI hierarchy에 승인 범위 밖 커스텀 컨트롤 0, 분할 열기·닫기·창 상태 복원 smoke. | XXX.2.2, XII.9.1, XII.4.1 |
| XXX.5.2 | `[ ]` | 인스펙터와 연결합니다. 선택 node의 CSS 기반 속성을 입력 포트로 보여 주고, 인스펙터 속성을 그래프로 끌면 속성 출력 node를 만드는 plan을 적용합니다. | `[DEV-MAC]` 끌기→plan 적용→Undo smoke. | XXX.5.1, XXX.2.3, IX.11.2, IX.11.3 |
| XXX.5.3 | `[ ]` | socket 연결 상호작용을 구현합니다. 끌어서 연결하고, 연결 가능한 socket을 강조하고, 연결할 수 없는 사유를 표시하며, pointer up에서만 확정합니다. | `[DEV-MAC]` 연결·취소 fixture 통과, drag 중 project revision 증가 0. | XXX.5.1, XII.3.7 |
| XXX.5.4 | `[ ]` | node·socket 진단을 그래프에 표시하고 진단 목록에서 해당 node로 이동하게 합니다. | `[DEV-MAC]` 진단 fixture 표시 위치 정확. | XXX.5.1, XXX.2.5 |
| XXX.5.5 | `[ ]` | node와 socket을 VoiceOver 요소로 노출하고 키보드만으로 node 이동·연결·삭제를 할 수 있게 합니다. | `[DEV-MAC]` VoiceOver automation smoke, 키보드 연결 시나리오 통과. | XXX.5.3, XII.8.1 |

### XXX.6 통합·성능

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXX.6.1 | `[ ]` | 통합 fixture를 만듭니다. motion이 있는 토글 버튼, drag sheet, Context 테마 전환 카드, 10단계 nested component의 공개 포트 연결을 포함합니다. | validator 0 오류, 재생 trace byte-identical. | XXX.3.6, XXX.2.4, XXX.7.13 |
| XXX.6.2 | `[ ]` | node 1,000개 그래프 편집과 활성 motion이 있는 프리뷰 중 캔버스 인터랙션이 아키텍처 6.2\~6.4 계약을 지키는지 측정합니다. 활성 motion 수 기준은 착수 시 측정으로 정하고, 성능 예산 파일 변경은 ADR로 합니다. | `[REF-MAC]` 인터랙션 trace가 예산 이내. | XXX.6.1, XVI.4.6 |
| XXX.6.3 | `[ ]` | 문서 schema와 프로토타입 registry에 실행 코드, 파일·네트워크, 시스템 시각 접근 경로가 없음을 검사하는 inventory test를 추가합니다. | `[DEV-MAC]` inventory 0건, 위반 fixture 거부. | XXX.1.3, XXX.3.1 |

### XXX.7 Component Context

ADR-0025 결정 3을 구현합니다. Context는 선언된 인터페이스입니다. 아키텍처 14.3의 "경계마다 Property" 규칙은 Property 값에 적용되고, Context는 14.5의 `ContextRequirement` 선언으로만 컴포넌트 경계를 넘습니다. 저장 번호는 IX.12.1이 예약한 값을 씁니다.

| ID | 상태 | 구현 지시 | 검증·완료 기준 | 선행 |
|---|---|---|---|---|
| XXX.7.1 | `[ ]` | `ContextKey` resource에 StableId, 이름, TypeSpec, 필수 default expression, scope(프로젝트·라이브러리·문서)를 정의합니다. default는 닫힌 식이어야 하며 default 없는 key는 만들 수 없습니다. 참조는 StableId로 합니다. 번호는 IX.12.1 예약 값을 쓰고 나머지 저장 형식은 착수 시 ADR로 고정합니다. | default 누락·type mismatch·열린 식 fixture 거부, round-trip, rename 후 참조 유지. | IX.12.1, IX.1.2, VIII.5.2 |
| XXX.7.2 | `[ ]` | node에 붙는 `ContextProvision`(provider node StableId, ContextKey ref, value expression), ComponentDefinition 공개 인터페이스의 `ContextRequirement`(ContextKey ref), Slot의 공개 Context key 목록(ADR-0025 결정 3.11)을 정의합니다. provision은 소유 definition·View 안의 node에만 붙고, 자기 트리 안의 ComponentInstance node에는 붙일 수 있습니다. | round-trip, 소유 범위 밖 node를 가리키는 provision 거부, instance node provision 허용, Slot 공개 목록 round-trip. | XXX.7.1, IX.1.1, IX.2.1, IX.3.1 |
| XXX.7.3 | `[ ]` | ReferenceTarget에 Context 종류를 IX.12.1이 예약한 tag로 추가하고 binding dependency graph가 ContextKey와 ContextProvision을 공통 node로 표현하게 합니다. 기존 ReferenceTarget tag(`crates/expressions/src/ast.rs`의 Token·Property·ComponentProperty·Environment)와 bytes는 바꾸지 않습니다. | dependency 종류 fixture 통과, 기존 ReferenceTarget golden bytes 불변. | XXX.7.2, VIII.6.2 |
| XXX.7.4 | `[ ]` | Context 해석기를 구현합니다. instance expansion view의 펼친 트리에서 소비 지점의 가장 가까운 조상 provision을 쓰고, 없으면 key default를 씁니다. provision 값 식이 같은 key를 읽으면 바깥 조상 값으로 해석합니다. slot 콘텐츠는 작성 scope 기준으로 해석합니다. slot host에 이르면 컴포넌트 내부 조상을 건너뛰고 instance node로 이어 올라가며, Slot 공개 목록에 있는 key만 slot host부터 컴포넌트 내부 조상 provision을 찾습니다(ADR-0025 결정 3.11). 조상 탐색 결과는 runtime index로 캐시합니다. | 중첩·shadowing·provision 없음·같은 key 재읽기 fixture 값 정확, slot fixture(공개하지 않은 key는 instance 바깥 값, 공개한 key는 내부 provision 값, nested slot) 값 정확, 같은 입력 같은 결과. | XXX.7.3, IX.2.2, IX.3.2 |
| XXX.7.5 | `[ ]` | Context validator를 구현합니다. ComponentDefinition 안의 선언 없는 소비(그 정의가 작성한 slot 콘텐츠 안의 소비 포함)와 타입 불일치는 오류, 존재하지 않는 key의 provision과 Slot 공개 목록 항목은 orphan, 쓰지 않는 requirement는 경고, default 해석과 shadowing은 정보 진단입니다. 서로 다른 key가 서로를 읽는 cycle은 binding cycle detector로 저장 전 거부합니다. | diagnostic fixture 정확, K1↔K2 cycle fixture 거부, View 최상위 트리의 선언 없는 소비 허용, View 최상위에서 작성한 slot 콘텐츠가 공개되지 않은 내부 provision 대신 instance 바깥 값으로 해석되는 fixture, 정의 안에서 작성한 slot 콘텐츠의 선언 없는 소비 오류 fixture. | XXX.7.4, VIII.6.3 |
| XXX.7.6 | `[ ]` | Context 해석 결과를 바꾸는 변경에서 영향받는 소비 binding만 topological order로 재평가하도록 binding scheduler에 연결합니다. 무효화 조건은 provision 추가·삭제·값 변경, ContextKey default 변경, 소비 node나 그 조상의 reparent·이동·삭제, instance의 ComponentRef 교체, slot 콘텐츠 삽입·삭제·재정렬, Slot 공개 목록 변경, View Variant 구조 patch(node add·delete·move·replace, slot change)의 적용·해제, Fragment 묶기·풀기입니다. | 조건별 fixture에서 영향받는 소비 binding 값이 전체 재해석 결과와 같음, 무관 subtree 평가 count 0, 평가 순서 deterministic. | XXX.7.4, VIII.6.4, IX.7.3 |
| XXX.7.7 | `[ ]` | evaluated value provenance chain에 ContextKey와 provision node StableId를 기록합니다. default로 해석된 경우와 slot 공개 목록을 거친 경우도 표시합니다. | 최종값→provision→key chain fixture 정확, default 해석·slot 경유 표시 fixture. | XXX.7.4, VIII.6.5 |
| XXX.7.8 | `[ ]` | ContextKey 생성·rename·default 변경·삭제, provision 생성·값 변경·삭제, requirement 추가·삭제, Slot 공개 목록 추가·삭제 command를 구현합니다. 소비가 있는 key 삭제는 소비 binding을 OrphanedBinding으로 바꾸고, provision만 삭제하면 default로 해석합니다. | apply/inverse 통과, orphan·default fallback fixture 정확. | XXX.7.5, VIII.8.1, Phase VII |
| XXX.7.9 | `[ ]` | View Variant와 Target Profile override가 provision 값을 PropertyPath로 덮어쓰도록 resolver를 연결합니다. 우선순위는 기존 공통값→profile override→variant override 규칙을 따릅니다. | 우선순위 조합 fixture 통과, 같은 specificity 충돌 diagnostic. | XXX.7.4, IX.7.6, IX.10.7 |
| XXX.7.10 | `[ ]` | Semantic Diff·Merge에 Context를 연결합니다. ContextKey·provision·requirement·Slot 공개 목록을 StableId 단위로 diff하고, provision 삭제 대 소비 추가와 key 타입 변경 대 provision 값 변경을 전용 conflict로 분류합니다(아키텍처 27.1). | 3-way fixture conflict 분류 정확, diff bytes deterministic. | XXX.7.8, XIV.3.7, XIV.10.5 |
| XXX.7.11 | `[ ]` | 인스펙터용 Context 조회 API를 구현합니다. 선택 속성의 binding이 Context 소비이면 key, 해석된 값, 제공한 provision node 경로를 반환하고, Value Binding 선택기 후보로 그 위치에서 소비 가능한 key와 선언 필요 여부를 반환합니다. 실제 UI는 XXX.7.12에서 연결합니다. | 조회 결과 fixture 정확, 같은 입력 같은 후보 순서. | XXX.7.7, XXX.7.5 |
| XXX.7.12 | `[ ]` | macOS 인스펙터에 Context를 연결합니다. Value Binding 선택기에 Context를 연결원 종류 하나로 보여 주고, 제공한 조상 node로 이동하는 동작, ComponentDefinition 인터페이스의 requirement 목록, Slot 공개 목록을 네이티브 컨트롤로 표시합니다. Context를 별도 기능 패널로 나누지 않습니다(아키텍처 14.2). | `[DEV-MAC]` 선택→연결→Undo smoke, 조상 이동 smoke, VoiceOver label 확인. | XXX.7.11, XII.9.1, IX.11.17 |
| XXX.7.13 | `[ ]` | 통합 fixture를 만듭니다. 테마 Context가 10단계 nested component를 중간 Property 없이 통과하고, 중간 단계 shadowing, 외부 library key 소비, slot 콘텐츠의 공개·비공개 key 해석을 포함합니다. | validator 0 오류, provenance가 provision 경로 포함, semantic snapshot byte-identical. | XXX.7.9, XXX.7.10, IX.9.2 |

**Phase XXX 완료 조건:** CSS 기반 속성과 상태 머신 위의 노드 그래프로 만든 인터랙션이 같은 입력 로그에서 같은 값 trace를 내고, 그래프 편집이 CommandTransaction과 창별 Undo로 동작하며, 프리뷰가 프로젝트를 바꾸지 않습니다. Component Context 선언·제공·소비가 명시 경계 규칙과 slot 해석 규칙으로 검증되고 provenance에 provision이 나타나며 XXX.7.13 fixture가 통과합니다.

---

## 2. 작업 결과 검토 체크리스트

각 원자적 작업을 병합하기 전에 검토자는 다음 항목만 순서대로 확인합니다.

1. WBS ID가 존재하고 선행 작업이 모두 `[x]`인가.
2. 변경 파일이 작업 범위 안에 있는가.
3. 테스트가 실제 요구사항 실패를 먼저 재현하는가.
4. 구현이 테스트를 통과시키기 위해 요구 범위보다 넓어지지 않았는가.
5. public API·numeric registry·파일 포맷 변경에 ADR이 있는가.
6. canonical bytes 또는 golden fixture가 이유 없이 갱신되지 않았는가.
7. macOS·Windows 공통 결과가 필요한 작업에 교차 플랫폼 검증이 있는가.
8. 오류를 로그만 남기고 성공으로 처리하지 않는가.
9. 프로젝트 내용·경로·Git 정보가 telemetry나 task report에 노출되지 않는가.
10. 성능 경로에서 disk, Git, network, process 호출이 추가되지 않았는가.
11. `fmt`, `clippy -D warnings`, workspace test, 플랫폼 build가 통과하는가.
12. task report와 status ledger가 갱신되었는가.

## 3. 계획 변경 규칙

- 작업을 더 크게 합치지 않습니다. 필요하면 더 작은 하위 ID를 추가합니다.
- 미착수(`[ ]`·`[!]`) 행은 결정 변경에 맞춰 그 자리에서 고칠 수 있습니다. 행의 주제가 바뀌면 고치지 말고 삭제한 뒤 새 ID를 추가합니다.
- 목적이 사라진 행은 계획서와 ledger에서 삭제합니다. `cancelled` 같은 폐기 표기나 폐기 기록은 남기지 않으며 이력은 Git으로 확인합니다.
- 삭제한 ID는 재사용하지 않습니다. 삭제·변경으로 깨진 선행 셀, 본문 ID 참조, ADR·아키텍처 문서의 ID 참조는 같은 변경에서 고칩니다.
- 착수·완료된 행(`[~]`·`[x]`)과 다른 작업자가 진행 중인 행은 고치거나 삭제하지 않고 새 migration·refactor 행을 추가합니다.
- architecture decision이 바뀌면 먼저 본문 아키텍처, ADR, 영향받는 WBS 선행 관계를 수정합니다.
- 완료된 작업의 요구사항을 바꾸면 새 migration 또는 refactor WBS를 추가합니다.
- 저비용 에이전트가 판단해야 하는 문장이 발견되면 해당 문장을 선택 규칙, 입력, 출력, 오류 조건으로 다시 작성한 뒤 실행합니다.
- 실행 중 새 기능을 발견해도 현재 작업에 포함하지 않고 backlog WBS 후보로만 기록합니다.

---
