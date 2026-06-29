# 작업 보고서: IV.1.6 stellar.lock.yaml 모델

- WBS: IV.1.6
- 작업명: 잠금 파일(`stellar.lock.yaml`) 데이터 모델 정의(매니페스트와 별도)
- 상태: 완료 (`[x]`)
- 선행: IV.1.1(`YamlValue`·매니페스트 모델)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/lock.rs` (신규: `Lockfile`, `ResolvedDependency`, `LockError`, `to_yaml`/`from_yaml`, 테스트)
- `crates/project/src/lib.rs` (`pub mod lock;`)
- `docs/task-reports/IV.1.6.md`(신규), 계획서(IV.1.6 `[ ]`→`[x]`)

## 구현 요약

- 매니페스트와 **별도** 모델로 `Lockfile { schema_version, resolved: Vec<ResolvedDependency> }`를
  정의했습니다.
- `ResolvedDependency`는 지시한 6개 필드를 모두 포함합니다: `logical_id`, `source`,
  `exact_commit`, `content_hash`, `schema_version`(해석된 라이브러리의 매니페스트 스키마 버전),
  `dependency_path`(루트→해당 의존성 logical ID 경로).
- `resolved`는 **시퀀스**라 같은 라이브러리가 서로 다른 경로/commit으로 여러 번 등장할 수
  있습니다(다중 버전). 매핑이 아니므로 logical_id 충돌로 덮어쓰지 않습니다.
- `to_yaml`/`from_yaml`은 키 순서를 고정하고, 알 수 없는 키·타입 불일치·누락 필드를 거부하며,
  schema_version은 `scalar::parse_u32`(IV.1.4 엄격 정수)로 해석합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-project` (49 unit) | 0 |
| `cargo fmt -p stellar-project -- --check` | 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| **다중 버전 동일 라이브러리 round-trip**(design-system 2회, 서로 다른 commit·경로 보존) | 통과 |
| writer→loader 텍스트 round-trip | 통과 |
| 빈 resolved round-trip | 통과 |
| 알 수 없는 키(top/entry)·누락 필드·잘못된 타입·invalid schema_version 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- `LockError`를 매니페스트 `ManifestError`와 별도로 정의하고, 트리 접근 헬퍼도 lock 모듈에
  국소적으로 두었습니다(에러 타입이 모듈별로 다름). 향후 공통 schema 오류로 통합할 여지가
  있으나 현재는 모듈 독립성을 우선했습니다.

## 남은 위험 / 한계

- lock writer가 사용자 편집 필드를 받지 않고 전부 resolver 결과에서 생성되게 강제하는 것은
  IV.1.7에서 추가합니다(현재 모델은 round-trip·검증만 제공).
- `content_hash`/`exact_commit`의 형식 검증(예: sha256 길이)은 하지 않습니다. 형식 계약은
  resolver/저장 계층에서 다룹니다.
