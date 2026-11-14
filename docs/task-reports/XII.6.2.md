# 작업 보고서: XII.6.2 Open Project 명령과 잘못된 폴더 안내

- WBS: XII.6.2
- 작업명: Open Project command가 folder를 선택하고 ProjectProbe 후 session을 엽니다. 잘못된 폴더는 원인별로 안내합니다.
- 상태: 완료 (`[x]`)
- 선행: IV.5.3 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (로컬 Xcode 27.2, macOS 27.2, `scripts/macos/smoke.sh`의 headless `--project-open-check`). 폴더 선택 패널은
  검사에서 주입한 chooser로 대신했고, 사람이 `NSOpenPanel`을 조작하지는 않았습니다.

## 변경 파일 목록

- `apps/macos/StellarMac/UI/Shell/ShellProjectOpenFailure.swift`(신규) — 코어 실패 코드(`STELLAR_FAILURE_*`)·FFI 호출
  상태 → 안내 원인, 원인별 제목·해결 방법, 시작 창 안내 종류.
- `apps/macos/StellarMac/UI/Shell/ShellProjectOpenCheck.swift`(신규) — `--project-open-check`.
- `apps/macos/StellarMac/UI/Shell/ShellAppCoordinator.swift` — 열기 실패를 `ShellProjectOpenFailure`로 안내(메뉴·Finder:
  alert, 시작 창: 시작 창 안내, 복원·Locate Folder…: 창 실패 상태). 임시 일반 문구 제거.
- `apps/macos/StellarMac/UI/Shell/ShellProjectWindow.swift` — 창 실패 상태가 원인 제목·해결 방법을 씀, `folderName` nonisolated.
- `apps/macos/StellarMac/main.swift`, `scripts/macos/smoke.sh` — `--project-open-check` 연결.
- `docs/design/ui/shell_frame.md` — 14.8 열기 실패 안내 대응표.
- `docs/task-reports/XII.6.2.md`(신규), 계획서(XII.6.2 `[ ]`→`[x]`).

Open Project의 나머지 경로(메뉴 ⌘O → 폴더 선택 → `stellar_project_open` → 완료 event → 창, 같은 폴더 중복 방지, 창 닫기 시
session 닫기, 복원)는 직전 커밋 `Feat: 셸 프레임을 실제 앱 창으로 연결(…)`에서 앱 통합과 함께 들어갔고, 이 작업은 그 경로의
실패 대응과 검증을 완성합니다. 코어 쪽(`stellar_project_open`의 열기 분류·안전 probe·실패 코드)은 이미 있던 것을 그대로 썼습니다.

## 구현 요약

- **열기 순서**: 메뉴 막대 Open Project…(⌘O, 앱 수준 router라 창이 없어도 동작) 또는 시작 창 Open Project…가 폴더를 고르게
  하고, 코어 `stellar_project_open`에 절대 경로를 넘깁니다. 코어가 document thread에서 열기 분류(`.git`·`stellar.yaml`,
  IV.2.4)와 신뢰되지 않은 프로젝트용 안전 probe(매니페스트 메타데이터만, symlink 매니페스트 거부, IV.5.3)를 마치고
  `PROJECT_OPENED`를 보내면 그때 프로젝트 창과 session handle을 만듭니다. UI 스레드는 기다리지 않고 완료 event를 pull합니다.
- **안내 대응**: 코어 실패 코드 8종과 FFI 호출 상태(빈·상대 경로, 대기열 가득, runtime 없음)를 `ShellProjectOpenFailure`
  13개 원인으로 바꿉니다. 원인마다 제목이 다르고, 메시지는 문제와 해결 방법을 함께 말하며 전체 경로 대신 폴더 이름만 보여
  줍니다. 코어 진단 메시지(한국어, 경로 포함 가능)는 화면에 쓰지 않습니다. `NOT_A_REPOSITORY`일 때만 폴더가 아직 있는지
  확인해 "폴더 없음"(옮겨진 최근·복원 프로젝트)을 "Git 저장소 아님"과 구분합니다(실패 경로의 로컬 stat 한 번).
- **안내 위치**: 메뉴 막대·Finder에서 연 실패는 alert, 시작 창에서 연 실패는 시작 창 안내(`stellar.yaml` 없음은
  `.notStellarProject`로 "전환하지 않고 안내만", 폴더 없음은 `.projectMissing`), 복원·Locate Folder… 실패는 그 창의 실패
  상태(복구 동작 Locate Folder…·Try Again·Close Window). 실패한 열기는 창·session을 만들지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `scripts/macos/smoke.sh` | `BUILD SUCCEEDED`(Swift warning·error 0), smoke·lifecycle·canvas·text 구조·text-input·input·update·영역 UI 검사 7종·shell-integration·project-open·UI 구조 검사 모두 OK, `smoke OK`, 종료 0 |
| `StellarMac --project-open-check` | `project-open ok: 164 checks (menu open via core, 11 invalid folder fixtures, mapping, alert/welcome/restore)`, 종료 0 |
| `cargo test -p stellar-ffi -p stellar-runtime` | 57 + 41 + 8 + 2 + 0 + 11 passed, 0 failed(Rust 변경 없음, 코어 열기 경로 근거) |
| `git diff --check` | 통과 |

Rust 코드를 바꾸지 않았으므로 `cargo fmt`·`cargo clippy`는 이 작업의 변경 대상이 없습니다.

| 검사(`--project-open-check`) | 결과 |
|---|---|
| 대응표: 실패 코드 8종·호출 상태 5종·모르는 코드·열기와 무관한 코드 → 원인, 원인별 제목 13개가 서로 다름, 폴더 이름 표시·전체 경로 비노출, 시작 창 안내 종류 | 통과 |
| 메뉴 ⌘O(단축키·말줄임표 확인) → 폴더 선택 취소 시 요청 0 → 폴더 선택 → 코어 요청 1, probe 전 창 없음 → 완료 후 창 1·session 이름 일치·코어 session 1 → 창 닫기 후 session 0 | 통과 |
| 실제 코어로 연 잘못된 폴더 9종의 코드 → 원인: 폴더 없음 → `folderMissing`, 일반 폴더 → `notARepository`, Git만 → `notAStellarProject`, YAML 손상·1 MiB 초과 → `manifestCorrupt`, 스키마 위반 → `manifestInvalid`, `project.id: not-a-uuid` → `invalidProjectID`, 심볼릭 링크 매니페스트 → `untrustedSymlink`, 권한 000 매니페스트 → `manifestUnreadable` | 통과 |
| 같은 9종을 메뉴로 열 때 alert 제목 = 원인 제목, 폴더 이름 포함, 전체 경로 비노출, 창·session 0 | 통과 |
| 상대 경로·빈 경로 → 코어가 요청을 받지 않음(`STELLAR_INVALID_ARGUMENT`) → `invalidPath` | 통과 |
| 시작 창에서 연 실패: Git만 → `.notStellarProject`, 폴더 없음 → `.projectMissing`, YAML 손상 → 원인 제목이 붙은 `.operationFailed`, alert 없음, 진행 표시 해제 | 통과 |
| 복원(없어진 폴더) → 창 실패 상태의 제목·메시지 = `folderMissing` 문구 | 통과 |
| 검사 민감도: `MANIFEST_INVALID`를 `manifestCorrupt`로, 폴더 없음 분기를 `notARepository`로 바꾼 변이에서 9건 실패(종료 1) 확인 후 원복·재빌드 | 확인 |

## 차이(작업 지시와 다른 점)

- 폴더 선택은 검사에서 `NSOpenPanel` 대신 주입한 chooser로 대신했습니다. 실제 앱은 `NSOpenPanel`(폴더만, 여러 개 선택 불가,
  시작 창이 있으면 sheet)을 씁니다.
- `PATH_UNRESOLVED`(17)는 열기 중 폴더가 사라지는 경쟁 상황이라 fixture로 재현하지 않고 대응표 검사만 했습니다.

## 남은 위험 / 한계

- 코어는 아직 신뢰 저장소 연결 전이라 모든 프로젝트를 신뢰되지 않음으로 엽니다. 신뢰된 프로젝트의 symlink 매니페스트
  허용 경로는 신뢰 UI(XVI.7.2)가 생기면 다시 확인해야 합니다.
- 최근 프로젝트 목록 코어 경로가 없어 시작 창의 최근 목록은 비어 있습니다(Open Recent 없음).
- 같은 폴더 판정은 셸이 경로를 표준화(심볼릭 링크 해소)해 비교합니다. 코어 `ProjectIdentity` 기준 중복 판정과 다중 창 정책은
  XII.2.4(선행 XII.2.1 미완)에서 정합니다.
