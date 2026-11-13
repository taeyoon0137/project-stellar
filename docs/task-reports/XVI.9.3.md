# 작업 보고서: XVI.9.3 Developer ID 서명 pipeline

- WBS: XVI.9.3
- 작업명: Developer ID signing pipeline 구성 — 로컬 개발 key를 저장소에 저장하지 않음
- 상태: 완료 (`[x]`)
- 선행: XVI.9.2 (`[~]`, 이 lane — 필요한 macOS 부분인 Release 번들·구조 검사는 존재, 미확보는 깨끗한 장비 실행뿐)

## 필수 실행 환경

- `DEV-MAC` (로그인 키체인의 Developer ID Application identity로 실제 서명, Apple timestamp 서버 사용).

## 변경 파일 목록

- `scripts/macos/sign_release.sh` — 빌드를 `release_build.sh`(XVI.9.1·XVI.9.2 검사 포함)로 위임, 저장소 추적 파일의 서명 자료
  검사 추가, Developer ID 인증서 체인·Apple 보안 timestamp·TeamIdentifier 확인, 서명본에 `bundle_check.sh` 적용.
- `docs/task-reports/XVI.9.3.md`(신규), 계획서(XVI.9.3 `[ ]`→`[x]`).

## 구현 요약

- **pipeline 순서**: 저장소 서명 자료 부재 확인 → `release_build.sh`(서명 없는 Release 빌드, arm64·macOS 26·번들 ID,
  ad-hoc 사본 번들 검사) → `codesign --force --options runtime --timestamp --sign <identity>` →
  `codesign --verify --deep --strict` → Developer ID 체인·timestamp·team 확인 → `bundle_check.sh`(서명본 그대로:
  구조·링크·rpath·서명 구조·재배치 실행) → `--smoke`.
- **key 비저장**: identity는 `STELLAR_CODESIGN_IDENTITY` 또는 로그인 키체인에서만 찾고(기존 동작), 저장소에는 identity·
  team·인증서를 기록하지 않습니다. 추적 파일에 `.p12/.pfx/.cer/.crt/.der/.pem/.key/.p8/.keychain(-db)/.mobileprovision/
  .provisionprofile`가 있으면 서명 전에 중단합니다.
- `--development`(Apple Development, 로컬 검증용) 모드는 timestamp 없이 같은 경로를 탑니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `scripts/macos/sign_release.sh` (Developer ID) | Release 빌드·구성 검사 ✓, 서명, deep strict verify, bundle 검사(서명본) ✓, `✓ 서명 완료(developer-id)`, 종료 0 |
| `codesign --verify --deep --strict --verbose=2 StellarMac.app` | `valid on disk`, `satisfies its Designated Requirement`(종료 0) |
| `codesign -dvv StellarMac.app`(team·이름 가림) | `Identifier=xyz.taeyoon.stellar`, `flags=0x10000(runtime)`, `Authority=Developer ID Application: <name> (<team>)` → `Developer ID Certification Authority` → `Apple Root CA`, `Timestamp=Nov 14, 2026 at 1:19:17 AM`, `Info.plist entries=19`, `Sealed Resources version=2` |
| `shasum -a 256 .../Contents/MacOS/StellarMac`(서명본) | `2e18cdd7a3fe3f39c7093e4a38d7f805a434086a21de7dd77caba0d4c4032b1c` |
| `git ls-files`에서 서명 자료 확장자 검색 | 0건 |
| `bash -n scripts/macos/sign_release.sh` | 통과 |
| `git diff --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **`codesign --verify --deep --strict` 통과**(Developer ID 서명본) | 통과 |
| Developer ID 체인·보안 timestamp·Hardened Runtime(공증 전제 조건) | 통과 |
| 서명본 재배치 실행(개발 경로 읽기 차단) `--smoke`/`--lifecycle-check`/`--canvas-check` | 통과 |
| 저장소에 서명 key·인증서·profile 없음 | 통과 |

## 차이(작업 지시와 다른 점)

- 서명 스크립트(`sign_release.sh`)는 2026-11-14 `Chore:` 커밋으로 이미 있었고, 이번 작업에서 Release 구성·번들 검사와
  Developer ID 체인 검사를 pipeline에 묶었습니다.
- 보고서에는 서명 identity 이름과 team 식별자를 가렸습니다(저장소 비기록 원칙).

## 남은 위험 / 한계

- CI(`CI-MAC`) 서명은 구성하지 않았습니다. CI에서 서명하려면 인증서를 secret으로 주입하는 별도 결정이 필요합니다.
- 인증서 만료·폐기 시 갱신 절차는 사람이 키체인에서 수행해야 합니다.
- 공증·stapling과 Gatekeeper 확인은 XVI.9.4 소관입니다.

## 상태 정정(통합 리뷰, 2026-11-14)

- `[x]` → `[~]`: 엄격 선행 XVI.9.2가 `[~]`입니다.
