# 작업 보고서: XVI.9.4 공증 제출과 ticket stapling pipeline

- WBS: XVI.9.4
- 작업명: notarization submission과 ticket stapling pipeline 구성
- 상태: 부분 완료 (`[~]`) — 공증·stapling·Gatekeeper 평가를 실제로 통과, "clean machine" Gatekeeper 검증은 미확보
- 선행: XVI.9.3 (`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (키체인 공증 profile `stellar-notary`로 Apple notary service 실제 제출). 개발 도구·인증서가 없는 깨끗한 macOS
  장비는 사용하지 못했습니다.

## 변경 파일 목록

- `scripts/macos/notarize.sh` — 제출 결과 JSON·공증 로그 저장, status `Accepted`·로그 `issues: null` 확인, stapling 후 서명
  재검증과 `bundle_check.sh`, `spctl` source 확인, quarantine 속성 사본의 `spctl`·`syspolicy_check distribution`.
- `scripts/macos/bundle_check.sh` — stapled 공증 ticket(`Contents/CodeResources`)을 allowlist에 추가.
- `docs/task-reports/XVI.9.4.md`(신규), 계획서(XVI.9.4 `[ ]`→`[~]`).

## 구현 요약

- **pipeline**: Developer ID 서명 확인 → zip(`ditto -c -k --keepParent`) → `notarytool submit --wait --output-format json`
  (`apps/macos/build/notarization/submission.json`) → `notarytool log`(`notarization-log.json`) → status `Accepted`와
  `issues: null`이 아니면 실패 → `stapler staple`·`stapler validate` → `codesign --verify --deep --strict` →
  `bundle_check.sh`(stapled 번들 구조·서명·재배치 실행) → `spctl --assess --type execute`가 `source=Notarized Developer ID`인지 →
  quarantine 속성(`com.apple.quarantine`)을 붙인 사본으로 다운로드 상태를 모사해 `spctl`과 `syspolicy_check distribution` 통과 확인.
- **자격 증명**: notarytool 키체인 profile만 사용하고 저장소·스크립트·로그 출력에 Apple ID·암호·team을 기록하지 않습니다.
- 제출 전 `syspolicy_check notary-submission` 점검은 아직 공증되지 않은 앱을 Gatekeeper 거부로 fatal 처리해 제출 전 단계로
  쓸 수 없어 넣지 않았습니다(실행 결과로 확인).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `scripts/macos/sign_release.sh` (XVI.9.3 산출물, 서명본 실행 파일 SHA-256 `2e18cdd7…4032b1c`) | 종료 0 |
| `scripts/macos/notarize.sh` (1차, `syspolicy_check notary-submission` 포함) | 종료 70: 미공증 앱을 "Gatekeeper rejected" fatal로 판정 → 사전 점검 제거 |
| `scripts/macos/notarize.sh` (최종) | 종료 0, `✓ 공증·stapling·Gatekeeper 검사 완료` |
| `xcrun notarytool submit ... --wait` | submission id `f2d9c2f9-cbb5-41fa-8f2f-e052416d9dca`, status `Accepted` |
| `xcrun notarytool log f2d9c2f9-…` | `"statusSummary": "Ready for distribution"`, `"statusCode": 0`, `"issues": null` |
| `xcrun stapler staple` / `xcrun stapler validate` | `The staple and validate action worked!` / `The validate action worked!` |
| stapling 후 `codesign --verify --deep --strict` + `bundle_check.sh` | `valid on disk`, 번들 검사 7단계 ✓(stapled `Contents/CodeResources` 포함) |
| `spctl --assess --type execute --verbose=2 StellarMac.app` | `accepted`, `source=Notarized Developer ID` |
| quarantine 속성 사본 `spctl --assess` / `syspolicy_check distribution` | `source=Notarized Developer ID` / `App passed all pre-distribution checks and is ready for distribution.` |
| `bash -n scripts/macos/notarize.sh` / `scripts/macos/bundle_check.sh` | 통과 |
| `git diff --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 공증 제출·승인, 로그 issues 없음 | 통과 |
| ticket stapling·offline validate | 통과 |
| Gatekeeper 평가(이 장비, 다운로드 상태 모사 포함) | 통과 |
| **Gatekeeper clean machine 검증** | 미실행(깨끗한 장비 없음) |

## 차이(작업 지시와 다른 점)

- 공증 스크립트(`notarize.sh`)는 2026-11-14 `Chore:` 커밋으로 이미 있었고, 이번 작업에서 결과 판정(status·issues), 증거
  저장, stapling 후 재검증, 다운로드 상태 모사 검사를 추가했습니다.
- "clean machine"은 같은 장비의 quarantine 사본 평가로 근사했습니다. 계획 1.3 5번 규칙에 따라 `[~]`입니다.

## 남은 위험 / 한계

- 깨끗한 macOS 26 장비(또는 VM)에서 공증본을 인터넷 다운로드 경로(quarantine 실제 부여)로 받아 Finder로 첫 실행하는
  Gatekeeper 확인이 필요합니다. 첫 실행 확인 대화상자는 사람의 클릭이 필요합니다.
- 배포 형식(zip/dmg)과 배포 위치는 정하지 않았습니다. dmg로 배포하면 dmg 자체 서명·공증·stapling 단계가 추가로 필요합니다.
- 공증은 Apple 서비스 가용성과 네트워크에 의존합니다.

## 재평가(2026-11-14, Sparkle 내장 반영)

- **상태**: `[~]`를 유지합니다. 이유는 두 가지입니다.
  - 엄격 선행 XVI.9.3이 `[~]`입니다(그 선행 XVI.9.2가 깨끗한 장비 실행 미확보).
  - "Gatekeeper clean machine 검증"이 미확보입니다.
- **번들 변경**: 리뷰 수정 커밋 `5b993ac`로 번들에 Sparkle.framework가 들어갔습니다. 위 공증 증거
  (submission `f2d9c2f9-…`)는 Sparkle이 없던 번들에 대한 것입니다. 현재 번들은 다시 공증해야 합니다.
- **이번에 하지 않은 것**: 공증 재제출은 하지 않았습니다. 사용자 Apple 계정으로 외부 서비스에 binary를 올리는 일이라,
  통합 리뷰(P6)가 사용자 확인을 요구했기 때문입니다.
- **stapled 번들 검사**: `notarize.sh`는 바꾸지 않았습니다. stapling 후 실행하는 `bundle_check.sh`는 이제
  Sparkle.framework 구조와 중첩 코드 team 일치를 함께 검사합니다(XVI.9.2 보고서 참조).
- **재공증 절차**: Developer ID 서명(`sign_release.sh`, 이번에 통과) 뒤 사용자가 `scripts/macos/notarize.sh`를
  실행합니다. 중첩 코드가 모두 같은 Developer ID·Hardened Runtime·secure timestamp로 서명되어 있어야 공증을 통과합니다.
