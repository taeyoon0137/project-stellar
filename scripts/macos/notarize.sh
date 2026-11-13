#!/usr/bin/env bash
# Developer ID로 서명된 StellarMac.app의 공증(notarization)과 ticket stapling.
#
# 선행: scripts/macos/sign_release.sh (Developer ID 서명)
#
# 공증 자격 증명은 저장소·스크립트에 두지 않습니다. 사용자가 한 번 직접 실행해
# 로그인 키체인에 profile을 만듭니다(비밀번호·키는 사용자가 입력):
#   xcrun notarytool store-credentials "stellar-notary" --apple-id <Apple ID> --team-id <Team ID>
# 다른 이름을 쓰면 STELLAR_NOTARY_PROFILE로 지정합니다.
#
# 종료 코드: 0 성공, 66 앱 없음, 그 외 공증·검증 실패.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PROFILE="${STELLAR_NOTARY_PROFILE:-stellar-notary}"
APP="$REPO_ROOT/apps/macos/build/Build/Products/Release/StellarMac.app"

if [ ! -d "$APP" ]; then
  echo "✖ 서명된 앱이 없습니다. 먼저 scripts/macos/sign_release.sh 를 실행하세요." >&2
  exit 66
fi

# Developer ID 서명인지 확인(Apple Development 서명은 공증 불가).
SIGNATURE_INFO="$(codesign -dvv "$APP" 2>&1)"
if ! printf '%s\n' "$SIGNATURE_INFO" | grep -q "Authority=Developer ID Application"; then
  echo "✖ Developer ID Application 서명이 아닙니다. sign_release.sh 를 배포 모드로 실행하세요." >&2
  exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
ZIP="$WORK/StellarMac.zip"
ditto -c -k --keepParent "$APP" "$ZIP"

xcrun notarytool submit "$ZIP" --keychain-profile "$PROFILE" --wait
xcrun stapler staple "$APP"
xcrun stapler validate "$APP"

# Gatekeeper 평가(공증 ticket 포함 확인).
spctl --assess --type execute --verbose=2 "$APP"

echo "✓ 공증·stapling 완료: $APP"
