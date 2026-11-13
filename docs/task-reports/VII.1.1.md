# 작업 보고서: VII.1.1 CommandId, WindowSessionId, CommandOrigin

- WBS: VII.1.1
- 작업명: `CommandId`, `WindowSessionId`, `CommandOrigin`(User·System·Migration·FutureAgent) 정의
- 상태: 완료 (`[x]`)
- 선행: Phase VI(phase 단위 선행 — 완화 적용, 아래 "차이" 참조)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/Cargo.toml` — `stellar-core-types` path 의존성 추가(2e06c27이 허용한 `commands -> core-types` edge).
- `crates/commands/src/lib.rs` — crate 문서, `pub mod id;`.
- `crates/commands/src/id.rs`(신규) — `CommandId`, `AgentProvenance`, `CommandOrigin`, `OriginDecodeError`,
  `WindowSessionId` 재수출, compile-fail doctest, unit test.
- `Cargo.lock` — 위 의존성 추가로 cargo가 자동 갱신(`stellar-commands`의 dependency 목록에 `stellar-core-types` 1줄).
- `docs/task-reports/VII.1.1.md`(신규), 계획서(VII.1.1 `[ ]`→`[x]`).

## 구현 요약

- **`CommandId`**: `StableId` 기반 newtype. 16바이트·표준 UUID 문자열 round-trip(`to_bytes`/`from_bytes`,
  `Display`/`FromStr`). node id·창 세션 id와 섞이지 않습니다.
- **`WindowSessionId`**: core-types(V.9.1)에 이미 정의된 타입을 그대로 재수출합니다(중복 정의 없음).
- **`CommandOrigin`**: `User`, `System`, `Migration`, `FutureAgent(AgentProvenance)`. canonical binary는 tag
  `u8`(0..=3, golden 고정) + FutureAgent만 16바이트 edit session id. `decode`는 알 수 없는 tag·입력 부족·
  trailing bytes를 거부하고, `decode_prefix`는 소비 길이를 돌려줘 상위 codec이 이어 읽을 수 있습니다.
- **FutureAgent**: 동작(lease·승인·rollback)은 구현하지 않고 출처 표식과 provenance(agent edit 세션 id)만
  둡니다. history 항목이 이를 보존해 나중에 사용자가 특정 agent 세션 변경만 되돌릴 기준이 됩니다(§31.4·§31.5).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-commands` | 완료 |
| `cargo test -p stellar-commands` | unit 4 passed, doctest(compile_fail) 3 passed, 0 failed |
| `cargo clippy -p stellar-commands --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-commands -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| CommandId bytes·문자열 round-trip, 잘못된 문자열 거부(`command_id_binary_and_string_round_trip`) | 통과 |
| CommandOrigin 4 variant round-trip, tag golden `[0,1,2,3]`, provenance 조회(`origin_round_trips_every_variant`) | 통과 |
| 빈 입력·unknown tag·session id 부족·trailing bytes 거부, prefix 소비 길이(`origin_decode_rejects_malformed_input`) | 통과 |
| `CommandId == WindowSessionId` 비교, `CommandId`→`WindowSessionId` 대입, `StableId`→`CommandId` 대입 compile-fail | 통과(doctest 3건) |

## 차이(작업 지시와 다른 점)

- 선행 `Phase VI`은 phase 단위 선행이라 사용자 지시(2026-11-14)에 따라 완화했습니다. 필요한 코드(core-types id,
  document 모델)는 존재하며, VI.10.5만 `[~]`(CI 등록 미완)입니다.
- `WindowSessionId`는 V.9.1에서 core-types에 이미 정의되어 있어 새로 정의하지 않고 재수출했습니다.

## 남은 위험 / 한계

- FutureAgent의 실제 동작(lease, 승인, 취소 시 원자 rollback)은 Phase XIX 범위입니다.
