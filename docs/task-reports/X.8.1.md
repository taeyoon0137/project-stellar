# 작업 보고서: X.8.1 FontSource와 FontFaceRef 정의

- WBS: X.8.1
- 작업명: `FontSource`(Project, System, External)와 file hash·face index·variation axes를 담은 `FontFaceRef` 정의
- 상태: 완료 (`[x]`)
- 선행: III.3.5(`TypedValue`, `[x]`), V.1.5(`ContentHash`, `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증. `planning/task-status.yaml`의 required_environments는 `DEV-MAC`만 지정).

## 변경 파일 목록

- `crates/text/src/font.rs`(신규) — `OpenTypeTag`, `FontVariation`, `FontSource`, `FontFileHash`, `ParseFontFileHashError`, `FontFaceRef`, `FontRecordError`, crate 내부 `ByteReader`, 단위 테스트.
- `crates/text/src/lib.rs` — `pub mod font;` 선언, crate 문서 갱신.
- `docs/task-reports/X.8.1.md`(신규), 계획서(X.8.1 `[ ]`→`[x]`).

## 구현 요약

- **`FontSource`**: Project(1), System(2), External(3)의 안정 numeric tag와 진단용 이름을 둡니다. 0과 4 이상은
  알 수 없는 tag로 거부합니다(0을 비워 두어 0으로 채워진 데이터를 잡습니다).
- **`FontFileHash`**: 폰트 파일 SHA-256 digest(32바이트) 값 타입입니다(ADR-0004). `text`는 `storage`에 의존할 수
  없어 hash를 계산하지 않고 호출자(폰트 import·discovery 계층)가 계산한 digest를 받습니다. 문자열 형식은 저장
  `ContentHash`와 같은 `sha256:<64자리 소문자 16진수>`이고, 알고리즘 이름 누락·다른 알고리즘·길이 오류·대문자 hex를
  거부합니다.
- **`FontFaceRef`**: source, file hash, face index, variation axes(`FontVariation { tag, value: FiniteF32 }`)를 담습니다.
  생성자는 axis를 tag 오름차순으로 정렬하고 같은 tag는 마지막 값을 남깁니다(VI.7.2 style run 규칙과 동일).
- **canonical binary**: `u8 source tag` + `[32] digest` + `u32 LE face index` + `u32 LE axis 개수` +
  `{[4] tag, f32 LE}`. decode는 잘린 입력, 잔여 bytes, 알 수 없는 source tag, 비ASCII tag, NaN·무한대, 정렬 위반·중복
  tag를 모두 오류로 반환하므로 같은 값의 bytes 표현은 하나뿐입니다. 큰 axis 개수는 남은 입력 길이까지만 예약해
  큰 할당을 만들지 않습니다.
- **`OpenTypeTag`**: 문서 모델(VI.7.2)과 같은 규칙(출력 가능한 ASCII 4바이트)의 text-local 값 타입입니다.

## 실행한 명령 / 검증 (`DEV-MAC`, Apple Silicon arm64)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-text` | 종료 코드 0 |
| `cargo test -p stellar-text` | 종료 코드 0 (11 unit passed, 0 failed) |
| `cargo clippy -p stellar-text --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-text -- --check` | 종료 코드 0 |
| `cargo run -q -p xtask -- validate-crate-graph` | 종료 코드 0 |
| `cargo run -q -p xtask -- validate-dependencies` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **round-trip**: 세 source × axis 0·1·3개 조합의 encode→decode가 원래 값과 같고 재인코딩 bytes도 같음(`face_ref_round_trips_for_every_source`) | 통과 |
| canonical golden bytes 고정(`face_ref_golden_bytes`), source tag·이름 golden(`source_tags_are_stable_golden`) | 통과 |
| axis 입력 순서·중복·`-0.0`과 무관하게 같은 bytes(`constructor_canonicalizes_variation_order_and_duplicates`) | 통과 |
| 모든 prefix 길이 truncation, trailing byte 거부 | 통과 |
| 알 수 없는 source tag(0, 4, 255), 정렬 위반, 중복 tag, NaN·+Inf, 제어 문자 tag 거부 | 통과 |
| axis 개수 `u32::MAX` 입력이 큰 할당 없이 `Truncated` | 통과 |
| file hash 문자열 round-trip(빈 입력 SHA-256 공개 vector 사용), 71자 고정, 파싱 오류 4종 | 통과 |

## 차이(작업 지시와 다른 점)

- file hash는 저장 `ContentHash`를 직접 쓰지 않고 같은 형식의 text-local `FontFileHash`로 정의했습니다. crate 의존
  정책상 `stellar-text`는 `stellar-core-types`만 의존할 수 있기 때문입니다. 두 타입은 32바이트 digest와 문자열
  형식이 같아 상위 계층(XIII.6.1 폰트 import)이 손실 없이 변환할 수 있습니다.
- `FontSource` variant에는 데이터(에셋 ID, 패키지 ID 등)를 넣지 않았습니다. 프로젝트 에셋 연결은 XIII.6.1, 외부
  패키지 참조는 XIII.6.4가 담당하므로 이 작업에서 공용 필드를 선점하지 않았습니다.

## 남은 위험 / 한계

- `TypedValue::Typography`(III.3.5)는 아직 불투명 placeholder입니다. typography 값에 `FontFaceRef`를 연결하는 일은
  `crates/core-types` 변경(ADR 필요)이라 이 작업 범위 밖입니다.
- hash 값이 실제 파일 bytes와 일치하는지는 호출자 책임입니다. 이 타입은 검증하지 않습니다.
