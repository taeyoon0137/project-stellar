# ADR-0135: 프로젝트 revision hash의 이름을 바로잡고, 사용자 세트 내용을 revision에 넣으며, 창 revision을 headless와 맞춘다

## Status

Accepted (2026-11-24).

- ADR-0128 결정 2와 그 Consequences의 한계 세 가지(필드 이름, 사용자 세트 파일, 창 revision)의 후속입니다. ADR-0128
  본문은 고치지 않고 이 문서가 보완합니다.
- 계획서 행 IX.11.29(`set`이 ops 쓰기 경로의 base revision 검사를 거침)와 XXIV.4.2(revision을 포함한 snapshot)의
  뜻을 지키는 정리입니다. 새 행은 없습니다.
- ADR-0005 §1(파일이 원본), ADR-0082 결정 4(세트 선택과 사용자 세트 파일), ADR-0113 결정 1·2(열기 계획, MCP snapshot
  재사용)를 따릅니다.

## Context

- `ProjectRevision::Saved`의 hash 필드 이름이 `manifest_hash`였고, CLI·MCP JSON revision object의 키도
  `manifest_hash`였습니다. 실제 값은 세 가지입니다.
  - cache에서 읽은 기본 세트 프로젝트: current manifest content hash.
  - 원본에서 읽은 상태(FromText): 원본 revision hash(`session::source_revision`).
  - 기본 세트가 아닌 프로젝트: 위 hash에 세트 선택을 더한 hash(ADR-0128 결정 2).
  - 그래서 AI 클라이언트나 사람이 이 값을 manifest hash로 읽고 `design/generations/`와 맞춰 보면 틀립니다.
- ADR-0128은 `stellar.yaml`의 세트 이름만 revision에 넣었습니다. 사용자 세트 파일(`design/style-defaults/*`) 편집은
  생략값 해석을 바꾸지만 generation도, 텍스트 원본 digest(세트 파일은 원본 집합에서 뺌)도 바꾸지 않습니다. writer A가
  revision R을 읽은 뒤 편집기·Git pull이 세트 파일 값을 바꿔도 A의 `--base R`은 통과했습니다.
- MCP snapshot 재사용 판정(`ProjectSnapshot::disk_unchanged`)도 세트 파일을 보지 않아, 세트 파일이 바뀐 뒤에도 옛
  snapshot과 옛 revision을 돌려줄 수 있었습니다.
- runtime `agent_edit::project_revision`(에이전트 proposal의 base 대조)은 창이 연 cache generation 번호와 manifest
  hash만 썼습니다.
  - 세트가 `css`가 아닌 프로젝트에서 headless revision과 표기가 달랐습니다(ADR-0128 Consequences).
  - 창이 Git pull 같은 외부 원본 변경을 반영하고 cache 맞춤 쓰기를 다음 저장에 남긴 동안, headless는 원본에서 읽은
    text revision을, 창은 옛 cache revision을 냈습니다.
  - 그래서 MCP로 읽은 revision으로 창에 proposal을 보내면 같은 디스크 상태인데도 base 불일치가 났습니다.
- ADR-0128이 더한 `g0:<hash>` 표기(저장 전 프로젝트 + 다른 세트)를 MCP resource schema와 CommandTransaction schema의
  token pattern이 받지 않았습니다. 해석기(`FromStr`)는 받습니다.

## Decision

1. **hash 이름을 `hash`로 바꿉니다.**
   - `ProjectRevision::Saved { generation, hash }`, `ProjectRevision::hash()`입니다. 뜻은 "revision hash"이고 manifest
     content hash와 같은 것은 cache에서 읽은 기본 세트 상태뿐입니다(모듈 문서).
   - CLI·MCP JSON revision object는 `{"token", "generation", "hash"}`입니다. 값은 그대로이고 키 이름만 바뀝니다.
     저장 전 기본 세트 프로젝트는 `null`입니다.
   - manifest hash가 필요한 비교는 세트를 더하지 않은 `ProjectSnapshot::storage_revision`이나
     `ProjectSnapshot::current_manifest`를 씁니다(`compare`의 HEAD 재사용 판정은 ADR-0128 그대로).
   - `LoadedGeneration::manifest_hash`, recovery의 `base_manifest_hash`처럼 실제 manifest hash를 담는 storage·project
     필드는 그대로 둡니다.
2. **사용자 세트를 고르면 그 세트를 평탄화한 값을 revision에 넣습니다.**
   - `revision::StyleDefaultsKey`가 revision에 더할 세트 상태입니다.
     - 내장 세트(`css`, `react-native`)는 이름뿐입니다. 결과 hash는 ADR-0128 표기와 같습니다.
     - 사용자 세트는 이름과, `flatten_named`로 `extends` 사슬까지 푼 값의 content hash입니다. 입력은 영역 구분자
       (`stellar.project-revision.user-set.v1`), 사슬 이름, 그리고 모든 구획 × longhand의 `format_value` 표기입니다.
     - 세트 파일을 읽거나 풀지 못하면(없는 세트, 순환, 문법 오류) 그 오류 code와 문장의 hash입니다.
   - `with_style_defaults(&StyleDefaultsKey)`는 ADR-0128의 입력(영역 구분자 ‖ 이전 hash ‖ 줄바꿈 ‖ 이름) 뒤에, 사용자
     세트일 때만 줄바꿈과 내용 hash 표기를 더합니다. 세트 이름에는 줄바꿈이 없어 두 입력이 섞이지 않습니다.
   - 값의 뜻이 같은 편집(줄 순서, 주석)과 사슬 밖 세트 파일 편집은 revision을 바꾸지 않습니다. 사슬 안 세트 파일의 값,
     `extends`를 바꾸면 revision이 바뀝니다.
   - `ProjectSnapshot`은 열 때 사용자 세트를 한 번 읽고(`load_user_sets`) 활성 세트를 평탄화합니다(`active_set`과 같은
     두 단계). `ProjectSnapshot::style_defaults_key(name)`이 같은 사용자 세트로 다른 이름의 상태도 만듭니다. 같은
     쓰기에서 세트를 바꾸는 `ops::apply`, `snapshot::save`, `ops::migrate`가 결과 revision에 이 값을 더합니다.
   - `disk_unchanged`는 사용자 세트를 골랐으면 세트 파일을 다시 풀어 같은지 봅니다. MCP는 세트 파일이 바뀐 뒤 옛
     snapshot을 다시 쓰지 않습니다.
3. **창 revision을 같은 디스크 상태의 headless revision과 같게 만듭니다.**
   - `agent_edit::project_revision`은 두 부분을 headless와 같은 함수로 만듭니다.
     - 원본·cache 부분: 창이 연 cache generation입니다. 창이 cache 맞춤 쓰기를 남겼고(`reconcile_pending`) 마지막 읽기
       계획이 Forward·Migrate가 아니면 원본이 cache보다 앞선 상태라 headless가 FromText로 엽니다. 이때는
       `snapshot::text_revision`(이제 공개)으로 같은 text revision을 만듭니다.
     - 세트 부분: 창이 열 때·마지막 저장 때 맞춘 선택(`ProjectWindowSession::saved_style_defaults`, 모델 거울의 저장 전
       선택이 아님)에 `StyleDefaultsKey::load`를 더합니다. 창은 사용자 세트 내용을 들고 있지 않으므로 사용자 세트를
       골랐을 때만 부를 때 세트 파일을 읽습니다.
   - runtime test가 기본 세트, 내장 세트, 사용자 세트(파일 편집 전후), 원본이 cache보다 앞선 상태와 맞춤 쓰기 뒤에서
     창 revision이 `open_snapshot` revision과 같음을 봅니다.
4. **revision token schema가 `g0:<hash>`를 받습니다.**
   - MCP resource의 revision object와 CommandTransaction `base_revision`의 pattern은
     `^g(0|(0|[1-9][0-9]*):sha256:[0-9a-f]{64})$`입니다. 해석기와 같은 문법입니다. `schemas/command-transaction.schema.json`은
     `cargo xtask generate-command-schema`로 다시 만듭니다.

## Alternatives

| 대안 | 판단 |
|---|---|
| Rust 필드만 바꾸고 JSON 키 `manifest_hash`는 호환을 위해 둠 | 채택하지 않음. AI 클라이언트가 이름을 보고 manifest hash로 씁니다. 공개 포맷(Phase XXVIII) 전이라 지금 바꾸는 비용이 가장 작습니다 |
| JSON에 `hash`를 더하고 `manifest_hash`도 남김 | 채택하지 않음. 같은 값을 두 이름으로 내는 이중 경로입니다 |
| JSON `manifest_hash`를 진짜 manifest hash로 바꿈 | 채택하지 않음. revision object는 token에서도 만들어지며(base revision 결과) token에는 manifest hash가 없습니다 |
| 사용자 세트 파일 bytes를 그대로 hash | 채택하지 않음. 사슬 밖 파일·주석 편집도 revision을 바꾸고, 뜻이 같은 상태가 다른 revision이 됩니다(ADR-0128의 "내용이 같으면 같은 revision") |
| 세트 파일을 텍스트 원본 digest에 다시 넣음 | 채택하지 않음. 원본 집합과 stamp 뜻이 바뀌어 cache 판정(ADR-0113)이 세트 편집마다 FromText가 됩니다 |
| 세트 파일 편집을 revision에 넣지 않고 문서화만 함 | 채택하지 않음. 생략값 해석이 바뀐 상태 위에서 낡은 base 쓰기가 통과합니다 |
| 창이 열 때 세트 내용 hash를 기억 | 채택하지 않음. 창 session(소유 밖)에 세트 파일 관찰을 더해야 하고, 외부 세트 편집을 창이 반영하는 경로가 없어 기억한 값이 디스크와 어긋납니다 |
| 창 revision을 언제나 디스크에서 다시 계산(open_snapshot) | 채택하지 않음. 창이 기준으로 삼은 상태가 아니라 지금 디스크를 말하게 되고, document thread에서 원본 전체를 읽습니다 |

## Consequences

- revision hash의 뜻이 이름과 맞습니다. manifest hash 비교는 따로 있는 값으로 합니다.
- 사용자 세트 파일 편집을 base revision 비교와 MCP snapshot 재사용 판정이 봅니다. 세트 편집 뒤 낡은 revision으로 한
  쓰기는 `base-revision-mismatch`입니다.
- 같은 디스크 상태에서 창과 CLI·MCP의 revision이 같아, MCP로 읽은 revision을 창 proposal의 base로 쓸 수 있습니다.
- 저장 전 프로젝트에 세트를 고른 `g0:<hash>`가 schema 검사를 통과합니다.
- 한계가 있습니다.
  - 창이 저장 중 원본은 썼지만 cache stamp를 쓰지 못한 상태(`SaveOutcome::cache_stale`)는 창 메모리에 표시가 없어 창은
    cache revision, headless는 text revision입니다. 다음 저장이나 다시 열기가 맞춥니다.
  - 창이 사용자 세트를 고른 동안 세트 파일을 외부에서 바꾸면 창 revision이 바로 바뀝니다. 창은 세트 내용을 반영하지
    않으므로(활성 세트 거울 연결 작업의 범위) 그 사이 창의 생략값 해석과 revision이 가리키는 세트 내용이 다를 수
    있습니다. 낡은 base 쓰기를 막는 쪽으로 어긋납니다.
  - 내장 세트 표는 엔진 version이 정합니다. 엔진을 올려 표가 바뀌어도 revision은 그대로입니다.
  - CLI `views::revision`과 MCP `headless::revision`은 같은 object를 따로 만듭니다(crate 의존 방향). 출력 fixture가
    두 쪽을 함께 지킵니다.

## Compatibility

- 파일 포맷 변경은 없습니다(`stellar.yaml`, 세트 파일, chunk, 원본, command codec).
- 공개 API
  - `ProjectRevision::Saved`의 필드 `manifest_hash`가 `hash`로, 메서드 `manifest_hash()`가 `hash()`로 바뀝니다.
  - `ProjectRevision::with_style_defaults`가 `&str` 대신 `&StyleDefaultsKey`를 받습니다.
  - 새 API: `revision::StyleDefaultsKey`(`resolve`, `load`, `name`, `content`), `ProjectSnapshot::style_defaults_key`,
    공개 `snapshot::text_revision`, `ProjectWindowSession::saved_style_defaults`.
- CLI·MCP JSON: revision object의 `manifest_hash` 키가 `hash`가 됩니다(값 같음). MCP resource schema와 CLI 결과
  fixture를 함께 갱신합니다.
- revision 표기: 기본 세트와 내장 세트 프로젝트는 그대로입니다. 사용자 세트를 고른 프로젝트는 hash가 달라집니다.
  저장해 둔 base revision은 다시 조회해 얻습니다.
- schema: revision token pattern이 `g0:<hash>`를 더 받습니다. 받던 표기는 모두 그대로 받습니다.
- FFI 변경은 없습니다.

## Performance

- headless 열기는 이미 하던 사용자 세트 읽기·평탄화를 그대로 한 번 하고, 사용자 세트일 때 짧은 텍스트(구획 4 ×
  longhand 수 줄) hash 한 번을 더합니다.
- MCP 재사용 판정은 사용자 세트를 골랐을 때만 세트 디렉터리(상한 256 파일, 파일별 loader 상한)를 다시 읽습니다.
- 창 `project_revision`은 내장 세트면 I/O가 없습니다. 사용자 세트면 부를 때마다 세트 디렉터리를 읽습니다. 부르는 곳은
  에이전트 proposal base 대조(`agent_io`)이고, 편집 hot path가 아닙니다.

## Security

- 세트 파일 읽기는 기존 경로 guard와 크기·개수 상한(`load_user_sets`)을 그대로 씁니다.
- revision hash는 무결성 표지가 아니라 변경 감지용입니다. 오류 상태 hash에 들어가는 문장은 프로젝트 root 기준 경로와
  진단뿐이고 결과 JSON에는 hash만 나갑니다.

## Migration

- 데이터 migration은 없습니다.
- JSON revision object의 `manifest_hash`를 읽던 자동화는 `hash`를 읽습니다. 사용자 세트를 고른 프로젝트의 저장된
  base revision은 다시 조회합니다.
