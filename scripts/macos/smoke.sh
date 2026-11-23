#!/usr/bin/env bash
# macOS 셸 smoke/lifecycle/update test (II.5.2, II.5.3, XVI.9.5).
# UI는 재설계 전까지 빈 창 하나만 있는 껍데기입니다(ADR-0098). Stellar 앱을 Debug로 빌드한 뒤:
#  - --smoke: FFI로 코어 버전을 읽고 종료 코드 0
#  - --lifecycle-check: runtime create/destroy가 1:1인지 확인하고 종료 코드 0
#  - --command-install-check: `stellar` 명령 설치기 위치·설치·제거·상태 fixture(XXIV.5.3~XXIV.5.5)
#  - --update-check: 업데이트 구성·채널 격리·appcast 서명·승인 gate(Sparkle, loopback fixture)
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# 로컬에서 xcode-select가 CommandLineTools를 가리키면 Xcode를 직접 지정합니다.
# CI 러너처럼 이미 Xcode가 선택된 환경에서는 DEVELOPER_DIR를 덮어쓰지 않습니다.
if ! xcodebuild -version >/dev/null 2>&1; then
  if [ -d /Applications/Xcode-beta.app ]; then
    export DEVELOPER_DIR=/Applications/Xcode-beta.app/Contents/Developer
  elif [ -d /Applications/Xcode.app ]; then
    export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
  fi
fi

DERIVED="$REPO_ROOT/apps/macos/build"

# Swift package(Sparkle, ADR-0017)는 Package.resolved 고정 버전만 쓰고 자격 증명 조회는 netrc로 합니다
# (공개 package라 인증 불필요, 키체인 접근 대화상자 방지).
xcodebuild \
  -project "$REPO_ROOT/apps/macos/Stellar.xcodeproj" \
  -scheme Stellar \
  -configuration Debug \
  -derivedDataPath "$DERIVED" \
  -packageAuthorizationProvider netrc \
  -disableAutomaticPackageResolution \
  CODE_SIGNING_ALLOWED=NO \
  build

APP="$DERIVED/Build/Products/Debug/Stellar.app/Contents/MacOS/Stellar"

OUTPUT="$("$APP" --smoke)"
echo "$OUTPUT"
if ! printf '%s\n' "$OUTPUT" | grep -q "stellar core version:"; then
  echo "smoke 실패: 버전 문자열을 읽지 못했습니다." >&2
  exit 1
fi

# lifecycle: create/destroy 1:1 (set -e가 비0 종료를 실패로 처리).
"$APP" --lifecycle-check

# update: Info.plist 업데이트 구성, Stable/Beta appcast 채널 격리, appcast 서명 검증, 승인 전 다운로드 금지
#         (XVI.9.5, ADR-0017; 실제 Sparkle updater와 127.0.0.1 fixture 서버, 외부 네트워크 없음).
"$APP" --update-check

# 명령 설치기(XXIV.5.3~XXIV.5.5): 메모리 파일 시스템과 임시 폴더 fixture만 씁니다.
"$APP" --command-install-check

echo "smoke OK"
