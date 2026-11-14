# ADR-0059: 의미 단위 텍스트 파일을 Git 원본으로 쓰고 chunk 저장소는 로컬 캐시로 전환

## Status

Accepted (2026-11-14). 제품 책임자가 결정했습니다("바꾸는 게 나을 것 같은데. 성능적인 부분은 청크를 캐시처럼 활용하는 방안도 있을 것 같기도 하고").

- 기술 아키텍처 §19·§21의 "`ChunkId + ContentHash` 혼합 chunk를 Git에 커밋하는 원본으로 쓴다"는 결정을 대체합니다.
- 첫 단일 View 텍스트 문법과 전환 WBS V.10.1~V.10.7를 아래에 정합니다. 전체 파일 분할·journal·cache 최적화는 후속 행에서 검증합니다.

## Context

- Stellar의 원본은 폴더와 실제 Git 저장소입니다(아키텍처 §3.3). 제품 방향은 다음과 같습니다.
  - AI 에이전트가 `stellar` CLI와 MCP로 프로젝트를 편집합니다(ADR-0005).
  - 디자인 속성은 CSS 어휘를 씁니다(ADR-0024).
  - 실제 코드로 변환합니다(ADR-0025).
- 현재 저장 방식(ADR-0010·0011·0012)은 바이너리 chunk를 내용 hash 이름으로 `design/objects/`에 쓰고, 세대 manifest와 `design/current` 포인터를 Git에 커밋합니다. 이 방식은 Git 협업에서 다음 문제가 있습니다.
  - 저장할 때마다 `design/current` 한 줄이 바뀝니다. 그래서 각자 저장한 두 브랜치를 병합하면 이 파일이 항상 충돌하고, Stellar 없이 풀 수 없습니다.
  - chunk가 바이너리라 `git diff`와 PR에서 무엇이 바뀌었는지 읽을 수 없습니다.
  - 파일 이름이 내용 hash라서 수정이 "삭제 + 새 파일"로 보입니다. 저장할 때마다 manifest 파일도 늘어납니다.
  - 사람·AI 에이전트·외부 도구가 파일을 직접 읽고 고칠 수 없습니다.
- 공개 포맷 동결(Phase XXVIII) 전이므로 지금이 포맷을 바꾸는 비용이 가장 낮은 시점입니다.

## Decision

1. **Git 원본은 의미 단위 텍스트 파일입니다.** 컴포넌트 정의, 페이지, View, 토큰 세트, 프로젝트 메타데이터를 각각 파일 하나로 둡니다.
   - 파일 이름은 사람이 읽을 수 있는 이름에 짧은 고정 ID(`StableId` 파생)를 붙입니다. 이름이 바뀌면 파일 이름을 바꾸고 ID로 같은 대상임을 판정합니다.
   - 큰 View는 Frame 의미 경계로 여러 파일로 나눕니다(ADR-0012의 256 KiB 기준을 출발점으로 다시 측정합니다).
2. **텍스트는 결정론적입니다.** 같은 문서는 항상 같은 bytes입니다.
   - 정렬 순서를 고정하고, 속성 하나를 한 줄에 씁니다.
   - float은 정확히 왕복되는 최단 표현을 씁니다.
   - 서로 다른 대상을 고친 두 브랜치는 Git의 줄 단위 병합으로 대부분 해결되고, 구조가 얽힌 충돌만 Semantic Merge(merge driver 포함)가 맡습니다.
3. **chunk 엔진은 Git에서 제외한 로컬 캐시가 됩니다.** 위치는 `.stellar/cache/`입니다.
   - 기존 `ChunkId + ContentHash` 저장, 지연 로딩, 무결성 검증을 재사용해 빠른 열기와 부분 로딩을 맡습니다.
   - 캐시는 언제든 지우고 텍스트에서 다시 만들 수 있어야 합니다. 텍스트 원본의 hash로 무효화합니다.
4. **여러 파일 저장의 원자성은 저장 journal로 보장합니다.** 바뀐 파일을 모두 임시 파일로 쓴 뒤 rename합니다. 중간에 종료되면 다음 열기에서 journal로 앞 세대 또는 새 세대 한쪽으로 맞춥니다.
5. **외부 변경을 반영합니다.** 에디터, AI 에이전트, `git pull`처럼 앱 밖에서 텍스트를 바꾸면 기존 파일 감시(XIII.2)와 외부 변경 반영(XIII.3) 경로로 다시 읽습니다.
6. **위층은 그대로 둡니다.** 문서 모델(record, `StableId`), validator, Semantic Diff·Merge, history, command, 복구 스냅샷은 바꾸지 않습니다. 파일 층(`crates/storage`의 세대·manifest 원본 역할과 `crates/project`의 `document_io`)만 바꿉니다.

첫 실행 단위(V.10.1~V.10.3):

- [text-view-source-v1](../spec/text-view-source-v1.md): 기존 제한 YAML 파서·writer를 재사용하며 `.stellar.yaml` 파일 하나에 단일 문서·페이지·View와 Frame/Fragment를 기록합니다. V.10.4의 첫 부분은 Text 본문·문단 style index·style run의 순서와 범위까지 보존하며 지원 범위를 확장합니다. 모든 Frame 속성은 사람이 읽는 값으로 기록하고, 지원하지 않는 node·modifier·필드·schema는 오류로 거부합니다.
- `stellar text-source export <프로젝트> <새 파일>`은 실제 기존 generation을 읽고 재직렬화 chunk pointer 집합 전체가 같을 때만 별도 파일을 만듭니다. 모르는 optional chunk도 묵시적으로 버리지 않습니다. `check <파일>`은 실제 DocumentModel을 재구축해 읽기 전용 검증합니다.
- V.10.6의 첫 부분으로 `text-source apply <파일> <입력|-> [--dry-run]`을 연결합니다. 실제 원본 hash·문서 ID 확인, 필수 저장 잠금, 기존 command transaction·semantic diff, 미지원 결과의 dry-run 거부, 단일 파일 조건부 저장을 사용합니다. 전체 파일 감시·dirty 재로드·병합·여러 파일 journal의 완료를 뜻하지 않습니다.
- V.10.4의 컴포넌트 원본 첫 범위는 독립 파일 하나에 빈 인터페이스와 root Frame을 기록합니다. 이름과 StableId 유래 고정 접미사로 파일을 찾고, 전체 StableId로 정의와 root를 조립합니다. 기존 generation의 전체 라이브러리를 변환하지 않습니다. [컴포넌트 원본 v1](../spec/component-source-v1.md)
- 기본 앱·CLI 저장 경로는 그대로 둡니다. 이 파일은 전체 프로젝트 변환 완료의 표식이 아닙니다. 다중 페이지·View/전체 의미는 V.10.4, journal은 V.10.5, 외부 변경/명령 연결은 V.10.6, 기본 전환과 성능 측정은 V.10.7입니다.
- 단일 파일은 기존 atomic writer와 handle 검사를 재사용합니다(`crates/storage/source_file`). 새 파일의 기존 대상은 거부하고, 수정 저장은 열린 bytes hash를 다시 검사합니다. 원본 읽기의 사용자 symlink 허용은 §19.3을 유지하고, 원자 쓰기와 cache 경로는 symlink를 따라 교체하지 않습니다.
- 컴포넌트 원본을 프로젝트에 포함할 때 쓰는 opt-in 위치는 `design/components/`입니다. 프로젝트 CLI·headless MCP 조회는 이 내부의 원본 파일을 다시 읽고, 기존 component-library chunk와의 이중 원본은 거부합니다. 사용자가 명시한 단독 파일의 링크 읽기 정책은 유지하며, 프로젝트 단위 조회는 내부로 해석된 실제 경로의 제한된 bytes만 읽습니다. [원본과 조회 계약](../spec/component-source-v1.md)
- 선택적 `open_with_cache`는 원본을 검증한 뒤 기존 ChunkId·envelope로 `.stellar/cache/text-source-v1/<source-hash>/`를 재생성합니다. 누락·손상은 재생성, cache I/O 실패는 원본 열기 성공과 별도 상태로 반환합니다. 매번 원본을 해석하므로 빠른 열기 성능 완료로 보지 않습니다.

정하지 않은 사항(후속 결정):

- 다중 의미 단위의 디렉터리 배치와 파일 간 참조, 전체 텍스트 스키마
- 큰 View 분할 기준과 측정
- 저장 journal 형식
- 캐시 키와 GC
- V.10.7의 기본 전환 시점 및 기존 generation 원본 역할 폐기 범위

## Alternatives

| 대안 | 판단 |
|---|---|
| 현재 chunk를 Git 원본으로 유지 | `design/current` 충돌, 읽을 수 없는 diff, AI·외부 도구 비친화 문제가 남습니다. 미선택 |
| chunk 유지에 current·manifest만 Git에서 제외 | 세대 원자성과 GC 근거가 흔들리고, 바이너리 diff 문제가 그대로 남습니다. 미선택 |
| 노드 하나에 파일 하나 | 수십만 파일로 Git·파일 시스템 비용이 큽니다(아키텍처 §21.4). 미선택 |
| 텍스트 원본만 쓰고 캐시 없음 | 큰 문서를 열 때마다 전체를 해석해야 해 성능 예산을 넘을 위험이 있습니다. 캐시를 두는 이 결정으로 보완 |

## Consequences

- 장점
  - PR 리뷰, Stellar 없는 일반 Git 병합, AI 에이전트와 CLI 편집, 코드 변환, 장기 공개 명세에 맞습니다.
  - 브랜치마다 반드시 생기던 `design/current` 충돌이 없어집니다.
- 단점
  - 이미 완료한 Phase V 행 가운데 "Git 원본으로서의 chunk·세대" 역할을 전제한 행은 의미가 바뀝니다. 전환 WBS에서 행별로 재분류합니다.
  - 여러 파일 원자성, 텍스트 결정론, 캐시 일관성이라는 새 검증 항목이 생깁니다.
  - 진행 중이던 chunk 원본 전제 작업은 통합할 때 다시 판단합니다(컴포넌트 라이브러리 chunk 저장, 제품 serializer의 chunk 분할 연결).

## Compatibility

- 프로젝트 파일 포맷이 바뀝니다. 공개 포맷 동결 전이라 외부 호환 약속은 없습니다.
- 공개 API와 FFI에는 열기·저장 경로의 내부 구현 변화만 있습니다. ADR 보호 경로(`crates/storage`)를 바꿀 때는 이 ADR을 근거로 같은 diff에 기록합니다.

## Performance

- 열기 성능은 캐시가 맡습니다. 캐시가 없는 첫 열기에서는 텍스트를 해석하는 비용이 듭니다.
- 저장은 바뀐 의미 단위 파일만 다시 씁니다.
- 열기·저장 예산(`config/performance-budgets.yaml`)은 전환 구현에서 실측해 확인합니다.

## Security

- 텍스트 파서는 신뢰할 수 없는 저장소 입력을 받습니다.
  - 크기·깊이·개수 상한을 둡니다.
  - 열린 handle 기준으로 symlink와 파일 종류를 검사합니다(보안 감사 STOR-1·STOR-4 교훈).
  - 오류는 panic 없이 반환합니다.
- 캐시는 Git 밖에 있고 언제든 다시 만들 수 있으므로, 캐시 손상은 다시 만드는 것으로 복구합니다.

## Migration

- 현재는 기존 loader로 읽은 단일 View 지원 문서를 새 텍스트 파일로 내보내고, 그 파일을 명시적 `text-source apply`로 편집할 수 있습니다. 기존 `design/current`·generation·objects·`stellar.yaml`·복구 snapshot은 수정하거나 삭제하지 않습니다. 생성 파일을 `check`로 재열고 semantic 레코드를 비교한 뒤 사람이 Git diff를 검토합니다. 미지원 입력은 원본·기존 출력 모두 보존합니다.
- 전체 프로젝트 변환은 V.10.4~V.10.7에서 semantic snapshot 동일성, journal 복구, 외부 변경 반영을 검증한 뒤 활성화합니다. 모든 기존 프로젝트의 일괄 전환과 기본 저장 전환은 그 전까지 하지 않습니다.
- 공개 배포 전이라 사용자 데이터 이전 부담은 개발용 fixture와 샘플 프로젝트에 한정됩니다.
