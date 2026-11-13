# 작업 보고서: XVI.6.1 제한 YAML parser fuzz target

- WBS: XVI.6.1
- 작업명: restricted YAML parser fuzz target 추가(panic·hang·과도한 할당 corpus 0)
- 상태: 완료 (`[x]`)
- 선행: IV.1.4(스칼라 엄격 해석; 제한 loader IV.1.2·writer IV.1.5 포함)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증, debug 빌드 `cargo test`).

## 변경 파일 목록

- `crates/project/tests/restricted_yaml_fuzz.rs` (신규: 결정론적 fuzz harness, 계측 할당자, 4개 `#[test]`)
- `crates/project/tests/fuzz_corpus/restricted_yaml/*.yaml` (신규 seed corpus 12개: 매니페스트·잠금 파일,
  금지 문법(anchor/alias/tag/merge key/다중 document/중복 키/비스칼라 키), YAML 1.1 혼동 스칼라, 스칼라
  스타일·directive, BOM·tab)
- `crates/project/src/yaml/loader.rs` (fuzz 발견 수정: 크기·문자·깊이·선읽기 상한, 반복형 event 처리,
  hash 기반 중복 키 판정, 오류 종류 4개 추가, 단위 테스트 7개 추가)
- `crates/project/src/yaml/writer.rs` (fuzz 발견 수정: 제어 문자 `\uXXXX` 이스케이프, 긴 키 explicit
  key 기록, 단위 테스트 2개 추가)
- `docs/task-reports/XVI.6.1.md`(신규), 계획서(XVI.6.1 `[ ]`→`[x]`)

## 구현 요약

### fuzz harness (`tests/restricted_yaml_fuzz.rs`)

- 입력: seed corpus 파일 + 공개 모델(`Manifest`·`Lockfile`) canonical 출력 + 무작위 값 트리의 block·flow
  출력 + 고정 stress 입력 22종(20만 단계 block 중첩, 1,000단계 block mapping, flow 300단계, 키 9만 개,
  flow 키 6만 개, 항목 40만 개, 1 MiB 스칼라·주석·공백, 문서 10만 개, alias 증폭, 크기 상한 경계 등).
- 변형: 고정 seed SplitMix64로 byte 뒤집기·삽입·구간 삭제·YAML token 삽입·구간 복제·다른 seed splice·
  구간 반복·절단·들여쓰기 변경·CRLF 변환(입력당 1~8회, 64 KiB 상한). 같은 seed는 같은 입력열을
  만듭니다(`campaign_inputs_are_deterministic`).
- 대상: `load_restricted` → 성공 시 `Manifest::from_yaml`, `Lockfile::from_yaml`, 모든 스칼라에
  `parse_bool`/`parse_u32`, `write_canonical` 후 재로드 값 일치(round-trip).
- 판정(입력마다):
  - **panic**: `catch_unwind`.
  - **hang**: 입력당 3 s 상한 + 60 s 진행 없음 watchdog(worker thread, 기본 2 MiB stack).
  - **과도한 할당**: 테스트 바이너리 전용 `#[global_allocator]`(System 위임, thread-local 현재·peak
    기록)로 `load_restricted`의 peak heap을 재고 `64 × 입력 길이 + 384 × min(입력 길이, 선읽기 상한)
    + 256 KiB` 이하인지 확인합니다(값 트리 몫 + scanner token 버퍼 몫 + 고정 여유).
- 발견 입력은 `CARGO_TARGET_TMPDIR/restricted-yaml-fuzz/`에 저장하고 테스트를 실패시킵니다. 기본 반복
  25,000회, `STELLAR_YAML_FUZZ_ITERATIONS`로 늘릴 수 있습니다.

### 수정 전 harness 결과(실제 측정)

harness를 수정 전 loader/writer에 그대로 적용해 확인했습니다. 이 실행용 임시 사본은 삭제했고 커밋하지
않았습니다. 이 실행의 할당 상한은 당시 기준(`96 × 입력 + 256 KiB`)입니다.

| 발견 | 입력 | 결과 |
|---|---|---|
| stack overflow(abort) | 20만 단계 block 시퀀스(`- - - …`, 약 400 KB), 1,000단계 block mapping | `SIGABRT` — 하위 parser의 재귀 `load` |
| hang | 키 9만 개 block mapping / flow 키 6만 개 | 73.27 s / 32.72 s — 중복 키 선형 탐색(키 수 제곱) |
| 과도한 할당 | 최상위 flow 시퀀스 40만 항목(800,003 B) | peak 134,217,888 B(입력 1 byte당 167.8 B). parser 단독 측정도 같은 비율로, 하위 scanner가 flow collection 전체를 token으로 버퍼링함 |
| 과도한 할당 | flow 키 6만 개(648,892 B) | peak 66,270,176 B |
| 조용한 잘림 | `"a: b\n\0\nc: [unclosed\n"` | `Ok({a: b})` — scanner가 NUL을 입력 끝으로 처리해 뒤 내용(잘못된 문법 포함)을 버림 |
| round-trip 실패 12건 | 이스케이프로 만든 NUL 등을 담은 값, 1,024자 초과 키 | writer가 원문 NUL을 기록 → 재로드 실패, 긴 키를 implicit key로 기록 → 재로드 실패 |

### loader 수정(`yaml/loader.rs`)

- `MAX_INPUT_BYTES = 1 MiB` 초과는 파싱 전에 `input-too-large`.
- YAML 1.2 `c-printable`이 아닌 문자(원문 NUL·C0·DEL·C1(NEL 제외)·U+FFFE/FFFF)는 줄·열과 함께
  `non-printable`. 이스케이프(`"\0"` 등)로 쓴 제어 문자는 값으로 허용합니다.
- 재귀 `Parser::load` 대신 `next_token`으로 event를 하나씩 당기고 첫 오류에서 멈춥니다. 중첩은
  `MAX_NESTING_DEPTH = 64`까지(`nesting-too-deep`).
- 문자 iterator가 이벤트 사이 선읽기를 세어 `MAX_EVENT_LOOKAHEAD_CHARS = 32 Ki`자를 넘으면 입력 끝을
  돌려주고, loader는 그 뒤 결과와 무관하게 `lookahead-too-large`로 거부합니다(잘린 입력을 성공으로
  보지 않음). 이 상한이 scanner의 token 버퍼를 묶습니다.
- 중복 키는 매핑별 `RandomState` 키 hash 집합으로 판정하고 hash가 겹칠 때만 실제 키를 비교합니다.
- 기존 오류 코드·위치·동작은 유지합니다(기존 단위 테스트 무수정 통과).

### writer 수정(`yaml/writer.rs`)

- C0(`\t`·`\n`·`\r`는 기존 이스케이프 유지)·DEL·C1·U+2028/2029·U+FEFF·U+FFFE/FFFF를 `\uXXXX`로 기록합니다.
- 렌더링 길이가 1,024자를 넘는 키는 `? <key>` / `: <value>` explicit key로 기록합니다. 그 외 출력은
  이전과 byte-identical입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 146 passed, 0 failed; 통합 `restricted_yaml_fuzz` 4 passed, 0 failed) |
| `STELLAR_YAML_FUZZ_ITERATIONS=200000 cargo test -p stellar-project --test restricted_yaml_fuzz restricted_yaml_fuzz_campaign_has_no_findings -- --nocapture` | 종료 코드 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |

| 캠페인 | 실행 입력 | 발견(panic/hang/할당/round-trip) | 시간 | 측정값 |
|---|---|---|---|---|
| 기본(25,000회) | 25,036 | **0** | 2.6 s | peak 최대 17,957,944 B(키 9만 개), 선읽기 상한보다 긴 입력의 최대 비율 22.4 B/byte, 입력당 최대 0.71 s |
| 확장(200,000회) | 200,036 | **0** | 11.2 s | 같은 최대값, 1 KiB 이상 입력 최대 비율 184.5 B/byte(64 KiB 이하 flow 변형) |

| 단위 검사 | 결과 |
|---|---|
| 크기 상한 초과 → `input-too-large`, 할당 4 KiB 미만 | 통과 |
| 원문 NUL → `non-printable`(2:1), 잘림 없음 / TAB·NEL·NBSP·BOM·이모지 허용 / 이스케이프 제어 문자 허용 | 통과 |
| 10만 단계 block 시퀀스·200단계 block mapping → `nesting-too-deep`(stack overflow 없음), 깊이 64 허용·65 거부 | 통과 |
| 키 5만 개 5 s 이내 로드, 그 뒤 중복 키 줄 번호 정확 | 통과 |
| 32 Ki자 초과 스칼라·최상위 flow·연속 주석 거부, 상한 이하 값은 잘림 없이 허용, 긴 block 문서 허용 | 통과 |
| writer: 제어·구분 문자 이스케이프 round-trip, 1,024/1,025자 키 경계와 escape 확장 키 round-trip | 통과 |
| 계측 할당자 자체 검사(1 MiB 할당 감지, 무할당 0) | 통과 |

## 차이(작업 지시와 다른 점)

- "fuzz target"을 cargo-fuzz/libFuzzer로 만들지 않았습니다. 새 의존성과 nightly toolchain이 필요하기
  때문입니다. 대신 저장소 안의 결정론적 harness(고정 seed 생성기 + seed corpus 변형 + 고정 stress
  입력)를 `#[test]`로 실행합니다. coverage-guided가 아닙니다.
- 검증 기준(발견 0)을 만족시키려고 fuzz가 찾은 결함을 IV.1.2 loader와 IV.1.5 writer에서 수정했습니다.
  그 결과 이전에는 받던 입력 일부를 이제 거부합니다: 원문 제어 문자, 1 MiB 초과, 65단계 이상 중첩,
  이벤트 하나에 32 Ki자 넘는 선읽기(긴 스칼라·최상위 flow·연속 주석).
- 과도한 할당 판정을 위해 테스트 바이너리에만 `unsafe impl GlobalAlloc`(System 위임) 계측 할당자를
  두었습니다. 제품 코드에는 unsafe가 없습니다.

## 남은 위험 / 한계

- coverage-guided fuzzing이 아니므로 발견력이 libFuzzer보다 낮습니다. 장기 실행과 crash artifact
  보관은 XVI.6.7(nightly job) 범위입니다.
- 상한 값(1 MiB, 깊이 64, 선읽기 32 Ki자)은 보안 기본값입니다. 큰 lockfile이나 긴 주석이 필요한 제품
  요구가 생기면 조정해야 할 수 있습니다.
- 할당 상한 계수(token 128 B × 3)는 yaml-rust2 0.11.0 내부 구조를 측정해 정한 값입니다. 의존성 버전을
  올리면 다시 측정해야 합니다.
- 시간 상한은 debug 빌드의 wall-clock 기준입니다(입력당 3 s, 측정 최대 0.71 s). 매우 느린 CI에서는
  여유가 줄어듭니다. watchdog가 hang을 실패로 보고해도 worker thread는 프로세스가 끝날 때까지 남습니다.
- `tools/xtask/src/yaml`의 별도 제한 loader는 이 작업 범위 밖입니다. 같은 결함(NUL 잘림, 재귀 stack
  overflow)이 있는지는 확인하지 않았습니다.
