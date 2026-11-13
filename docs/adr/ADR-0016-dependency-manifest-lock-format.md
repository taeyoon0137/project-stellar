# ADR-0016: `stellar.yaml` 의존성 entry와 `stellar.lock.yaml` entry 형식 단일화

## Status

Accepted (2026-11-14, 구현 기록)

## Context

Phase IV(IV.1.1·IV.1.6)는 project crate에 `stellar.yaml`·`stellar.lock.yaml` 모델을 먼저
정의했습니다. 당시 의존성 entry는 문자열 네 개(`source`, `repository`, `version`, 매핑 키 이름),
lock resolved entry는 문자열 여섯 개(`logical_id`, `source`, `exact_commit`, `content_hash`,
`schema_version`, `dependency_path`)였고 두 모델 모두 알 수 없는 키를 거부합니다.

이후 Phase XV(XV.1.1·XV.4.1·XV.4.2·XV.6.1)는 `stellar-dependencies` crate에 typed 모델을
정의했습니다. 의존성 entry에는 `library_id`, version 선택자(tag 스칼라, `{branch: ...}`,
`{commit: ...}`), 선택 `local_override`가 필요하고, lock entry에는 `library_id`, `repository`,
`requested`, 전이 `dependencies`가 더 필요합니다(계획 XV.4.2 "exact commit, cached content hash,
library schema version, transitive dependencies").

통합 리뷰는 같은 두 파일에 형식이 두 개 생겼다고 지적했습니다.

- 실제 `stellar.yaml` 파서(`Manifest::from_yaml`)는 `library_id`·`local_override`·매핑 `version`을
  알 수 없는 키·타입 오류로 거부했고, XV.1.1·XV.6.1 round-trip은 형식 중립 트리에서만 증명됐습니다.
- `stellar.lock.yaml`에는 project의 6필드 모델과 dependencies의 10필드 모델이 따로 있었습니다.
  두 모델은 서로의 출력을 읽지 못합니다.

계획 §1.2 규칙 3은 지시 없는 파일 포맷 추가를 금지합니다. 그래서 두 번째 형식을 남기지 않고 하나로
합쳐야 합니다.

## Decision

1. **형식은 하나입니다.** 두 파일의 의존성 부분은 `stellar-dependencies`의 typed 모델이
   정의합니다. project crate는 제한 YAML(`YamlValue`)과 형식 중립 트리(`DeclNode`)를 1:1로
   바꾸는 일만 합니다(`crates/project/src/manifest_node.rs`). 이 변환은 매핑 키를 스칼라로 제한하고
   깊이를 제한 loader와 같은 64로 제한합니다.
2. **`stellar.yaml`.** `Manifest::dependencies`는 `Vec<DependencyDeclaration>`입니다(이전 이름
   `manifest::Dependency`는 그 타입의 별칭으로 남습니다). entry 키는 `source`, `library_id`,
   `repository`, `version`, 선택 `local_override`이고 canonical 기록 순서도 이 순서입니다. tag는
   스칼라(`version: v2.4.0`, 아키텍처 20.3 예시와 같은 형식), branch·commit은 키 하나짜리
   매핑입니다.
3. **`stellar.lock.yaml`.** IV.1.6 `Lockfile`의 resolved entry 타입은 dependencies의 `LockEntry`
   (`lock::ResolvedDependency` 별칭)이고, 검증은 `LockDocument::from_node`(XV.4.2 validator)가
   합니다. IV.1.6 필드 여섯 개는 이름과 의미를 그대로 유지합니다. canonical 순서는 `logical_id`,
   `library_id`, `source`, `repository`, `requested`, `exact_commit`, `content_hash`,
   `schema_version`, `dependency_path`, `dependencies`이고 열 개 모두 필수입니다. entry 순서는
   LibraryId → commit → dependency path여야 합니다.
4. **lock `schema_version`은 1로 유지합니다.** resolver가 non-empty lock을 기록한 적이 없기
   때문입니다. 지금까지 lock을 쓴 경로는 scaffold의 빈 잠금 파일(`resolved: []`) 하나뿐이고, 이
   파일은 새 형식에서도 그대로 유효합니다. 버전을 올리면 이미 만들어진 빈 잠금 파일까지 이전 판정이
   필요해지는데, 그 비용에 비해 얻는 것이 없습니다.
5. `Lockfile::write_checked`는 기록 전에 모델 규칙(지원 버전, canonical 순서)을 검사합니다. 위반하면
   `LockWriteError::Invalid`로 거부하고 텍스트를 만들지 않습니다. 기록된 lock은 항상 다시 읽힙니다.

## Alternatives

- **IV.1.6 6필드 모델을 유지하고 새 필드를 선택 키로 추가:** XV.4.2는 누락 필드를 실패시키는
  validator를 요구합니다. 선택 키로 두면 그 검증을 만족할 수 없어 탈락했습니다.
- **dependencies에 별도 텍스트 parser 추가:** 제한 YAML 규칙(IV.1.2~IV.1.5)을 두 번 구현하게 됩니다.
  crate 의존 방향(dependencies → core-types만)도 깨지므로 탈락했습니다.
- **lock `schema_version` 2로 올림:** 4번 결정의 이유로 탈락했습니다.

## Consequences

- `stellar.yaml` 의존성과 lock entry에 parser가 하나씩만 있습니다. XV.1.x·XV.4.x·XV.6.1 검증은
  실제 파일 텍스트 경로(제한 loader → 모델 → canonical writer)로 수행합니다.
- `library_id` 없는 IV.1.1 형식의 `stellar.yaml` 의존성 entry는 이제
  `MissingField(dependencies.<name>.library_id)`로 거부합니다. 저장소 안의 해당 fixture(stellar-cli
  `valid.stellar.yaml`, fuzz seed)는 새 형식으로 갱신했습니다.
- `ManifestError`·`LockError`에 `InvalidValue { path, error }`와 `TooDeep`이 추가됐습니다.
  `LockError::InvalidSchemaVersion`은 `InvalidValue { error: InvalidInteger | UnsupportedVersion }`로
  바뀌었습니다. lock 최상위 오류 경로는 `<root>.x`가 아니라 `x`입니다.
- 오류 메시지는 경로와 위반 규칙만 담고 URL·override 경로 원문을 복사하지 않습니다(XV.1.4).

## Compatibility

- 파일 포맷: `stellar.yaml` 의존성 entry 형식이 확장되고 `library_id`가 필수가 됐습니다. 이 변경은
  초기 릴리스 전입니다. `stellar.lock.yaml` entry는 10필드가 됐고 schema version 1은 유지합니다.
  빈 잠금 파일은 바뀌지 않습니다.
- 공개 API: `manifest::Dependency`와 `lock::ResolvedDependency`는 dependencies 타입의 별칭입니다.
  필드 접근 코드 3곳(stellar-cli `inspect` 출력)은 `as_str()`/`value()`로 바꿨고, JSON 출력은 그대로입니다.
  `Lockfile::write_checked`의 오류 타입은 `LockWriteError`입니다.
- FFI 영향은 없습니다.

## Performance

- 매니페스트·lock 파싱에 `YamlValue → DeclNode` 복사가 한 번 추가됩니다. 의존성 절 크기에
  선형이고, 제한 loader 입력 상한(1 MiB) 안에서만 일어납니다.

## Security

- 변환 깊이를 제한해 프로그램이 만든 깊은 트리로도 호출 stack을 소모할 수 없습니다.
- repository URL의 option 주입(`-` 시작)과 명령 실행 transport(`ext::`, `fd::`)는 실제 파서
  경로에서 거부합니다. 이 검사는 이전에는 중립 트리에서만 적용됐습니다.

## Migration

- 저장소 fixture는 단일화 Fix 커밋 두 개(`stellar.yaml` 연결, lock 형식 통합)에서 새 형식으로
  갱신했습니다.
- 사용자 프로젝트: 초기 릴리스 전이라 이전할 실제 데이터가 없습니다. 기존 `stellar.yaml`에
  의존성이 있으면 entry마다 `library_id`(라이브러리 프로젝트 `stellar.yaml`의 `project.id`)를
  추가합니다. 비어 있지 않은 이전 형식 lock은 resolver로 다시 생성합니다(lock은 직접 편집하지
  않습니다).
