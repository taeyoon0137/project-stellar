# ADR-0109: 로컬 진단 store, 금지 필드 scanner, 텔레메트리 동의와 전송 대기열

## Status

Accepted

## Context

아키텍처 §39는 기본으로 로컬 진단만 저장하고, 사용자가 명시적으로 동의할 때만 비식별 성능 수치, 앱 충돌 정보,
렌더러 오류 코드, 기능별 실패 카운트를 보낼 수 있다고 정합니다. 프로젝트 내용, 텍스트, 레이어·토큰 이름, 파일
경로, Git URL·branch·commit, 터미널 입출력, MCP·에이전트 입출력은 수집하지 않습니다. 사용자는 전송 예정 데이터를
확인하고, 진단 번들을 수동으로 내보내고, 동의를 철회할 수 있어야 합니다.

계획 XVI.8.1\~XVI.8.6이 이 일을 나눕니다. II.3.2는 민감 값 래퍼(`Sensitive`)와 비민감 이벤트 타입을, XVI.1.4·XVI.1.5는
상호작용 trace의 privacy schema와 ring buffer store를 이미 만들었습니다. 다음이 정해지지 않았습니다.

- 로컬 진단 store의 record 종류, field, 파일 형식, 위치, 용량, 보존 기간.
- 진단 번들에서 금지 정보를 찾는 규칙과 그 결과를 보고하는 방식(보고가 금지 정보를 다시 내보내면 안 됩니다).
- 동의 상태의 전이 규칙과, 동의 파일이 없거나 손상됐을 때의 처리.
- 사용자에게 보여 주는 preview와 실제로 보내는 bytes가 같다는 보장.
- 동의 철회 때 이미 대기열에 있던 전송의 처리.
- 초기 출시에서 꺼진 기능(내장 터미널, MCP)의 field가 schema에 없음을 검사하는 방법.

제품 방향(2026-11-23)에 따라 지금은 엔진만 만들고 UI는 나중에 다시 설계합니다. 실제 네트워크 전송은 이 결정의
범위 밖입니다.

## Decision

1. **record 네 종류와 줄 형식(XVI.8.1).** `stellar_diagnostics::store`가 아키텍처 §39 전송 가능 항목을 record로
   정의합니다.
   - `crash`(시각, 충돌 종류, 코드, 스레드 역할, 가동 시간), `renderer_error`(시각, backend, 단계, 코드),
     `perf`(시각, 상호작용 종류, 문서·가시 node bucket, cache 상태, Logical Interaction Latency, GPU 시간),
     `feature_failure`(기능, 코드, 횟수, 첫·마지막 시각)입니다.
   - 모든 field는 정수, 닫힌 enum 이름(`[a-z][a-z0-9_]*`), 숫자 version(`a.b.c`)뿐입니다. 런타임 문자열 field가
     없으므로 금지 정보를 넣을 자리가 타입에 없습니다. 성능 record는 trace ID와 stage 시각을 남기지 않고 bucket과
     지연만 남깁니다.
   - 한 줄이 record 하나(`<tag> <field>=<value> ...`)이고 field 이름·순서는 `RecordSchema`가 고정합니다. 정수는 앞자리
     0 없는 10진수 하나만 정규형입니다. store 파일, 진단 번들, scanner 구조 검사가 같은 schema와 codec을 씁니다.
   - 렌더러 backend는 `metal`뿐입니다(ADR-0006 Windows 지원 보류). 기능 목록은 초기 출시 기능만 둡니다.
2. **store 위치·용량·보존.** store는 프로젝트 폴더와 Git 밖의 `<OS 사용자 데이터>/xyz.taeyoon.stellar/diagnostics/`
   아래 `store.stldiag`입니다(신뢰 저장소·잠금과 같은 앱 데이터 디렉터리). 디렉터리는 `0700`, 파일은 `0600`이고
   임시 파일에 쓰고 fsync한 뒤 rename하고 디렉터리를 fsync합니다(철회 때 동의 파일 교체가 대기열 삭제보다 먼저 남음). event record는 기본 2,048개(상한 65,536개)이고 가득 차면 가장 오래된
   것부터 지웁니다. 실패 카운트는 `(기능, 코드)`마다 counter 하나로 합치고 1,024쌍까지 둡니다. 넘친 수는
   `meta dropped=`에 셉니다. 기본 보존 기간은 30일이고 `prune`이 지웁니다. 보존 기간이 지난 record는 preview·
   내보내기·전송 대기열에 넣지 않습니다(`stellar diagnostics preview|export|enqueue`는 읽은 store를 메모리에서
   `prune`한 뒤 번들을 만들고, store 파일은 바꾸지 않습니다). 읽을 파일 크기 상한은 8 MiB입니다.
3. **오류는 내용을 되풀이하지 않습니다.** 형식 오류는 줄 번호와 정적 종류(모르는 tag, field 이름, 값 등)만 냅니다.
4. **금지 필드 scanner(XVI.8.2).** `stellar_diagnostics::scanner`는 세 층으로 검사합니다.
   - 내용 규칙: 사용자 홈·볼륨·임시 경로와 경로 구분자, URL scheme·scp 형식 Git 원격, `refs/heads/`·`origin/` 같은
     branch 표기, 숫자와 문자가 섞인 7\~64자 hex나 21자 이상 숫자열 commit, 제어 문자·ANSI escape·shell prompt·shell 이름
     같은 터미널 입출력, `jsonrpc`·`tools/call`·`"method"` 같은 MCP·에이전트 payload, 비ASCII 텍스트·따옴표 같은
     프로젝트 텍스트, 자격 증명 표기(`Bearer `, `sk-`, `password` 등)입니다. 규칙마다 금지 범주가 정해져 있습니다.
   - 문맥 값: 호출자(앱)가 아는 열린 프로젝트의 이름·경로·텍스트·레이어·토큰 이름, Git 원격·branch·commit을
     `ScanContext`로 주면 그 값이 나타나는지 봅니다. 4 bytes 미만이거나 schema 어휘(tag·field·enum 이름, 정수,
     `none`, version)와 같은 값은 공개 어휘와 구별할 수 없으므로 문맥 비교에서 뺍니다. 값이 공개 조각(형식 이름 줄
     전체, 또는 공백·`=`로 나눈 token 중 schema 어휘·정수·`none`·version인 것) 하나의 일부로만 나오면 탐지하지
     않습니다. 예를 들어 프로젝트 이름 `Stellar`는 형식 이름 안에, 레이어 이름 `Time`은 `uptime_ms` 안에 나오지만
     프로젝트 값이 새어 나온 것이 아닙니다. 그렇지 않으면 그런 이름의 프로젝트를 연 동안 모든 번들이 거부됩니다.
     비교에서 빠지거나 공개 조각 안에 숨은 값도 어긋난 자리에 들어오면 구조 검사가 막습니다.
   - 구조 검사: 번들은 결정 5의 형식과 schema에 정확히 맞아야 합니다. 맞지 않는 줄은 `unrecognized`입니다.
   - 결과(`Finding`)는 줄 번호, 범주, 규칙 이름만 담습니다. 찾은 문자열 자체는 담지 않습니다.
   - 검증은 범주별 secret corpus를 번들의 여러 위치(값, 덧붙인 token, 새 줄, key)에 넣어 전부 탐지(100%)하고,
     모든 enum 값을 쓴 정상 번들에서 탐지 0건인지 봅니다.
5. **동의 상태(XVI.8.3).** `stellar_diagnostics::consent::ConsentState`는 `NotAsked`(기본), `Declined`, `Granted`입니다.
   - 사건은 `Grant`, `Decline`, `Revoke`입니다. `NotAsked`·`Declined`에서 `Grant`는 `Granted`, `Decline`은
     `Declined`입니다. `Granted`에서 `Decline`·`Revoke`는 `Declined`입니다. 아직 동의하지 않은 상태의 `Revoke`는
     거부합니다(철회할 동의가 없음). 같은 상태로의 전이는 변화 없음으로 돌려줍니다.
   - 자동 전송은 `Granted`에서만 허용합니다(`allows_automatic_upload`).
   - 동의 파일(`diagnostics/consent`)이 없으면 `NotAsked`이고, 손상됐으면 `NotAsked`로 읽고 손상을 알립니다(닫힘:
     전송하지 않음). 동의 상태는 프로젝트 파일·Git에 두지 않습니다.
6. **진단 번들과 preview(XVI.8.4).** `stellar_diagnostics::bundle`이 번들 형식 하나를 정합니다.
   - `stellar.diagnostic-bundle v1` 줄, `env app_version= os_version= arch=` 줄, `meta dropped=` 줄, record 줄들,
     `end records=<n>` 줄입니다. 운영체제 version은 macOS `SystemVersion.plist`의 `ProductVersion`을 숫자로만
     읽고, 읽지 못하면 `0.0.0`입니다.
   - 수동 번들 내보내기와 전송 payload는 같은 bytes입니다. 둘 다 `preview_payload`가 만들고, 이 함수는 만든 bytes를
     scanner로 검사해 탐지가 있으면 거부합니다. 사용자가 확인한 것과 보내거나 내보내는 것이 다를 수 없습니다.
   - `PayloadPreview`는 `preview_payload`만 만들 수 있고, 전송 대기열은 `PayloadPreview`만 받으며, 전송자는 그
     bytes를 그대로 받습니다(타입으로 강제).
   - 수동 내보내기는 사용자가 직접 하는 로컬 파일 쓰기라 동의 상태와 무관합니다. 이미 있는 파일은 덮어쓰지
     않습니다.
7. **전송 대기열과 철회(XVI.8.5).** `stellar_diagnostics::consent_gate::TelemetryGate`가 동의 상태와 대기열(최대
   16개)을 함께 둡니다.
   - `enqueue`와 `flush`는 `Granted`에서만 동작합니다. 다른 상태에서는 거부하고 전송자를 부르지 않습니다.
   - `Granted`를 떠나는 전이(철회·거절)는 대기열을 모두 버리고 버린 수를 돌려줍니다. 이후 전송은 다시 `Grant`할
     때까지 막힙니다. 로컬 store는 지우지 않고 결정 2의 보존 정책을 따릅니다.
   - 영속 대기열은 `diagnostics/outbox`입니다. 철회는 먼저 동의 파일을 `Declined`로 쓰고 대기열 파일을 지웁니다.
     중간에 끊겨 대기열 파일이 남아도, 열 때 동의가 `Granted`가 아니면 대기열을 버립니다.
   - 네트워크 전송자는 구현하지 않습니다. `UploadTransport` trait만 두고 테스트는 기록용 전송자를 씁니다.
8. **feature inventory(XVI.8.6).** `stellar_diagnostics::store_inventory`가 store·번들 schema의 모든 tag, field 이름,
   enum 이름을 모으고, 초기 출시에서 꺼진 기능의 어휘(터미널·PTY·shell·MCP·JSON-RPC·에이전트·prompt·transcript·
   표준 입출력 등)가 하나도 없는지 검사합니다. schema 정의에는 `cfg(feature)`가 없어서 기능을 끈 빌드와 켠
   빌드의 schema가 같고, 소스 검사로 이를 확인합니다. 나중에 에이전트 관련 금지 field(XXIX.6.4)는 이 어휘 목록과
   scanner corpus에 더합니다.
9. **CLI.** `stellar diagnostics`가 `schema`, `scan <파일>`, `preview`, `export --output <파일>`,
   `consent [show|grant|decline|revoke]`, `enqueue`를 제공합니다. 기본 디렉터리는 결정 2의 위치이고
   `--data-dir`로 바꿀 수 있습니다. 모든 결과는 기존 결과 envelope JSON이고, `preview`의 `payload`는 내보내기 파일
   bytes와 같습니다. `scan`은 탐지가, `schema`는 위반이 있으면 1이고, `consent revoke`는 철회할 동의가 없으면 1,
   `enqueue`는 동의가 없으면 77(`noperm`), 대기열이 가득 차면 75(`tempfail`), `export`는 파일이 있으면
   73(`cantcreat`)입니다.

## Alternatives

- **JSON store·번들:** 사람이 읽기 쉽지만 diagnostics crate에 JSON 의존을 더하거나 따로 writer를 둬야 하고, 문자열
  field를 허용하는 형식이라 구조 검사가 약해집니다. 닫힌 줄 형식을 택했습니다.
- **trace ring buffer(XVI.1.5)를 그대로 내보내기:** trace에는 trace ID와 stage 시각이 있어 행동 순서를 복원할 수
  있습니다. 전송 가능 항목은 bucket과 지연 요약으로 충분합니다.
- **scanner를 정규식 없이 문맥 값 비교만으로:** 앱이 모르는 값(다른 프로젝트 경로 등)을 놓칩니다. 패턴 규칙,
  문맥 값, 구조 검사 세 가지를 함께 둡니다.
- **동의 손상 시 마지막 상태 추정:** 잘못 추정하면 동의 없이 전송할 수 있습니다. 닫힘(`NotAsked`)을 택했습니다.
- **수동 번들과 전송 payload를 다르게(번들에 더 많은 정보):** 사용자가 확인한 것과 보내는 것이 달라지고 검사
  경로가 둘이 됩니다. 하나로 합쳤습니다.

## Consequences

- 진단 record에는 금지 정보를 넣을 field가 없고, 그래도 섞여 들어온 bytes는 scanner가 막습니다.
- preview와 전송 bytes가 같다는 것이 타입으로 보장됩니다.
- 철회 뒤에는 대기열이 비고 전송자가 불리지 않습니다.
- 닫힌 enum이라 새 값(예: D3D12 backend, 새 기능)은 schema 변경이고 이전 reader는 그 줄을 거부합니다. 새 값은
  version을 올리지 않고 더할 수 있지만, 이전 앱은 새 store를 읽지 못합니다. 그때는 store를 버리고 새로 시작합니다
  (로컬 진단이라 데이터 손실 영향이 작습니다).
- 앱 연결(충돌 handler, 렌더러 오류 보고, 기능 실패 집계, 동의 화면)은 UI 재설계 때 합니다.

## Compatibility

- 프로젝트 파일 형식, 공개 FFI, 보호 경로는 바꾸지 않습니다.
- store·동의·대기열 파일은 앱 데이터 디렉터리의 로컬 파일이고 형식 이름에 version(`v1`)을 둡니다.
- CLI에 `diagnostics` 명령을 더합니다. 기존 명령과 종료 코드 계약은 바꾸지 않습니다.

## Performance

- record 추가는 O(1)(counter는 O(log n))이고 store 메모리는 용량으로 묶입니다.
- scanner는 번들 bytes에 선형입니다(번들 상한 8 MiB).
- 상호작용 경로에는 들어가지 않습니다.

## Security

- 진단 파일은 소유자 전용 권한입니다.
- scanner 결과와 형식 오류는 찾은 문자열을 되풀이하지 않습니다.
- 동의 파일 손상·부재는 전송 금지로 닫힙니다. 철회는 대기열 파일을 지우고, 남은 대기열도 열 때 버립니다.
- 네트워크 코드가 없으므로 이 결정으로 외부 전송이 생기지 않습니다.

## Migration

- 새 기능이라 기존 데이터가 없습니다. 이전 형식 파일은 없습니다.
