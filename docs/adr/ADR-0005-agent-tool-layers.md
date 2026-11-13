# ADR-0005: 3계층 에이전트 도구 구조와 사용자 구독 CLI 연결형 내장 에이전트

## Status

Proposed (2026-11-14). 제품 방향(사용자 구독 CLI 연결, 3계층 구조)은 제품 소유자가 정했고, 세부 경계는 아래 "Open Questions" 결정 후 Accepted로 바꿉니다.

## Context

- Stellar의 원본은 프로젝트 폴더와 Git 저장소의 파일이고, 문서 의미와 파일 포맷은 공통 Rust 코어가 소유합니다. 셸은 의미를 소유하지 않습니다(§8.1).
- Stellar 자체 AI 모델과 에이전트 런타임은 영구 비목표입니다(§4.3). 현재 기준(§31.1, §31.2)에서는 외부 CLI 에이전트가 터미널에서 실행되고 `stellar-mcp` 브리지로 실행 중 앱에 접속합니다.
- 요구사항은 세 가지입니다.
  1. 사용자가 이미 쓰는 AI 구독으로 열린 문서를 앱 안에서 바로 편집합니다. 에이전트는 캔버스 옆에 두고, 세션·입력·승인·미리보기를 제공합니다.
  2. 외부 에이전트와 스크립트도 같은 능력을 쓸 수 있어야 합니다.
  3. Stellar는 자체 모델과 자체 에이전트 런타임을 계속 갖지 않습니다.
- Headless CLI는 CI와 Git merge driver에서 앱 없이 동작해야 합니다(XXIV.1~XXIV.3).
- 계획서의 기존 행 XVIII.5.2, XIX.1.4, XIX.3.3, XIX.6.4는 이 요구와 의미가 충돌합니다. 계획 변경 규칙에 따라 이 행들은 그 자리에서 고치지 않고, 폐기하고 대체 ID를 연결합니다.
- 이 ADR은 §4.3을 바꾸지 않습니다. §31의 경계를 명확히 하고 확장합니다.
- Windows 지원은 ADR-0006에 따라 보류하므로 이 ADR의 플랫폼 항목은 macOS만 다룹니다. L1·L2 library 경계는 플랫폼 중립으로 둡니다.

## Decision

### 1. 3계층 구조와 의존 방향

| 계층 | 구성 요소 | 역할 | 실행 중 앱 필요 여부 |
|---|---|---|---|
| L1 | `stellar-cli`, `stellar-ops` | AI와 무관한 headless 파일 도구. core crate로 프로젝트를 조회·검증·편집 | 필요 없음 |
| L2 | `stellar-mcp`, `stellar-agent-protocol` | L1 library의 능력을 MCP tool·resource로 노출하는 어댑터 | 읽기는 없어도 됨. 쓰기는 앱 필요 |
| L3 | 내장 에이전트 UI, `stellar-agent-host` | 사용자 구독 CLI를 호스팅하고 L2와 연결하는 사용자 화면 | 앱 자체 |

- 아래 계층은 위 계층을 알지 못합니다. 이 관계는 crate 의존 그래프 검사로 강제하며, 직접 의존뿐 아니라 전이 도달도 검사합니다.
- L1에는 AI, MCP, 모델, 에이전트 관련 명령과 의존이 없습니다.
- L2는 L1 library를 in-process로 호출합니다. 실행 중 앱과는 공용 protocol crate의 정의로만 통신합니다.
- L3는 L2를 별도 프로세스로 실행하며, L2의 구현에 의존하지 않습니다.
- `stellar-cli`는 실행 중 앱의 얇은 클라이언트가 아닙니다. 파일이 원본이고 CI와 merge driver가 앱 없이 돌아야 하므로, 실행 중 앱은 프로젝트 파일을 쓰는 여러 writer 중 하나입니다.

### 2. L1: stellar-cli

- **명령**
  - 기존 계획: `validate`, `semantic-diff`, `semantic-merge`, `migrate`
  - 추가: `inspect`, `query`, `apply`
    - `apply`는 파일이나 표준 입력으로 CommandTransaction JSON과 base revision을 받습니다.
    - `--dry-run`을 주면 semantic diff만 출력합니다.
  - 배포 보조 명령은 두지 않습니다.
- **출력**: 표준 출력에는 JSON 결과만, 표준 오류에는 진단만 씁니다.
- **쓰기**
  - 세대 전환 저장 직전에 교차 프로세스 저장 잠금을 잡고, 디스크 generation과 base generation을 비교합니다. 잠금은 프로젝트 폴더와 Git 밖의 OS 사용자 데이터에 둡니다.
  - 저장한 generation에는 writer 종류(앱 창, headless CLI)를 기록합니다.
  - 잠금을 쓰고 있으면 일시적 실패로 끝납니다.
- **merge driver 모드**: Git이 넘긴 임시 경로만 씁니다. 그래서 저장 잠금과 앱 공존 검사 대상이 아닙니다.
- **실행 중 앱과의 공존**
  - L1 쓰기는 앱이 같은 프로젝트를 열고 있어도 막지 않습니다. 시스템 Git이나 외부 편집기와 같은 외부 변경입니다(§22.5).
  - 앱은 writer 종류가 앱 창이 아닌 generation을 §22.2의 두 선택지 정책이 아니라 §22.5의 semantic reconciliation 경로로 처리합니다. 그래서 사용자가 선택하는 순간 CLI 변경이 덮어써지는 일이 없습니다.
  - 변경이 에이전트 lease와 겹치면 §31.5에 따라 그 에이전트 작업을 롤백한 뒤 외부 변경을 적용합니다.
- **종료 코드**: sysexits 정의를 근거로 하며, 표는 XXIV.1.2가 소유합니다. 초안 대응은 다음과 같습니다.

| 코드 | 근거 | 의미 |
|---|---|---|
| 0 | 성공 | 성공 |
| 1 | 도메인 실패 | 검증 진단 있음, 미해결 semantic conflict, 규칙·사용자에 의해 거부된 command |
| 64 | EX_USAGE | 사용법 오류 |
| 65 | EX_DATAERR | 입력 schema 오류, base revision 불일치 |
| 66 | EX_NOINPUT | 프로젝트를 찾거나 열 수 없음 |
| 69 | EX_UNAVAILABLE | 요청한 동작에 필요한 Stellar 구성 요소를 쓸 수 없음 |
| 70 | EX_SOFTWARE | 내부 오류 |
| 73 | EX_CANTCREAT | 저장 실패 |
| 74 | EX_IOERR | 입출력 오류 |
| 75 | EX_TEMPFAIL | 저장 잠금 사용 중, 응답 기한 초과 |
| 76 | EX_PROTOCOL | 내부 protocol 버전 불일치 |
| 77 | EX_NOPERM | 신뢰 확인 실패, 권한 없음 |
| 78 | EX_CONFIG | 설정 오류 |

Git merge driver 모드에서는 0이 아닌 코드가 모두 충돌로 취급된다는 점을 문서화합니다. 128 이상은 신호용으로 남겨 둡니다.

### 3. L2: stellar-mcp

- **위치**
  - `tools/stellar-mcp` 실행 파일(XIX.1.1)은 진입점만 갖습니다.
  - 로직은 `crates/mcp`의 `stellar-mcp` library에 둡니다.
  - 공용 정의는 `stellar-agent-protocol`에 둡니다.
- **backend**: 세션을 시작할 때 고정합니다.
  - **headless 읽기 backend**
    - 실행 인자로 고정한 프로젝트 루트 하나만 다룹니다. tool 인자로 다른 프로젝트를 고를 수 없습니다.
    - 신뢰되지 않은 프로젝트에서는 모든 tool을 거부합니다.
    - 노출하는 tool은 L1의 읽기·검증·dry-run뿐이고, 쓰기 tool은 목록에 나타나지 않습니다.
    - 근거: 같은 사용자의 어느 프로세스든 이미 쓸 수 있는 L1 읽기 능력을 넘지 않습니다.
    - XIX.3.3과의 관계는 XIX.9.11에서 정합니다.
  - **attached backend**
    - 검증된 실행 중 앱 endpoint에 붙습니다.
    - 모든 쓰기는 앱의 lease, 승인, DocumentThread 경로를 거칩니다.
    - 결합 단위는 프로젝트가 아니라 창 세션(WindowSessionId)입니다. 외부 클라이언트는 첫 연결 승인 화면에서 대상 창을 고르고, 앱이 시작한 세션은 시작한 창에 묶입니다.
    - backend 선택은 앱이 기록한 목록이 아니라 endpoint를 실시간으로 검증한 결과로 정합니다.
- **쓰기에 attached가 필요한 이유**
  - 앱 메모리의 미저장 상태 아래에서 파일을 바꾸면 저장 충돌이 생깁니다.
  - lease와 단일 Undo는 앱 DocumentThread에만 있습니다.
  - live 미리보기는 앱의 transient preview 경로에서만 가능합니다.
  - 승인은 네이티브 UI에서만 받습니다(§31.2).
- **카탈로그**
  - resource 목록(XIX.2.1)과 tool 목록(XIX.2.3)은 `stellar-agent-protocol`의 Rust 정의에서 생성합니다.
  - CommandTransaction 입력 schema는 xtask가 closed enum(VII.1.6)에서 생성한 산출물입니다. `stellar-agent-protocol`은 commands crate에 의존하지 않습니다.
  - tool 이름은 MCP 명세의 이름 규칙을 따르고, 읽기 계열과 파괴 계열에는 MCP annotation을 붙입니다.
  - Stellar 사용 안내는 MCP initialize instructions로만 제공합니다.
- **결과 종류**
  - Applied, Proposed, AwaitingUser{request_id}, Rejected{UserDeclined | PolicyRule(name) | RevisionMismatch | LeaseScope | Untrusted}, Expired, Failed{code}
  - 사용자 결정을 기다리는 호출은 클라이언트의 tool timeout 전에 AwaitingUser를 돌려주고, 클라이언트는 같은 request_id로 `edit_status`를 호출해 결과를 받습니다.
  - 클라이언트가 취소를 알리면 대기 중인 요청을 철회합니다.
  - 대기 상한 값은 Stellar 실측으로 정합니다.
- **프로젝트 유래 문자열**: 레이어·토큰·컴포넌트 이름, 텍스트 내용, 외부 library 콘텐츠, commit 메시지는 출처를 표시한 데이터 영역으로 구분해 싣고, 구분 기호를 escape합니다.
- **변경 불가 경로**: MCP 경로에는 다음 상태를 쓰는 수단이 없습니다.
  - 프로젝트 신뢰 상태(XVI.7)
  - MCP 권한 상태(XIX.3)
  - 다른 세션의 lease(XIX.4)
  - 에이전트 모드
  - 앱 설정
  - terminal profile
- **외부 클라이언트 연결 정보**: `stellar-mcp --print-launch`가 MCP 클라이언트 설정에 넣을 실행 경로와 인자를 JSON으로 출력합니다. 클라이언트 등록은 사용자가 각 클라이언트 문서에 따라 직접 하며, Stellar는 다른 도구의 설정을 바꾸지 않습니다.

### 4. L3: 내장 에이전트

#### 4.1 제품 경계(§31.1 명확화)

- Stellar는 다음을 두지 않습니다: 모델 제공자 API client, API key 입력·저장, 모델 라우팅, 과금·인증, 자체 agent loop.
- 내장 에이전트는 사용자가 설치하고 직접 인증한 공식 CLI(모델 제공자가 배포하는 코딩 에이전트 CLI)를 subprocess로 실행합니다. agent loop와 모델 호출은 그 CLI가 수행합니다.
- 구독 인증은 전적으로 각 CLI가 맡습니다. Stellar 프로세스는 CLI 실행 파일을 실행하는 것 외에 CLI의 사용자 데이터 위치를 열지 않습니다. 이것은 §37.3 "Stellar는 비밀을 소유하지 않음"의 적용입니다.
  - 연결 상태는 세 값입니다: 실행 가능 / 버전 미지원 / 인증 필요(CLI가 보고).
  - 인증이 필요하면 설정 화면은 공식 절차 안내와 로그인용 내장 터미널 열기만 제공합니다.
- 사용자 문장은 그대로 CLI에 전달합니다. 시스템 프롬프트 템플릿은 저장하지도 편집하지도 않습니다.
- 모델 이름은 CLI가 보고한 값을 표시합니다. 세션 중에 모델이 자동으로 바뀌지 않게 합니다.
- 비용과 사용량 한도는 표시하지 않습니다.

#### 4.2 세션 수명과 lease 소유

- 앱이 시작한 세션에서는 앱 쪽 AgentSession이 lease와 권한을 소유합니다.
  - 같은 session nonce로 다시 연결한 bridge는 그 세션에 다시 묶입니다.
  - 롤백 조건은 세션 중지, CLI 프로세스 트리 소멸, 재연결 유예 초과입니다.
- 외부 클라이언트는 bridge 종료 시 권한 폐기와 전체 롤백 규칙을 그대로 따릅니다.
- 이 규칙은 XIX.9.12가 XIX.1.4와 XIX.6.4를 대체해 정합니다.
- 세션은 시작한 창에 묶이고, 앱을 다시 시작하면 복원하지 않습니다.

#### 4.3 구독 CLI 실행 요구사항

- **실행 방식**: 셸을 거치지 않고 인자 배열로 실행합니다.
  - 환경 변수는 허용 목록으로 넘기며 모델 제공자 API key 변수는 넘기지 않습니다.
  - 작업 폴더는 프로젝트 working tree 밖의 사용자 전용 임시 폴더이며, 세션 수명이 끝나면 정리합니다.
  - 프로세스 정리는 독립 process group 단위로 합니다.
- **launch profile**: CLI 종류와 버전별로 Stellar가 소유한 fixture로 관리합니다. 각 profile은 다음을 보장합니다.
  - Stellar MCP tool 외의 쓰기 수단이 없습니다.
  - 사용자 전역 지침, 메모리, 확장이 세션에 들어오지 않습니다.
  - Stellar 외의 MCP 서버가 없습니다.
  - CLI 쪽 대화 기록이 남지 않습니다. 남는 CLI라면 종료 시 정리하고 사실을 고지합니다.
  - CLI가 제공하는 경우 벤더 텔레메트리를 끕니다.
  - session nonce는 CLI의 공식 MCP 설정 채널로만 넘깁니다.
- **실행 거부**: 검증되지 않은 CLI 버전은 실행하지 않습니다.
- **세부 값은 구현 작업에서 정함**: 구체 인자, 신호 순서, timeout 값, 버전 재확인 조건은 해당 WBS 구현 작업에서 Stellar 실측과 각 OS·CLI 공식 문서를 근거로 정하고 작업 보고서에 기록합니다.

#### 4.4 출력 표시

- 1단계에서는 CLI 출력을 텍스트로만 표시합니다. 공급자별 parser는 두지 않습니다.
- 2단계(XVIII.5.4 승인 시)에서는 공급자 출력의 표시 전용 디코드를 `stellar-agent-host`에만 둡니다.
  - 도구 사용 위치에 알 수 없는 종류가 나타나면 세션을 종료합니다(fail closed).
  - 시작 검사를 통과하기 전에는 attached backend가 문서 접근을 보류합니다.
  - 디코드 결과 타입에서 CommandTransaction이나 문서 변경으로 가는 변환 경로는 두지 않습니다. 이것은 타입 inventory와 전이 도달 금지 규칙으로 검사합니다.

#### 4.5 패널과 승인의 정책 경계

화면 배치는 WBS가 정합니다.
- **승인 화면 위치**: 네이티브 chrome(패널, 창 sheet 또는 dialog)에만 둡니다. 캔버스와 플러그인 WebView 안에는 그리지 않습니다.
- **승인 화면 내용**: 주 내용은 Stellar 코어가 만든 것입니다: semantic diff, validation 결과, 렌더 프리뷰, 잠금 범위. 에이전트가 붙인 작업 설명은 출처가 에이전트임을 표시한 보조 정보로 둡니다.
- **민감 확인**: Terminal, FileDelete, GitCommit, GitPush 확인은 플랫폼 HIG의 파괴적 동작 확인 패턴을 따르고, 키보드 기본 동작만으로 승인되지 않게 합니다. 보조 기술로 확정할 수 있어야 합니다.
- **승인 요청 저장**: 승인 요청은 ProjectIdentity 단위로 코어에 보관하고, 첫 응답 하나만 유효합니다. 응답이 없거나, 창이 닫히거나, 세션이 끝나면 거부로 처리하고 XIX.6.3으로 정리합니다.
- **모드**: Stellar 권한(XIX.3.1)의 상한이며, 사용자 UI에서만 바꿀 수 있습니다. 범위 승인은 모든 모드에서 필요합니다.

| 모드 | 권한 상한 |
|---|---|
| 읽기 전용 | Read |
| 검토 후 적용(기본) | Read, Propose (batch마다 승인) |
| 승인 범위 내 바로 적용 | Read, Propose, Apply |

- **내장 세션 카탈로그**: Terminal, FileDelete 계열 tool을 노출하지 않습니다. GitCommit과 GitPush는 개별 확인으로만 허용합니다.
- **세션 시작 조건**: 신뢰된 프로젝트에서만 시작합니다. 시작 확인 화면에 대상 프로젝트, 전송 대상 CLI, 모드를 표시하고, 이 확인을 첫 연결 승인으로 기록합니다. 신뢰를 철회하면 즉시 중지·롤백·거부합니다.
- **중지**: "현재 세션 중지"와 "이 프로젝트의 에이전트 lease 모두 해제"를 둡니다. 후자는 그 프로젝트의 모든 lease를 XIX.6.3 절차로 롤백하고, 앱과 연결된 세션을 끝내며, 대기 요청을 거부로 닫습니다. 대상은 앱과 연결된 세션뿐입니다.
- **창과 앱 종료**: 활성 lease나 실행 중인 세션이 있으면 창 닫기와 앱 종료 전에 확인합니다.
- **알림**: OS 알림 본문에는 프로젝트 데이터를 넣지 않습니다(XVI.8 원칙과 같음).

### 5. 로컬 제어 채널과 신원 확인

- 네트워크 포트는 열지 않습니다(§31.2).
- bridge와 앱은 서로를 신뢰하기 전에 상대가 같은 OS 사용자이고 Stellar 배포 서명 주체인지 확인합니다. 확인 전에는 프로젝트 데이터나 session nonce를 보내지 않습니다.
- endpoint는 현재 사용자만 접근할 수 있는 OS 기본 위치에 둡니다.
- macOS에서는 상대 프로세스의 코드 서명(배포 서명 주체)을 1차 근거로 씁니다.
- 개발 빌드 예외는 Release 빌드에 존재하지 않는 compile-time 설정으로만 둡니다.
- 앱이 시작한 bridge에 대한 nonce(XIX.1.3) 처리:
  - 프로세스 목록에 노출되지 않는 채널로만 전달합니다.
  - 연결 상대가 그 세션이 만든 프로세스 트리 안에 있을 때만 그 세션으로 라우팅합니다.
  - nonce는 권한이 아니며 XIX.3 승인을 대체하지 않습니다.
- 퍼미션 비트, 경로 규칙, 길이, 비교 방식, 사용할 OS API는 WBS 구현 작업에서 각 OS 공식 문서를 근거로 정합니다.

### 6. 스레드와 성능

- 앱에 "Agent Host·IPC I/O 스레드"를 둡니다. 이 스레드는 문서를 쓰지 않습니다.
- 검증된 Command는 일반 DocumentThread queue에 사용자 입력보다 낮은 priority로 들어갑니다. queue가 넘치면 tool 결과로 busy를 돌려줍니다.
- lease 대상 계산과 batch 적용은 UI·문서 스레드 단일 점유 상한(§6.3)을 넘지 않게 나눠 처리합니다.
- 서명 확인처럼 느릴 수 있는 호출은 UI·문서 스레드 밖에서 합니다.
- 승인 이벤트는 기존 FFI pull 방식 event 경로로 셸에 전달합니다.

### 7. 배포

- **macOS**
  - 앱 번들의 보조 실행 파일 위치(Contents/Helpers)에 `stellar-cli`와 `stellar-mcp`를 별도 실행 파일로 둡니다.
  - 안쪽 코드부터 hardened runtime과 timestamp로 개별 서명하고, helper에는 앱 entitlement를 주지 않습니다. 앱과 함께 공증합니다.
  - 기존 릴리스 서명 스크립트의 "중첩 서명 대상 없음" 전제를 바꿔야 합니다.
- **Windows**: ADR-0006에 따라 보류합니다.
- **headless·CI 머신**: `stellar-cli` 단독 서명 배포물을 제공하며 `stellar-mcp`는 넣지 않습니다.
- **초기 릴리스**: 보조 실행 파일과 alias를 넣지 않습니다.
- **비밀 관리**: 서명 인증서, 팀 식별자, 공증 자격 증명은 저장소에 두지 않습니다.

### 8. crate 배치

| crate | 위치 | 허용 직접 의존 | 전이적으로도 도달 금지 |
|---|---|---|---|
| `stellar-ops` | `crates/ops` | project, document, commands, history, storage, semantic-diff, semantic-merge, diagnostics | local-ipc, agent-protocol, mcp, agent-host, ffi |
| `stellar-cli`(bin) | `crates/cli` | ops, diagnostics | local-ipc, agent-protocol, mcp, agent-host |
| `stellar-local-ipc` | `crates/local-ipc` | core-types | document, commands, ops |
| `stellar-agent-protocol` | `crates/agent-protocol` | core-types | commands, document, ops, agent-host |
| `stellar-mcp`(library) | `crates/mcp` | ops, agent-protocol, local-ipc, diagnostics | agent-host, ffi |
| `stellar-agent-host` | `crates/agent-host` | core-types, agent-protocol, local-ipc, diagnostics | document, commands, history, storage, ops |
| `stellar-ffi` | `crates/ffi` | core-types, agent-host, agent-protocol, ops, 에이전트 편집 구현 crate | (타입 변환 금지 inventory로 보완) |

- attached tool 호출은 `stellar-agent-protocol`이 정의한 host trait로 처리하고, 앱 runtime이 구현합니다.
- XIX.4~XIX.6의 구현 crate는 그 작업에서 정하되, `stellar-agent-host`에서 그 crate로 도달하지 않게 합니다.
- `render_preview`는 attached 전용입니다.

### 9. 구현 세부 결정 원칙

- ADR에는 경계와 불변식만 둡니다.
- 수치(길이, 한도, timeout), 퍼미션 비트, 신호 순서, 캐시 무효화 조건, CLI 인자 목록은 해당 WBS 구현 작업에서 Stellar 실측과 OS·CLI 공식 문서를 근거로 정하고 작업 보고서에 기록합니다.

## Alternatives

- **API key를 받아 Stellar가 직접 tool loop를 도는 방식**: 자체 런타임과 비밀 저장소가 필요해 §4.3과 §37.3에 위배됩니다. 채택하지 않습니다.
- **Stellar가 모델 제공자 인증 흐름에 직접 참여하는 방식**: §31.1의 "모델 선택·과금·인증 없음"과 §37.3에 위배됩니다. 채택하지 않습니다.
- **CLI를 내장 터미널에서만 대화형으로 실행하는 방식**: 공급자 출력 처리가 필요 없습니다. 하지만 세션 목록, 단계 표시, 작업 대상 지정, 승인 연계를 제공할 수 없습니다. 보조 경로로는 계속 지원합니다.
- **`stellar-cli`를 실행 중 앱의 얇은 클라이언트로 만드는 방식**: 파일이 원본이고 CI와 merge driver가 앱 없이 돌아야 한다는 조건에 맞지 않습니다. 채택하지 않습니다.
- **CLI와 MCP를 한 실행 파일로 합치는 방식**: L1이 L2 코드를 품게 됩니다. 배포 서명 주체 확인만으로 같은 출처임을 검증할 수 있으므로 이점도 작습니다. 채택하지 않습니다.
- **headless backend에서 쓰기를 허용하는 방식**: 앱의 승인·lease를 우회하고, 쓰기 허용 스위치가 에이전트가 고칠 수 있는 파일에 남게 됩니다. 채택하지 않습니다.
- **앱이 프로젝트를 열고 있으면 `stellar-cli` 쓰기를 거부하는 방식**: 외부 변경 원칙(§22.5)과 일관성이 없고, 앱이 열린 동안의 Git merge를 막습니다. 채택하지 않습니다.

## Consequences

- **대체되는 행**: XVIII.5.2(→ XVIII.5.4), XIX.1.4와 XIX.6.4(→ XIX.9.12), XIX.3.3(→ XIX.9.11)은 계획서 cancelled 표기를 지원하는 도구 작업(XXIX.1.1)이 끝난 뒤 `cancelled`로 바꾸고 대체 ID를 연결합니다.
- **유지되는 행**: XIX.1.1, XIX.1.5, XVI.9.3, XVI.11.8은 그대로 둡니다.
- **crate graph 검사기 확장**: 전이 도달 금지 규칙과 target·build·dev 의존 파싱을 추가합니다.
- **새 외부 의존성**: JSON 직렬화, JSON-RPC 프레이밍, 서명 확인 binding 등은 승인 목록에 등록해야 합니다.
- **유지보수 비용**: CLI 버전이 바뀔 때마다 launch profile fixture를 갱신해야 합니다.

## Compatibility

- 문서 의미와 파일 포맷은 바뀌지 않습니다.
- writer 종류 metadata는 storage의 generation metadata에 추가되며, 기존 generation은 앱 창 writer로 해석합니다.
- 잠금, 세션, 권한, transcript는 프로젝트 파일과 Git에 저장하지 않습니다.

## Performance

- CLI 프로세스와 IPC는 핵심 입력·편집·렌더링 경로 밖에 있습니다.
- 에이전트 Command는 사용자 입력보다 낮은 priority로 일반 queue에서 적용되고, 큰 작업은 나눠 처리합니다.

## Security

- 네트워크 포트가 없습니다. 신원 확인은 양방향이고, 확인 전에는 데이터를 보내지 않습니다.
- 권한은 세션 한정입니다. 파일에서 읽은 설정은 능력을 좁힐 수만 있고 넓힐 수 없습니다.
- headless backend는 읽기 전용입니다.
- 내장 세션에는 Terminal과 FileDelete가 없습니다.
- 모든 판단은 fail closed입니다.
- 벤더 자격 증명은 다루지 않습니다.
- 에이전트 입출력은 진단, 텔레메트리, 복구 snapshot에 넣지 않습니다.

## Migration

없습니다. 적용 순서는 다음과 같습니다.
1. 현재(Phase VI 진행 중): 이 ADR(Proposed)만 추가합니다.
2. ADR Accepted 후: 아키텍처 본문과 계획서를 개정하고 신규 WBS를 추가한 뒤 ledger를 다시 생성합니다.
3. XXIX.1.1~XXIX.1.3 완료 후: 대체 대상 행을 `cancelled`로 바꾸고 대체 ID를 연결합니다.
4. Phase VII: CommandTransaction closed enum과 에이전트 출처 표기를 유지합니다.
5. Phase XII: 인스펙터 영역 탭 패널 자리와 drawable 갱신 정책을 고려합니다.
6. 2026-11-14 제품 소유자 지시(최대 병렬 진행)에 따라 L1 골격(XXIV.1.1~XXIV.1.2)과 L2 골격(XIX.1.1~XIX.1.3, XIX.1.5)은 G8 이전에 착수합니다. 이 골격에는 쓰기 tool과 내장 에이전트가 없습니다.
7. G8 이후: Phase XVIII → XXIV.1·XXIV.4 나머지(L1) → XIX 나머지(XIX.1.6~1.12, XIX.9 포함, L2) → XXIX.2~XXIX.3(L3 코어·FFI) → XXIX.4(macOS) → XXIX.6(배포) → XXIX.7(E2E).

모두 초기 릴리스 빌드에는 노출하지 않습니다.

## Open Questions

Accepted로 바꾸기 전에 제품 소유자가 정할 항목입니다.

1. **벤더 약관**: 공식 CLI를 앱이 subprocess로 실행하고 launch profile로 설정을 고정하는 방식이 각 CLI 약관에서 허용되는지 확인합니다.
2. **출력 표시**: 1단계(텍스트만) 이후 2단계(표시 전용 디코드)까지 갈지 정합니다.
3. **headless 읽기 backend**: XIX.3.3("trusted project라도 별도 승인 없이는 Read 불가")과 달리, 신뢰된 프로젝트 하나에 고정한 읽기 전용 backend를 허용할지 정합니다.
4. **lease 소유자**: 앱이 시작한 세션은 AgentSession이 lease를 소유하고 재연결 유예를 둘지, XIX.1.4·XIX.6.4대로 bridge 종료 즉시 롤백할지 정합니다.
5. **계획서 `cancelled` 표기**: 대체되는 행을 표에서 `cancelled`로 표시하는 도구 지원(XXIX.1.x)을 도입할지 정합니다.
6. **커밋 형식**: 계획서 개정 기록("`Task(...)`만 허용"), `AGENTS.md`(`Docs:`·`Chore:` 등 허용), `cargo xtask validate-commit-message`(`Task(...)`만 허용)가 서로 다릅니다. 하나로 맞춥니다.
7. **PATH 노출**: macOS에서 앱 번들 안의 `stellar-cli`를 터미널에서 부르는 방법(사용자 승인 설치 명령, 수동 안내 등)을 정합니다.
8. **이름**: `stellar-cli`라는 이름은 다른 생태계의 기존 CLI와 겹칠 수 있습니다. 배포 이름(실행 파일·패키지)을 확정합니다.
