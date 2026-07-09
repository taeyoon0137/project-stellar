# ADR-0004: 저장 content hash 알고리즘

## Status

Accepted

## Context

Phase V 결정론적 청크 저장은 chunk payload와 generation manifest를 content hash로
주소화합니다(V.2.3 `design/objects/<prefix>/<hash>.starchunk`, V.4.2 manifest hash).
hash는 파일 경로·manifest·lockfile(IV.1.6)에 문자열로 기록되므로 알고리즘과 문자열
형식을 구현 전에 고정해야 합니다(V.1.5, 보호 경로 `crates/storage/` — I.3.5).

요구 사항:

- 결정론: 같은 canonical bytes(V.1.2)는 어느 플랫폼에서든 같은 hash 문자열을 만듭니다.
- 자기 기술: 문자열에 알고리즘 이름을 포함해 미래 알고리즘 교체를 데이터 차원에서
  허용합니다.
- 충돌 저항: 서로 다른 chunk가 같은 주소를 갖지 않아야 합니다(V.2.4는 동일 hash 파일의
  내용 검증도 수행하지만, 1차 방어는 알고리즘의 충돌 저항입니다).
- 의존성 정책: 직접 의존성은 `config/dependencies.yaml` 승인 목록으로 제한됩니다(I.3.2).

## Decision

- content hash 알고리즘은 **SHA-256**(FIPS 180-4)으로 고정합니다.
- 문자열 형식은 **`sha256:<64자리 소문자 16진수>`**로 고정합니다. 알고리즘 이름 prefix는
  필수이며, 대문자 16진수·다른 알고리즘 이름·잘못된 길이는 파싱에서 거부합니다.
- `crates/storage`에 `ContentHash` newtype을 두고, 알고리즘 tag + 32바이트 digest를
  보관합니다. 원시 `String`/`[u8; 32]`와 타입이 섞이지 않게 합니다.
- SHA-256 구현은 외부 의존성 없이 `crates/storage` 내부 구현으로 두고, FIPS 180-4
  공식 test vector로 고정합니다.

## Alternatives

- **BLAKE3**: 훨씬 빠르지만 외부 crate(`blake3`) 도입이 필요합니다. 현재 승인 의존성은
  `yaml-rust2` 하나이며, 저장 포맷의 1차 목표는 성능이 아니라 결정론·검증 가능성입니다.
  문자열에 알고리즘 이름이 포함되므로 성능이 병목이 되면 새 ADR로 `blake3:` prefix를
  추가 도입할 수 있습니다 — 현 시점 미선택.
- **SHA-1**: Git object id와 일치한다는 장점이 있으나 충돌 공격이 실증되어 신규 포맷에
  부적합 — 미선택.
- **알고리즘 이름 없는 raw hex**: 문자열이 짧아지지만 미래 알고리즘 교체 시 데이터만
  보고 구분할 수 없음 — 미선택.

## Consequences

- hash 문자열은 71자(`sha256:` 7자 + hex 64자)로 고정되어 경로 생성(V.2.3)이 고정 길이
  안전 경로를 보장할 수 있습니다.
- 내부 SHA-256 구현은 유지보수 부담이 있으나 test vector로 고정되고, 승인 의존성 0을
  유지합니다. 성능 요구가 확인되면(V.6.3 측정) 새 ADR로 교체합니다.

## Compatibility

- 파일 포맷: `.starchunk` 경로와 manifest의 hash 문자열 형식이 본 ADR로 고정됩니다.
- 알고리즘 교체는 새 prefix 도입 + migration(Phase V.8)으로만 가능합니다. 기존
  `sha256:` 값은 재해석되지 않습니다.

## Performance

- 순수 Rust SHA-256은 하드웨어 가속 구현보다 느립니다. chunk 크기 측정(V.6.3)에서
  hashing이 저장 예산을 위협하면 교체를 검토합니다. 현 phase의 완료 조건에는 영향 없음.

## Security

- SHA-256은 현재 실증된 충돌 공격이 없습니다. hash는 무결성 검증용이며 비밀값이
  아니므로 키 관리 영향 없음.

## Migration

1. 새 알고리즘 도입 시 새 ADR로 prefix(예: `blake3:`)를 등록합니다.
2. 파서는 알 수 없는 prefix를 명확한 오류로 거부하므로, 구버전 reader는 새 데이터를
   조용히 오독하지 않습니다.
3. 기존 세대의 재해싱은 migration(Phase V.8) 단계로만 수행합니다.
