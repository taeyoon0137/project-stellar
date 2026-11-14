# 작업 보고서: XXIV.5.4 `stellar` 명령 설치·제거 로직

- WBS: XXIV.5.4
- 작업명: 사용자 동의 뒤에만 파일을 바꾸는 설치·제거 로직과 ADR-0005 §2 대체 이름 규칙(`stellar` → `stellar-cli` → 설치하지 않고 알림)을 구현하고, 설치 방식과 안전 조건을 기록합니다.
- 상태: 미완료 (`[ ]` 유지). 선행 XXIV.5.3이 `[ ]`(엄격 선행 XXIV.5.1 대기)이므로 완료로 올리지 않았습니다. 구현과 `DEV-MAC` fixture 검증은 시작 창 검토 수정 커밋(`a481400`)에서 끝났습니다.
- 선행: XXIV.5.3 (`[ ]`, 엄격)

## 필수 실행 환경

- `DEV-MAC`: Apple silicon(arm64), macOS 27.2, Xcode 27.2, Debug 빌드.

## 변경 파일 목록

- `apps/macos/StellarMac/UI/Welcome/StellarCommandInstaller.swift`: 이름 결정(`naming`, `shadowingProgram`), 소유 링크 판정(번들 식별자 확인), 설치·제거, `LocalStellarCommandFileSystem`의 `RENAME_SWAP`·확인 후 되돌리기, 메모리 구현의 변경 횟수·경합 hook
- `apps/macos/StellarMac/UI/Welcome/WelcomeUICheck.swift`: fixture I1, N1~N10, R1~R5, E1, 임시 폴더 실디스크 검사
- `docs/design/ui/welcome.md` 6.2·6.4절(정의 원본)
- `docs/task-reports/XXIV.5.4.md`(이 보고서). 계획서 상태 표기는 바꾸지 않았습니다.

## 결정: 설치 방식과 안전 조건

정의 원본은 `docs/design/ui/welcome.md` 6.4절입니다.

| 항목 | 결정 | 근거 |
|---|---|---|
| 설치 방식 | 후보 폴더에 앱 번들 helper를 가리키는 심볼릭 링크 하나. 복사, 래퍼 스크립트, 셸 설정 파일 수정을 쓰지 않습니다. | ADR-0005 §7, 아키텍처 §37(최소 권한). 링크라서 앱을 업데이트해도 같은 helper를 가리킴 |
| 동의 | 판정은 읽기만 합니다. sheet의 Install·Install as stellar-cli·Update를 눌러야만 `install(_:)`이 실행됩니다. | ADR-0005 §7 |
| 명령 탐색 모델 | `/`가 없는 이름은 PATH 폴더 순서대로 찾아 처음 만난 실행 파일을 씁니다. 실행 권한 없는 같은 이름 파일은 고르지 않습니다. | zshmisc(1) COMMAND EXECUTION("searches each element of $path for a directory containing an executable file"), bash(1) COMMAND EXECUTION, POSIX Shell Command Language 2.9.1.1 |
| 다른 프로그램 | 링크를 따라가 실행 가능한 일반 파일이고 Stellar 소유 링크가 아닌 것 | 위 탐색 모델 |
| Stellar 소유 링크 | 대상이 `<이름>.app/Contents/Helpers/stellar` 모양이고, 그 번들이 있으면 `CFBundleIdentifier`가 `xyz.taeyoon.stellar`. 번들이 없는(dangling) 링크는 소유로 봄 | 다른 앱의 같은 이름 helper를 소유로 오판하지 않기 위함 |
| 대체 이름 규칙 | `stellar`, `stellar-cli` 순으로 ① 설치 위치에 소유 링크가 아닌 항목 ② 확인한 PATH에서 설치 폴더보다 앞(설치 폴더가 PATH에 없으면 전체)에 다른 프로그램 ③ 순서 모르는 폴더에 다른 프로그램(보수적 충돌) 중 하나라도 있으면 그 이름을 쓰지 않습니다. 둘 다 못 쓰면 설치 0과 알림. 자기 링크는 충돌이 아님 | ADR-0005 §2 |
| PATH 뒤쪽 프로그램 | 이름은 그대로 쓰되 동의 화면에 "설치하면 그 프로그램 대신 Stellar가 실행됨"을 알립니다. | ADR-0005 §2는 "먼저 해석되는" 경우만 대체 이름으로 정함. 동의 품질을 위해 사실을 알림 |
| 새 링크 | `symlink(2)`. 경로에 무엇이 있으면 `EEXIST`로 실패하므로 덮어쓰지 않습니다. | symlink(2) |
| 다시 연결 | 같은 폴더에 새 링크를 만들고 `renamex_np(…, RENAME_SWAP)`로 맞바꿉니다. 맞바뀐 항목이 확인한 소유 링크가 아니면 다시 맞바꿔 원래대로 돌립니다. `ENOTSUP`이면 옆 이름으로 `rename(2)` → 확인 → `symlink(2)`(아니면 `RENAME_EXCL`로 되돌림) | rename(2) `RENAME_SWAP`·`RENAME_EXCL`·`ENOTSUP` |
| 제거 | 후보 폴더의 소유 링크(`stellar`, `stellar-cli`)만 지웁니다. 같은 폴더 임시 이름으로 `rename(2)`한 뒤 확인하고, 아니면 `RENAME_EXCL`로 되돌립니다. 만든 폴더는 남깁니다. | rename(2), 남의 파일 보존 |
| 실행 직전 재확인 | helper, 계획한 이름의 PATH 앞쪽 충돌, 설치 위치 항목. 바뀌었으면 `conflict`이고 변경 0. 충돌 뒤에는 같은 계획을 재실행하지 않고 Check Again으로 새 동의를 받음 | 확인과 실행 사이 경합 방지 |
| 폴더 생성 | 홈 아래일 때만 현재 사용자 권한으로 만듭니다. | §37 최소 권한 |
| 설치 상태 기록 | 첫 실행 제안에 답했는지만 앱 사용자 기본값(`StellarCommandInstallOffered`)에 둡니다. 설치 여부는 파일 시스템에서 매번 판정하고, 프로젝트·Git·텔레메트리에는 남기지 않습니다. | ADR-0005 §7 |
| 실행 전제 | App Sandbox 밖 실행(현재 Xcode 설정: 샌드박스 없음, hardened runtime) | 샌드박스 앱은 홈 아래 임의 폴더에 쓸 수 없음 |

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| Debug `xcodebuild … build`(XXIV.5.3 보고서와 같은 옵션) | `BUILD SUCCEEDED`, Welcome 파일 경고 0 |
| `StellarMac --welcome-ui-check`(임시 실행 인자) 3회 | 3회 모두 `welcome-ui ok: 353 checks [disk installer ok; …]`, 종료 코드 0 |
| `./scripts/macos/smoke.sh` | `smoke OK`, 종료 코드 0 |
| `git diff --check` | 출력 없음 |

| 수용 기준 / fixture | 결과 |
|---|---|
| I1 설치·멱등·제거 fixture, 되돌리기 후 설치 항목 0, 판정만으로 변경 0(`mutationCount == 0`) | 통과 |
| 동의 없는 파일 변경 0: 판정(I1, N6, N9), sheet Not Now·Escape, sheet 표시 전(`mutationCount == 0`) | 통과 |
| N1 설치 위치에 다른 `stellar`(일반 파일) → 덮어쓰기 0, `stellar-cli` 설치, 기본 이름 강제 실행도 `conflict`, 제거는 `stellar-cli`만 | 통과 |
| N2 확인한 PATH(`/usr/local/bin`)에 다른 `stellar`, 설치 폴더는 PATH에 없음 → `stellar-cli`, 남의 프로그램 불변 | 통과 |
| N3 설치 폴더보다 PATH 앞 폴더에 다른 `stellar` → `stellar-cli` | 통과 |
| N4 PATH 뒤쪽에만 다른 `stellar` → `stellar` 유지, 가려질 프로그램 알림 | 통과 |
| N5 순서 모르는 폴더(Homebrew 접두사)의 다른 `stellar` → `stellar-cli`, 실행 권한 없는 파일은 충돌 아님 | 통과 |
| N6·N6b `stellar-cli`도 다른 프로그램(설치 위치·PATH 앞쪽 조합) → 설치 0, `namesUnavailable` 알림 상태, 변경 0 | 통과 |
| N7 Stellar 자기 링크만 있음 → `stellar` 유지(`installed`), PATH의 자기 링크는 충돌 아님 | 통과 |
| N8 다른 앱의 `Contents/Helpers/stellar` 링크 → 소유 아님, 제거 안 함 | 통과 |
| R1 옮겨진 앱을 가리키는 소유 링크(절대·상대·dangling) 다시 연결, 제거 2건 | 통과 |
| R2~R5 확인과 생성·교체·제거 사이 경합, 확인 뒤 PATH 앞쪽 가림 → 변경 0·남의 항목 보존 | 통과 |
| E1 파일 시스템 오류 전달 | 통과 |
| 실디스크(임시 폴더): 설치, `RENAME_SWAP` 다시 연결, 임시 링크 잔여 0, 기대와 다른 항목 교체·삭제 거부, `symlink` EEXIST, 대체 이름 설치·되돌리기 | 통과 |

변이 확인(커밋하지 않고 되돌림): `stellar` 이름의 PATH 가림 판정을 끄면 N2·N3·N5·N6b·N9와 sheet 대체 이름 검사가 실패. 맞바꾼 항목 확인을 건너뛰면 `disk: 남의 파일을 교체함` 실패.

## 차이(작업 지시와 다른 점)

- 구현은 이 행의 WBS 커밋이 아니라 시작 창 검토 수정 커밋(`Fix:`)에 있습니다(XXIV.5.3 보고서와 같은 이유).
- 순서 모르는 폴더의 충돌을 보수적으로 충돌로 보는 것은 ADR-0005 §2 "PATH 앞쪽에 있어 가려지는 경우"의 해석입니다. 셸 설정 파일이 앞에 붙이는 폴더는 순서를 확인할 수 없습니다.

## 남은 위험 / 한계

- 셸 alias·함수·builtin, 순서 모르는 폴더 목록 밖의 폴더에 있는 다른 `stellar`는 감지하지 않습니다.
- dangling 소유 링크는 모양만으로 소유로 봅니다(동의 화면에 이전 대상 표시).
- 실제 helper가 없어 실제 앱 번들 기준 설치는 하지 않았습니다(`running app command status=helperUnavailable`).

## 재개 조건

- XXIV.5.3이 `[x]`가 됩니다(XXIV.5.1 선행).
- helper가 든 빌드에서 실제 사용자 홈 대신 임시 홈으로 설치·제거 fixture를 다시 실행한 뒤 `Task(XXIV.5.4)`로 표기를 올립니다.
