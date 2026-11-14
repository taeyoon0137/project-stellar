#!/usr/bin/env bash
# 실사용 QA 실행(docs/qa/README.md).
#
# StellarMac 앱을 Debug로 빌드하고(smoke.sh와 같은 xcodebuild 설정), 저장소 밖 임시 폴더에 샘플 프로젝트를 만든 뒤
# 그 프로젝트로 앱을 엽니다.
#
# 사용법:
#   ./scripts/macos/qa.sh [옵션]
#
# 옵션:
#   --fresh        샘플 프로젝트를 지우고 다시 만듭니다(지우지 않으면 있던 샘플을 그대로 엽니다).
#   --no-build     앱을 다시 빌드하지 않고 마지막 Debug 빌드를 씁니다.
#   --no-launch    빌드와 샘플 준비까지만 하고 앱을 띄우지 않습니다.
#   --welcome      샘플 없이 앱만 띄웁니다(시작 창 확인용).
#   --name <이름>  샘플 폴더 이름(기본 sample). 영문·숫자·`.`·`_`·`-`만 쓰고, 첫 글자는 영문·숫자·`_`입니다.
#   -h, --help     이 도움말을 보여 줍니다.
#
# 샘플 위치: ${TMPDIR}/stellar-qa/<이름> (TMPDIR가 없으면 /tmp). 저장소 안에는 아무것도 만들지 않습니다.
# 샘플 내용: 페이지 2개(Home: Desktop·Mobile View, Components: Buttons View), Frame·Fragment·사각형·타원·선·다각형·
#           path·텍스트·이미지, modifier. 생성은 crates/project/examples/qa_sample_project.rs(코어 생성 API)가 합니다.
# 정리: rm -rf "${TMPDIR%/}/stellar-qa"
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

usage() {
  sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed '$d' | sed 's/^# \{0,1\}//'
}

FRESH=0
BUILD=1
LAUNCH=1
WELCOME=0
NAME="sample"
while [ $# -gt 0 ]; do
  case "$1" in
    --fresh) FRESH=1 ;;
    --no-build) BUILD=0 ;;
    --no-launch) LAUNCH=0 ;;
    --welcome) WELCOME=1 ;;
    --name)
      if [ $# -lt 2 ]; then
        echo "qa: --name에 이름이 필요합니다." >&2
        exit 64
      fi
      NAME="$2"
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "qa: 알 수 없는 옵션: $1 (도움말: --help)" >&2
      exit 64
      ;;
  esac
  shift
done

# 지우기(--fresh) 대상이 임시 폴더 밖으로 나가지 않도록 이름을 제한합니다. 생성기가 `-`로 시작하는 인자를
# 옵션으로 읽으므로 첫 글자에 `.`과 `-`를 받지 않습니다.
if ! printf '%s' "$NAME" | grep -Eq '^[A-Za-z0-9_][A-Za-z0-9._-]*$'; then
  echo "qa: 샘플 이름은 영문·숫자·.·_·-만 쓰고 .이나 -로 시작하지 않아야 합니다: $NAME" >&2
  exit 64
fi

# 로컬에서 xcode-select가 CommandLineTools를 가리키면 Xcode를 직접 지정합니다(smoke.sh와 같음).
if ! xcodebuild -version >/dev/null 2>&1; then
  if [ -d /Applications/Xcode-beta.app ]; then
    export DEVELOPER_DIR=/Applications/Xcode-beta.app/Contents/Developer
  elif [ -d /Applications/Xcode.app ]; then
    export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
  fi
fi

DERIVED="$REPO_ROOT/apps/macos/build"
APP_BUNDLE="$DERIVED/Build/Products/Debug/StellarMac.app"
APP_BINARY="$APP_BUNDLE/Contents/MacOS/StellarMac"

# 1. 앱 Debug 빌드(smoke.sh와 같은 설정).
if [ "$BUILD" -eq 1 ]; then
  echo "== 앱 빌드(Debug)"
  xcodebuild \
    -project "$REPO_ROOT/apps/macos/StellarMac.xcodeproj" \
    -scheme StellarMac \
    -configuration Debug \
    -derivedDataPath "$DERIVED" \
    -packageAuthorizationProvider netrc \
    -disableAutomaticPackageResolution \
    CODE_SIGNING_ALLOWED=NO \
    -quiet \
    build
fi
if [ ! -x "$APP_BINARY" ]; then
  echo "qa: 앱 빌드 결과가 없습니다: $APP_BUNDLE (--no-build 없이 다시 실행하세요)" >&2
  exit 1
fi

# 2. 샘플 프로젝트 준비.
TMP_BASE="${TMPDIR:-/tmp}"
QA_ROOT="${TMP_BASE%/}/stellar-qa"
SAMPLE="$QA_ROOT/$NAME"
if [ "$WELCOME" -eq 0 ]; then
  mkdir -p "$QA_ROOT"
  if [ "$FRESH" -eq 1 ] && [ -e "$SAMPLE" ]; then
    echo "== 샘플 다시 만들기: 기존 $SAMPLE 삭제"
    rm -rf "$SAMPLE"
  fi
  if [ -e "$SAMPLE" ]; then
    echo "== 기존 샘플 사용: $SAMPLE (다시 만들려면 --fresh)"
  else
    echo "== 샘플 생성: $SAMPLE"
    cargo run -q --manifest-path "$REPO_ROOT/Cargo.toml" -p stellar-project --example qa_sample_project -- \
      "$QA_ROOT" "$NAME" --image "$REPO_ROOT/assets/docs/hero.jpg"
  fi
  # 생성기는 끝까지 성공한 샘플만 최종 이름으로 남깁니다. 그 전 버전이 남긴 반쯤 만든 폴더도 거르도록
  # 저장한 문서 generation(design/current)까지 봅니다.
  if [ ! -f "$SAMPLE/stellar.yaml" ] || [ ! -e "$SAMPLE/.git" ] || [ ! -e "$SAMPLE/design/current" ]; then
    echo "qa: 샘플이 완성된 Stellar 프로젝트가 아닙니다(stellar.yaml·.git·design/current 중 없음): $SAMPLE (--fresh로 다시 만드세요)" >&2
    exit 1
  fi
fi

echo "== 앱: $APP_BUNDLE"
if [ "$LAUNCH" -eq 0 ]; then
  echo "== --no-launch: 앱을 띄우지 않습니다."
  exit 0
fi

# 3. 앱 실행. 이미 실행 중인 같은 빌드가 있으면 새 빌드가 반영되지 않으므로 먼저 종료하게 합니다.
if pgrep -f "$APP_BINARY" >/dev/null 2>&1; then
  echo "qa: StellarMac이 이미 실행 중입니다. 앱을 종료(⌘Q)한 뒤 다시 실행하세요." >&2
  exit 1
fi
if [ "$WELCOME" -eq 1 ]; then
  echo "== 앱 실행(시작 창)"
  open "$APP_BUNDLE"
else
  echo "== 앱 실행: $SAMPLE"
  open -a "$APP_BUNDLE" "$SAMPLE"
fi
echo "== 확인 목록: docs/qa/README.md"
