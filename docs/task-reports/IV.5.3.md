# 작업 보고서: IV.5.3 비신뢰 probe 안전 제약

- WBS: IV.5.3
- 작업명: 비신뢰 프로젝트 probe는 메타데이터만 읽고 git 실행·외부 symlink·필터 금지
- 상태: 완료 (`[x]`)
- 선행: IV.5.1(ProjectProbe), IV.5.2(신뢰 상태)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증; symlink/악성 fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/project/src/probe.rs` (`safe_probe`, `SafeProbeError`, 보안 테스트)
- `docs/task-reports/IV.5.3.md`(신규), 계획서(IV.5.3 `[ ]`→`[x]`)

## 구현 요약

- `safe_probe(path, trust)`: **신뢰되지 않은** 프로젝트는 **문서 메타데이터(`stellar.yaml`)만**
  읽습니다.
  - **Git 미실행**: `.git`은 존재만 확인하고 git 프로세스를 실행하지 않습니다(probe는 fs 읽기 +
    파싱만; 외부 프로세스 미생성).
  - **외부 symlink 미추종**: `stellar.yaml`이 symlink면 따르지 않고
    `UntrustedSymlinkRefused`로 거부합니다(외부 파일 노출 방지).
  - **필터 미실행**: git을 실행하지 않으므로 clean/smudge·hook·fsmonitor 등 어떤 필터도 실행되지
    않습니다(구조적 불가능).

## 적대적/보안 검증(ultracode)

- **악성 fixture 무프로세스**: `.git/config`에 `core.fsmonitor = touch <marker>`를 심은 악성
  프로젝트를 비신뢰로 probe해도 `marker`가 생기지 않음을 확인(git/외부 프로세스 미실행).
- **symlink 거부**: 외부 파일을 가리키는 `stellar.yaml` symlink를 비신뢰에서 거부.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-project` (106 unit) | 0 |
| `cargo fmt -p stellar-project -- --check` | 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| 비신뢰 + 정상 매니페스트 → 메타데이터만 읽음 | 통과 |
| 비신뢰 + symlink 매니페스트 → 거부 | 통과 |
| **악성 fsmonitor fixture → marker 미생성(외부 프로세스 미실행)** | 통과 |

## 차이(작업 지시와 다른 점)

- 외부 symlink 정책은 비신뢰에서 매니페스트 symlink를 **일괄 거부**하는 보수적 정책을 택했습니다
  (in-project vs external 구분보다 안전 우선).

## 남은 위험 / 한계

- 본 probe는 매니페스트만 읽습니다. 신뢰 프로젝트의 전체 문서 로드·git 연동은 후속 Phase에서
  trust를 확인한 뒤 수행합니다.
- Phase IV(프로젝트 매니페스트·생성·열기) 전체가 완료되었습니다.

## 후속 수정(보안 리뷰 FR-1·FR-2 반영): 매니페스트를 연 handle 기준으로 판정

- **문제**: `safe_probe`는 `symlink_metadata`로 symlink 여부를 본 뒤 `File::open`(symlink를 따라감)으로
  다시 열었고, root 검증의 `is_file()`과 열기 사이에도 간격이 있었습니다. 그 사이 `stellar.yaml`을 외부 파일
  symlink로 바꾸면 비신뢰 probe가 프로젝트 밖 파일을 매니페스트로 읽었고(FR-2), FIFO로 바꾸면 blocking
  `open()`이 writer를 기다리며 document thread를 멈췄습니다(FR-1 트리거).
- **수정**: 판정을 연 handle로 옮겼습니다. macOS에서는 `O_NONBLOCK`으로 열고(FIFO·장치에서 멈추지 않음),
  비신뢰 probe는 `O_NOFOLLOW`를 함께 줘 검사 뒤 symlink로 바뀌면 `ELOOP`를
  `UntrustedSymlinkRefused`로 보고합니다. 연 handle의 `fstat`이 정규 파일이 아니면
  `ManifestRead(InvalidInput)`입니다. 경로 사전 검사(lstat)는 일찍 거부하는 데만 씁니다. macOS 밖
  unix는 flag 값이 플랫폼마다 달라 주지 않고, 비신뢰 probe는 lstat한 항목과 연 handle의 device·inode가
  다르면 거부합니다(FIFO 교체로 멈출 수 있는 한계는 남음, ADR-0006 보류 플랫폼).
- **검증**: `untrusted_probe_never_reads_a_manifest_swapped_to_an_external_symlink`(rename 교체 4,000회 경합,
  수정 전 외부 파일 내용을 매니페스트로 읽어 실패), `probe_does_not_block_on_a_manifest_swapped_to_a_fifo`
  (FIFO hard link 교체 경합, 수정 전 5초 안에 probe가 돌아오지 않아 실패),
  `opened_handle_decides_fifo_and_symlink_manifests`(읽기 함수 직접 호출: FIFO·FIFO symlink는 바로
  `InvalidInput`, Refuse 정책 symlink는 거부, Follow 정책 symlink는 대상을 읽음).
