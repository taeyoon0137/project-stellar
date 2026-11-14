# ADR-0015: 저장 decoder 경화(fuzz 발견 수정)

## Status

Accepted (2026-11-14, 구현 기록)

## Context

기술 아키텍처 §37.2(파서)는 신뢰되지 않은 입력에 대해 "청크 길이·참조·해시 검증", "깊이·개수
제한", "Fuzzing 필수"를 요구하고, §38.2는 "YAML·binary parser fuzzing"을 필수 테스트 층으로
둡니다. 구현 계획 XVI.6.2는 chunk envelope/canonical decoder fuzz target을 추가하고 "malformed
input panic 0"을 검증 기준으로 둡니다.

`crates/storage/`는 ADR 보호 경로(I.3.5)입니다. fuzz가 찾은 결함을 고치는 변경도 저장 포맷
parser의 동작을 바꾸므로 이 ADR에 기록합니다.

발견 기록:

| 날짜 | WBS | 발견 | 위치 | 수정 |
|---|---|---|---|---|
| 2026-11-14 | XVI.6.2 | `ContentHash` 문자열 parse가 다중 byte UTF-8 문자에서 panic | `crates/storage/src/hash/mod.rs` | slice 전 문자 검사 |
| 2026-11-14 | 보안 감사 STOR-1 | current pointer·manifest·chunk 읽기가 symlink를 따라 장치·FIFO·거대 파일을 크기 확인 없이 끝까지 읽음(`/dev/zero`에서 메모리 무한 할당, FIFO에서 무한 대기) | `crates/storage/src/generation/current.rs`, `crates/storage/src/generation/loader.rs`, `crates/storage/src/chunk/store.rs` | 연 handle `fstat`으로 정규 파일·크기 상한 확인, 상한 + 1 bytes에서 읽기 중단, macOS `O_NONBLOCK` |
| 2026-11-14 | 보안 감사 STOR-4 | chunk·manifest·current 쓰기가 `create_dir_all` 뒤 경로로 기록해 symlink 디렉터리를 따라 프로젝트 밖에 디렉터리·파일을 만듦 | 같은 세 파일 | 구성 요소별 `lstat`·한 단계 `create_dir`, temp 생성·rename 직전 identity 재확인, 기존 파일 검증은 `O_NOFOLLOW` |
| 2026-11-14 | 보안 감사 STOR-5 | atomic write가 temp를 `O_TRUNC`로 열어, 예측 가능한 temp 이름에 심어 둔 symlink를 따라 프로젝트 밖 파일을 덮어씀 | `crates/storage/src/atomic_file.rs` | temp는 `create_new`로만 만들고 이미 있는 이름은 건너뜀 |

XVI.6.2 발견의 원인: parse는 hex 부분의 **byte 길이**(64)만 확인한 뒤 2 byte 단위로 slice합니다.
hex 자리에 2·3·4 byte 문자가 있으면 byte 길이는 맞아도 slice 경계가 문자 안쪽에 떨어져 panic합니다
(예: `sha256:` + `€` + `a`×61, 총 71 byte). 이 parse는 신뢰되지 않은 입력에서 호출됩니다.

- chunk envelope의 payload hash(V.2.2, chunk 읽기 V.2.6)
- chunk pointer·generation manifest의 content hash(V.3.1, V.4.1)
- `design/current` 파일 내용(V.4.3)
- fallback scanner가 해석하는 `design/generations/` 파일 이름(V.4.5)

release profile은 `panic = "abort"`(II.1.3)이므로 손상·조작된 chunk 하나, 또는 파일 이름 하나로
프로젝트 열기 경로가 프로세스를 끝낼 수 있었습니다.

보안 감사·종단 테스트(2026-11-14)는 같은 비신뢰 입력 경계(Git으로 받은 프로젝트 폴더)에서 파일을 열고
쓰고 지우는 경로의 결함을 찾았습니다(감사 지적 STOR-1부터 STOR-6까지, 위 발견 기록 표). parser 결함은
아니지만 같은 보호 경로의 같은 위협이라 이 ADR에 함께 기록합니다.

## Decision

1. `ContentHash::from_str`은 slice 전에 hex 부분의 모든 문자를 앞에서부터 검사하고, 소문자 ASCII
   16진수가 아닌 첫 문자를 `ParseContentHashError::InvalidHexDigit { found }`로 보고합니다. 검사를
   통과한 hex는 ASCII뿐이므로 2 byte slice는 항상 문자 경계입니다.
2. ADR-0004의 문자열 형식(`sha256:<64자리 소문자 16진수>`), 오류 종류, 수용 집합은 바꾸지 않습니다.
3. 회귀 방지는 결정론적 fuzz harness `crates/storage/tests/chunk_canonical_fuzz.rs`(XVI.6.1과 같은
   방식: 고정 seed, 새 의존성·nightly 없음)와 seed corpus `crates/storage/tests/fuzz_corpus/
   chunk_canonical/`로 고정합니다. 발견 입력은 corpus에 회귀 seed로 남깁니다.
4. 이후 저장 decoder의 fuzz 발견 수정도 위 발견 기록 표에 추가합니다.
5. 보안 감사 발견(STOR) 수정은 공개 API(오류 enum variant·함수 signature)와 파일 포맷을 바꾸지
   않습니다. 새로 거부하는 경우는 기존 `Io` variant로 보고합니다. 규칙은 다음과 같습니다.
   - 저장소 파일 읽기(STOR-1): 마지막 구성 요소 symlink는 따릅니다(기술 아키텍처 19.3 "심볼릭 링크
     전면 허용", project loader의 사용자 작업 트리 규칙과 같음). 연 handle을 `fstat`해 정규 파일이고
     크기가 상한 이하일 때만 읽고, 읽기도 상한 + 1 bytes에서 멈춥니다. macOS는 `O_NONBLOCK`으로 열어
     FIFO·장치 열기에서 기다리지 않습니다. 상한은 current pointer 72 bytes(형식 크기), manifest 파일
     64 MiB, chunk 파일 256 MiB입니다. manifest·chunk 상한은 제품 예산이 아니라 비신뢰 입력 방어
     상한이며 project loader 기본 상한(XIV.2.7)과 같습니다. 쓰기도 같은 상한을 넘는 파일은 기록하지
     않습니다(다시 읽을 수 없는 세대를 만들지 않음).
   - 저장소 파일 쓰기(STOR-4): `design`·`design/objects`·shard·`design/generations`가 symlink면 따라가지
     않고 거부합니다(assets의 symlink 에셋 디렉터리 거부와 같은 규칙). 없으면 한 단계씩 `create_dir`로
     만들고, temp 생성·rename 직전에 같은 디렉터리 identity(`(dev, ino)`)인지 다시 확인합니다.
     content-addressed 파일의 dedup 검증은 기존 항목을 따라가지 않고(macOS `O_NOFOLLOW`) 기대 크기까지만
     읽습니다. symlink·정규 파일이 아닌 항목·크기가 다른 파일은 불변성 위반(`CorruptExisting`)으로
     보고하고 덮어쓰지 않습니다.
   - 공통 검사는 crate 내부 `fs_guard` 모듈에 둡니다(공개 API 아님). 거부 이유는 `Io` variant의
     `std::io::Error`에 담깁니다.
   - atomic write temp(STOR-5): `create_new`(`O_CREAT`·`O_EXCL`, 마지막 구성 요소 symlink를 따르지
     않음)로만 만듭니다. 이름이 이미 있으면 열거나 지우지 않고 다음 이름으로 최대 16번 다시
     만듭니다(assets `atomic_write`와 같은 규칙). 실패 정리는 이 호출이 만든 temp만 지웁니다.

## Alternatives

- **byte 단위 직접 변환**(`as_bytes()` + nibble 계산): 결과는 같지만 오류에 담을 `char`를 따로
  복원해야 해 변경이 커집니다 — 미선택.
- **오류를 삼키는 `catch_unwind` 감싸기**: release가 `panic = "abort"`라 동작하지 않고, 결함을
  숨깁니다 — 미선택.
- **cargo-fuzz/libFuzzer**: 새 의존성과 nightly toolchain이 필요합니다(XVI.6.1과 같은 이유) — 미선택.

## Consequences

- 신뢰되지 않은 hash 문자열은 panic 대신 오류가 됩니다: envelope·pointer·manifest는 `InvalidHash`,
  current pointer는 `InvalidFormat`, fallback scanner는 그 파일을 후보에서 제외합니다.
- 이전에 수용하던 입력 중 거부로 바뀐 것은 없습니다. 이전에 panic하던 입력만 오류로 바뀝니다.
  ASCII 입력은 이전과 같은 첫 위반 문자를 보고합니다(이전에도 문자열 순서로 검사).
- hex 64자를 한 번 더 순회합니다.
- (STOR-1) 64 MiB를 넘는 manifest 파일과 256 MiB를 넘는 chunk 파일은 읽기·쓰기 모두 거부합니다.
  이전에는 storage가 받았지만, project crate `SavedProjectLoader`는 이미 같은 기본 상한으로 거부했습니다.
  정규 파일이 아닌 저장소 항목은 읽지 않고 `Io`(`InvalidInput`)입니다. 이전에는 디렉터리면 `Io`
  (`IsADirectory`), 장치·FIFO면 무한 읽기·대기였습니다.
- (STOR-4) symlink 디렉터리 아래로의 저장은 거부됩니다. 읽기는 19.3에 따라 symlink를 따르므로, `design`
  등을 symlink로 둔 작업 트리는 열 수 있지만 저장할 수 없습니다. 저장소 경로가 아닌 프로젝트 root
  자체의 symlink는 그대로 허용합니다.
- 디렉터리 fd 기준 연산(`openat`·`renameat`)이 std에 없어, 디렉터리를 다시 확인한 직후와 경로 기반
  syscall 사이의 짧은 창은 남습니다. commit된(정적) symlink는 모두 차단하고, 그 창에 디렉터리를
  교체하는 동시 공격자만 남습니다. macOS 밖 Unix는 flag 없이 열기 전 `stat`으로 종류를 확인합니다.
- (STOR-5) temp 이름 16개가 모두 이미 있으면 저장 단계가 `AlreadyExists`로 실패합니다. 이전에는 그
  항목을 열어 덮어썼습니다. 이름 형식(`.<이름>.tmp-<pid>-<n>`)은 그대로입니다.

## Compatibility

- 파일 포맷·문자열 형식·공개 API·FFI 변경 없음.

## Performance

- 성능 예산 영향 없음(hash parse당 64자 추가 순회 1회).

## Security

- §37.2 파서 경계: 신뢰되지 않은 프로젝트의 chunk·manifest·current pointer·파일 이름이 앱을
  종료시키는 경로(서비스 거부)를 제거합니다.

## Migration

- 데이터 마이그레이션 없음.
