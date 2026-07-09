# 작업 보고서: V.1.5 content hash 알고리즘 ADR + ContentHash newtype

- WBS: V.1.5
- 작업명: content hash 알고리즘 ADR 고정, `ContentHash` newtype·문자열 형식(알고리즘 이름 포함)
- 상태: 완료 (`[x]`)
- 선행: I.3.5, ADR 승인

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `docs/adr/ADR-0004-content-hash.md`(신규) — SHA-256 고정, `sha256:<hex64>` 문자열 형식.
- `crates/storage/src/hash/mod.rs`(신규) — `HashAlgorithm`, `ContentHash`,
  `ParseContentHashError`.
- `crates/storage/src/hash/sha256.rs`(신규) — 내부 SHA-256 구현(FIPS 180-4).
- `crates/storage/src/lib.rs` — `pub mod hash;` 등록.
- `docs/task-reports/V.1.5.md`(신규), 계획서(V.1.5 `[ ]`→`[x]`).

## 구현 요약

- **ADR-0004**: content hash 알고리즘을 SHA-256으로, 문자열 형식을
  `sha256:<64자리 소문자 16진수>`(71자 고정)로 고정했습니다. 알고리즘 이름 prefix가
  데이터에 남아 미래 교체(새 ADR + migration)를 데이터 차원에서 허용합니다. BLAKE3는
  외부 의존성 정책(I.3.2, 현재 승인 의존성 `yaml-rust2` 하나) 때문에 미선택 —
  성능 병목 확인 시(V.6.3) 새 ADR로 재검토합니다.
- **`ContentHash`**: 알고리즘 tag + 32바이트 digest의 newtype. `Display`/`FromStr`
  round-trip, `Ord`/`Hash`(map key 사용 가능). 파서는 알 수 없는 알고리즘·길이 불일치·
  대문자/비16진수 문자를 각각 구분된 오류로 거부합니다.
- **SHA-256 내부 구현**: 외부 의존성 없는 FIPS 180-4 구현. NIST 공식 vector(빈 입력,
  "abc", 448bit, 896bit, 1백만 'a')와 패딩 경계(55/56/63/64바이트, python hashlib로
  검산한 값)로 고정했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 35 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |
| `python3 hashlib` 경계 vector 검산(55/56/63/64바이트) | 4개 모두 일치 |

| 검사 | 결과 |
|---|---|
| hash test vector(NIST 5종 + 경계 4종) | 통과 |
| 문자열 parse round-trip(`string_round_trip`) | 통과 |
| 알 수 없는 알고리즘·잘못된 길이·대문자 hex 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. ADR 승인은 저장소 유지자가 명시한 자율 진행 지시에 따라 Accepted로 기록했습니다.
  이견이 있으면 ADR-0004의 Status를 조정하고 새 ADR로 교체할 수 있습니다.

## 남은 위험 / 한계

- 내부 SHA-256은 하드웨어 가속 구현 대비 느립니다. chunk 크기 측정(V.6.3)에서 hashing이
  저장 예산을 위협하면 ADR-0004의 Migration 절차로 교체합니다.
- `ContentHash`는 32바이트 digest 고정입니다. digest 길이가 다른 알고리즘 도입 시
  구조 변경이 필요합니다(현재는 SHA-256 단일 registry).
