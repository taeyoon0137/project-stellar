#!/usr/bin/env bash
# Developer ID로 서명된 Stellar.app의 공증(notarization)과 ticket stapling, Gatekeeper 검사(XVI.9.4).
#
# 선행: scripts/macos/sign_release.sh (Developer ID 서명)
#
# 공증 자격 증명은 저장소·스크립트에 두지 않습니다. 사용자가 한 번 직접 실행해
# 로그인 키체인에 profile을 만듭니다(비밀번호·키는 사용자가 입력):
#   xcrun notarytool store-credentials "stellar-notary" --apple-id <Apple ID> --team-id <Team ID>
# 다른 이름을 쓰면 STELLAR_NOTARY_PROFILE로 지정합니다.
#
# 순서: Developer ID 서명 확인 → 제출·대기 →
#       status Accepted·로그 issues 없음 확인 → stapling·validate → 서명 재검증·번들 검사 →
#       Gatekeeper 평가(spctl: Notarized Developer ID) → 다운로드 상태 모사(quarantine 속성 사본)에서
#       spctl과 syspolicy_check distribution
#
# 산출물: apps/macos/build/notarization/ 아래 제출 결과·공증 로그 JSON (build/는 git ignore)
# 종료 코드: 0 성공, 66 앱 없음, 그 외 공증·검증 실패.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PROFILE="${STELLAR_NOTARY_PROFILE:-stellar-notary}"
APP="$REPO_ROOT/apps/macos/build/Build/Products/Release/Stellar.app"
EVIDENCE="$REPO_ROOT/apps/macos/build/notarization"

if [ ! -d "$APP" ]; then
  echo "✖ 서명된 앱이 없습니다. 먼저 scripts/macos/sign_release.sh 를 실행하세요." >&2
  exit 66
fi

fail() {
  echo "✖ 공증 실패: $*" >&2
  exit 1
}

# Developer ID 서명인지 확인(Apple Development 서명은 공증 불가).
SIGNATURE_INFO="$(codesign -dvv "$APP" 2>&1)"
if ! printf '%s\n' "$SIGNATURE_INFO" | grep -q "Authority=Developer ID Application"; then
  fail "Developer ID Application 서명이 아닙니다. sign_release.sh 를 배포 모드로 실행하세요."
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
ZIP="$WORK/Stellar.zip"
ditto -c -k --keepParent "$APP" "$ZIP"

mkdir -p "$EVIDENCE"
SUBMISSION="$EVIDENCE/submission.json"
xcrun notarytool submit "$ZIP" --keychain-profile "$PROFILE" --wait --output-format json > "$SUBMISSION"
SUBMISSION_ID="$(plutil -extract id raw -o - "$SUBMISSION")"
SUBMISSION_STATUS="$(plutil -extract status raw -o - "$SUBMISSION")"
echo "공증 제출: id=$SUBMISSION_ID status=$SUBMISSION_STATUS"

# 공증 로그: Accepted여도 경고(issues)가 있으면 실패로 처리합니다.
LOG="$EVIDENCE/notarization-log.json"
xcrun notarytool log "$SUBMISSION_ID" --keychain-profile "$PROFILE" "$LOG"
[ "$SUBMISSION_STATUS" = "Accepted" ] || fail "공증 status=$SUBMISSION_STATUS (로그: $LOG)"
grep -Eq '"issues"[[:space:]]*:[[:space:]]*null' "$LOG" || fail "공증 로그에 issues가 있습니다: $LOG"

xcrun stapler staple "$APP"
xcrun stapler validate "$APP"

# stapling 후에도 서명이 유효하고 번들 구조(stapled ticket 포함)가 허용 범위인지 확인합니다.
codesign --verify --deep --strict --verbose=2 "$APP"
"$REPO_ROOT/scripts/macos/bundle_check.sh" "$APP"

# Gatekeeper 평가(공증 ticket 포함 확인).
SPCTL="$(spctl --assess --type execute --verbose=2 "$APP" 2>&1)"
printf '%s\n' "$SPCTL"
printf '%s\n' "$SPCTL" | grep -q "source=Notarized Developer ID" || fail "Gatekeeper source가 Notarized Developer ID가 아님"

# 다운로드 상태 모사: quarantine 속성을 붙인 사본을 Gatekeeper 평가와 배포 점검에 통과시킵니다.
QUARANTINED="$WORK/quarantine/Stellar.app"
mkdir -p "$WORK/quarantine"
ditto "$APP" "$QUARANTINED"
xattr -w com.apple.quarantine "0083;$(printf '%x' "$(date +%s)");stellar-notarize-check;" "$QUARANTINED"
xattr -p com.apple.quarantine "$QUARANTINED" >/dev/null
QUARANTINE_SPCTL="$(spctl --assess --type execute --verbose=2 "$QUARANTINED" 2>&1)"
printf '%s\n' "$QUARANTINE_SPCTL" | grep -q "source=Notarized Developer ID" \
  || fail "quarantine 사본 Gatekeeper 평가 실패: $QUARANTINE_SPCTL"
syspolicy_check distribution "$QUARANTINED"

echo "✓ 공증·stapling·Gatekeeper 검사 완료: $APP"
