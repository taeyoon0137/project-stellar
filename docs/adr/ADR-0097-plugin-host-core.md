# ADR-0097: 플러그인 코어를 package 검증·적재 gate·IDL Host API·async batch RPC로 세움

## Status

Accepted (2026-11-23). 엔진 우선 방향(2026-11-23, UI는 나중에 다시 설계)에 따라 `plugin-host` lane이 정했습니다.

- 기술 아키텍처 §33(플러그인 로드맵)과 §37.3(플러그인 권한은 최소 권한과 세션 단위 승인)을 구현 수준에서 구체화합니다.
- 보호 경로 `schemas/`에 `schemas/plugin-manifest.schema.json`(생성)과 `schemas/plugin-host-api.idl.json`(원본)을 둡니다.
- WKWebView·WebView2 생성, 셸 FFI, Swift UI는 이 ADR 범위가 아닙니다(XXI.2, XXI.3, XXI.5.3은 WebView 연결 때 정합니다).

## Context

- 계획서 Phase XXI는 HTML/JS UI(WebView), JS↔WASM 직접 경로, 권한형 Host RPC를 요구합니다. 지금은 엔진만 만들고 UI는 나중에
  처음부터 설계합니다. 그래서 WebView 없이 Rust 테스트로 검증할 수 있는 부분(package·manifest, 적재 gate, Host API 정의와
  binding, RPC 코어, 권한 저장, 외부 process 수명)을 먼저 세웁니다.
- 핵심 가치 중 AI 친화와 Git 협업에 따라 플러그인도 사람·CLI·MCP와 같은 엔진 경로(ops)로 문서를 읽고 바꿔야 합니다.
  플러그인 전용 판정·해석 경로를 따로 만들면 결과가 갈라집니다.
- 기존 crate 배치(ADR-0005 §8): runtime은 mcp-core에 도달하지 않습니다. plugin host는 앞으로 앱 runtime이 쓸 crate이므로 L2
  MCP crate에 의존할 수 없습니다. 외부 JSON crate는 아직 도입하지 않았습니다(XXIX.1.2).
- 신뢰하지 않은 플러그인 package는 적대적 입력입니다(§37.2 깊이·개수 제한, 경로 탈출).

## Decision

1. **crate와 생성 파일.** 새 crate `crates/plugin-host`(`stellar-plugin-host`)를 둡니다. 허용 직접 의존은 `stellar-ops`(Host API가
   부르는 L1 진입점), `stellar-project`(제한 YAML loader 재사용), `stellar-storage`(package digest용 `ContentHash`), `stellar-git-porcelain`(Git 신뢰 gate `GatedGitRunner`)뿐입니다.
   `git.status`는 session이 이미 확인한 신뢰(`TrustedProject`)로 gate를 열고 `ops::git::GitSession`을 부릅니다.
   - `plugin.yaml` JSON Schema(`schemas/plugin-manifest.schema.json`)는 `manifest.rs` 상수에서 생성합니다. Host API는 IDL
     `schemas/plugin-host-api.idl.json`이 원본이고 Rust(`crates/plugin-host/src/generated/host_api.rs`)·TypeScript
     (`crates/plugin-host/bindings/stellar-host-api.ts`) binding을 생성합니다.
   - 생성기는 `stellar-plugin-bindgen` 실행 파일이고, `tests/schema_drift.rs`가 저장소 파일과 byte 단위로 비교합니다(drift 0).
   - JSON codec은 mcp-core `json.rs`의 의도적 사본이며 `tests/json_copy_sync.rs`가 본문 일치를 검사합니다.
2. **package와 manifest(XXI.1.1).** package는 이름이 `.stellar-plugin`으로 끝나는 디렉터리이고 최상위에 `plugin.yaml`이 있습니다.
   - 필드: `manifest_version`(현재 1), `id`(소문자 reverse-DNS, 두 label 이상, 128 bytes 이하), `version`(SemVer core와 선택
     pre-release, build metadata 없음), `name`(표시용), `entry`(package 상대 `.html`), `host_api`(요구 major version),
     `permissions`(닫힌 권한 이름 목록, 중복 없음), 선택 `wasm`(`- path: <상대 .wasm>`, 16개 이하).
   - 모르는 key, 형식 오류, 지원하지 않는 `manifest_version`은 거부합니다(fail closed). 스칼라는 제한 YAML 규칙대로 문자열로 읽고
     엄격 해석합니다.
   - package 상대 경로는 `PackagePath::parse` 하나로만 해석합니다(manifest 필드, asset 요청, 플러그인 data 파일 경로 모두).
     빈 경로, 절대 경로, `..`, `.`·빈 구성 요소, `\`, `:`, 제어 문자, 255 bytes 초과, 32단계 초과를 거부합니다.
3. **package 검증(XXI.1.2).** `PluginPackage::open`은 package 전체를 `symlink_metadata`로 훑습니다.
   - package root 자신이 symlink이면 거부합니다. package 안 symlink는 대상이 안이든 밖이든 모두 거부합니다(대상이 밖이거나
     깨졌으면 `symlink-escape`, 안이면 `package-symlink`). WebView scheme handler가 나중에 파일을 열 때 링크를 해석하는 경로와
     TOCTOU 여지를 없애려고 전면 금지를 택했습니다. 프로젝트 폴더의 symlink 전면 허용(§19.3)과 다른 이유는 package가 신뢰하지
     않은 실행 코드 묶음이기 때문입니다.
   - 일반 파일·디렉터리 외 항목(FIFO 등), 경로 규칙에 맞지 않는 이름, 항목 4,096개·전체 256 MiB·파일 64 MiB 초과를 거부합니다.
   - `entry`와 `wasm[].path`는 package 안 일반 파일이어야 하고 WASM은 magic `\0asm`·version 1로 시작해야 합니다.
   - `host_api`가 이 빌드의 Host API major(IDL `host_api_version`)와 다르면 `unsupported-host-api`입니다.
   - 플러그인 디렉터리 하나를 훑는 `scan_plugins`는 같은 id를 가진 package를 **모두** 거부합니다(`duplicate-id`). 어느 쪽을
     믿을지 정할 근거가 없기 때문입니다.
   - package digest는 (상대 경로, 내용 hash) 목록의 sha256입니다. 서명 gate와 권한 저장이 이 값을 씁니다.
4. **적재 gate(XXI.1.3).** 서명·registry 검증은 `SignatureGate` trait으로 분리합니다. 이 lane은 registry가 없으므로
   `NoSignatureRegistry`만 둡니다: 서명 파일(`plugin.sig`)이 없으면 unsigned, 있으면 검증할 수 없음(invalid)입니다.
   - `LoadPolicy`의 기본값은 release입니다. release는 검증된 서명만 받습니다. unsigned local plugin은 development 모드에서만
     받습니다. invalid 서명은 어느 모드에서도 거부합니다.
   - development 모드는 호출자(앱)가 사용자의 명시 개발자 설정으로만 켭니다. library는 모드를 추측하지 않습니다.
   - Host session은 `admit`이 만든 `AdmittedPlugin`으로만 시작할 수 있습니다(타입으로 gate를 강제).
5. **권한(XXI.4.5).** 권한은 닫힌 enum입니다: `document.read`, `document.apply`, `git.read`, `file`, `network`, `terminal`,
   `clipboard`.
   - 유효 권한 = manifest 선언 ∩ 사용자 승인입니다. 선언하지 않은 권한은 승인할 수 없습니다.
   - 승인 범위는 `session`(메모리, 앱 종료 시 사라짐)과 `persistent`(파일 저장) 둘입니다. 저장 파일은 앱 사용자 데이터의
     `plugins/grants.json`이며 plugin id별로 승인 권한과 승인 당시 package digest를 적습니다. digest가 바뀐(업데이트된) package는
     저장 승인을 쓰지 못하고 다시 승인받아야 합니다. 쓰기는 임시 파일 뒤 rename입니다.
   - 권한 검사는 method 실행 전에 dispatcher 한 곳에서만 합니다. 권한이 필요한 method는 IDL에 적고 생성 Rust가 표로 냅니다.
6. **Host API IDL(XXI.4.1·XXI.4.2).** IDL은 JSON 문서입니다: `idl_version`, `host_api_version`, 이름 붙은 record type,
   namespace별 method(이름, 권한, 매개변수 이름·형식·필수 여부, 결과 type).
   - v1 namespace: `host`(info, 권한 없음), `document`(read_snapshot), `query`(tree_page, node, tokens, components), `command`
     (apply), `git`(status), `file`(read, write, list), `network`(fetch). 각 namespace는 서로 다른 권한에 묶입니다.
   - 문서·Git method는 기존 ops 공개 함수만 부릅니다: `query::project_structure`, `query::document_tree_page`,
     `query::node_detail`, `query::tokens`, `query::components`, `apply::apply`(command 해석은
     `mcp_command_input::command_input`), `git::GitSession::status`. plugin host에 판정·해석 사본을 두지 않습니다.
   - 매개변수 검사는 생성 표(`ParamSpec`)로 dispatcher가 한 번 합니다(모르는 key, 필수 누락, 형식).
   - 고빈도 namespace(`pointer`, `render`, `frame`, `input`, `animation`)는 IDL에 둘 수 없습니다(IDL lint test).
   - TypeScript binding은 type 선언과 `createHostClient(transport)`(id 부여, 응답 대응, batch, chunk 재조립)를 냅니다.
7. **RPC(XXI.4.3).** 메시지는 JSON object입니다.
   - 요청 `{"stellar_rpc":1,"id":…,"method":"ns.name","params":{…}}`, batch `{"stellar_rpc":1,"id":…,"batch":[{"method":…,"params":…},…]}`.
     응답은 같은 `id`에 `result` 또는 `error{code,message}`, batch는 항목별 결과 배열입니다. batch 항목은 순서대로 실행하고 앞
     항목 실패가 뒤 항목을 멈추지 않습니다.
   - 비동기: `HostSession::spawn`이 session마다 worker thread 하나를 두고 `submit`은 즉시 돌아옵니다. 응답은 channel로
     받습니다. 한 session 안의 실행 순서는 제출 순서입니다(쓰기 순서 결정성).
   - 상한: 메시지 1 MiB, batch 64항목, 처리 중 메시지 16개, 1초 창에 메시지 20개(batch는 1개로 셈). 넘으면 즉시
     `rate-limited`(`retry_after_ms`)나 `too-many-in-flight`입니다. 1초 20개는 60Hz pointer·render loop가 첫 1초 안에 막히는
     값입니다. 시간은 주입 가능한 `Clock`으로 재서 fixture가 결정적입니다.
8. **대용량 응답(XXI.4.4의 macOS 부분).** 직렬화한 응답이 256 KiB를 넘으면 `{"stellar_rpc":1,"id":…,"chunk":{"index","count","data"}}`
   조각으로 나눠 보냅니다. 조각은 UTF-8 경계에서 자르고 JS client가 이어 붙여 해석합니다. WKWebView reply handler 연결과
   Windows Shared Buffer는 WebView 연결 때 정합니다.
9. **플러그인 command(XXI.4.6).** `command.apply`는 `stellar apply`와 같은 `ops::apply::apply`와 같은 저장 잠금
   (`LockPolicy::Required`)을 씁니다. 그래서 같은 transaction의 semantic diff가 CLI 경로와 같습니다. 앱 DocumentThread·Undo
   경로 연결은 앱 attached 경로가 생길 때 합니다.
10. **플러그인 data namespace(XXI.6.2).** 플러그인 전용 파일은 프로젝트 루트의 `plugins/<plugin-id>/` 아래에만 둡니다(`file.*`).
    core 문서 schema·`design/`에 플러그인 데이터를 넣지 않습니다. 디렉터리는 Git이 보는 일반 파일이므로 협업 플러그인이
    공유할 수 있고, 지우거나 없어도 프로젝트는 그대로 열립니다. 경로는 `PackagePath` 규칙을 따르고 symlink를 따라가지 않습니다.
11. **외부 process 플러그인(XXI.5.1·XXI.5.2).** 장기 실행·native 플러그인은 앱 process에 적재하지 않고 별도 process로 띄웁니다.
    - IPC는 stdin·stdout의 줄 단위 JSON입니다(writer가 개행을 모두 escape하므로 한 메시지가 한 줄). 플러그인 → host 요청은
      7의 RPC envelope 그대로이고 같은 session(권한·상한)을 지납니다. host → 플러그인 수명 메시지는
      `{"stellar_process":1,"type":"heartbeat","seq":n}`이고 플러그인은 `{"stellar_process":1,"type":"heartbeat-ack","seq":n}`로
      답합니다.
    - heartbeat 응답이 제한 시간 안에 없거나 process가 끝나면 kill하고 session을 닫습니다. 권한 철회는 process를 끝냅니다.

## Alternatives

- **WIT(Component Model)로 Host API 정의.** WASM component 도구에 묶이고 TypeScript 생성기를 외부 도구로 들여야 합니다. Host API는
  WebView JS가 주로 부르므로 JSON IDL과 자체 생성기를 택했습니다.
- **mcp-core의 JSON·transaction 해석 재사용(의존).** runtime이 mcp-core에 도달하지 않는다는 규칙과 충돌해 기각했습니다.
- **package 안 내부 symlink 허용.** 대상 확인 시점과 사용 시점 사이 교체 위험이 있어 기각했습니다.
- **중복 id 중 먼저 찾은 package 채택.** 디렉터리 순서에 따라 결과가 바뀌어 기각했습니다.
- **플러그인 data를 `.stellar/` 아래에 두기.** `.stellar/cache`는 Git 제외 파생 데이터이므로 협업 data를 공유할 수 없어 기각했습니다.

## Consequences

- 플러그인 문서 접근이 CLI·MCP와 같은 ops 함수를 지나므로 결과·잠금·오류가 같습니다.
- JSON codec 사본이 셋이 됩니다(XXIX.1.2에서 하나로 합칩니다).
- WebView 연결(XXI.2), JS↔WASM SDK(XXI.3), 외부 process UI(XXI.5.3), 협업 확장 event(XXI.6.1)는 이 결정 위에 남은 일입니다.

## Compatibility

- 새 생성 schema 두 개와 IDL 하나를 더합니다. 기존 파일 포맷·공개 API·FFI는 바뀌지 않습니다.
- manifest 형식이나 Host API가 호환되지 않게 바뀌면 `manifest_version`·`host_api_version`을 올립니다.

## Performance

- 1초 20 메시지·batch 64 상한으로 Host RPC가 interaction T0\~T5 경로에 끼지 않습니다. 고빈도 event는 Host RPC를 지나지 않습니다.
- package 검증은 적재 때 한 번이며 항목 수·크기 상한이 시간을 묶습니다.

## Security

- 경로 탈출·symlink·중복 id·모르는 version을 적재 전에 거부합니다. release는 서명된 package만 받습니다.
- 모든 method는 실행 전에 유효 권한을 확인합니다. 오류 문구에 프로젝트·package 원문 값을 넣지 않습니다.
- 외부 process 플러그인은 앱 process 밖에서 돌고 응답 없음·종료 시 정리됩니다.

## Migration

- 새 기능이므로 기존 데이터 migration은 없습니다.
