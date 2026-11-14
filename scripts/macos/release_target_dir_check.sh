#!/usr/bin/env bash
# release_build.sh의 Rust target 디렉터리 회귀 검사(XVI.9.1 후속).
#
# Xcode Rust 코어 phase의 cargo는 CARGO_TARGET_DIR(또는 cargo 설정의 build.target-dir)에 staticlib을 만듭니다. 이 검사는
# 비어 있는 임시 CARGO_TARGET_DIR로 scripts/macos/release_build.sh 전체(Release 빌드·구성·번들 검사)를 실행하고, 실행
# 파일 링크(-L)와 아키텍처 검사가 그 디렉터리의 staticlib을 썼는지 확인합니다. 저장소 target/에 남은 다른(오래된)
# staticlib이 링크되거나, 그곳에 staticlib이 없어 링크가 실패하면 이 검사도 실패합니다.
#
# 사용법: scripts/macos/release_target_dir_check.sh
# 종료 코드: 0 성공, 64 사용법 오류, 그 외 실패.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

if [ "$#" -ne 0 ]; then
  echo "사용법: $0" >&2
  exit 64
fi

fail() {
  echo "✖ release target 디렉터리 검사 실패: $*" >&2
  exit 1
}

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
TARGET_DIR="$WORK/target"
CORE_LIB_DIR="$TARGET_DIR/aarch64-apple-darwin/release"
LOG="$WORK/release_build.log"

if ! CARGO_TARGET_DIR="$TARGET_DIR" "$REPO_ROOT/scripts/macos/release_build.sh" >"$LOG" 2>&1; then
  tail -n 40 "$LOG" >&2
  fail "CARGO_TARGET_DIR=$TARGET_DIR 에서 release_build.sh 실패"
fi

# xcodebuild 로그의 링크 명령에 임시 target의 staticlib 디렉터리가 검색 경로로 들어가야 합니다.
grep -Fq -- "-L$CORE_LIB_DIR" "$LOG" || fail "실행 파일 링크 명령에 -L$CORE_LIB_DIR 없음(다른 staticlib 링크)"
# release_build.sh의 아키텍처 검사도 같은 staticlib을 봐야 합니다.
grep -Fqx "✓ Rust 코어 staticlib: $CORE_LIB_DIR/libstellar_ffi.a" "$LOG" \
  || fail "release_build.sh가 $CORE_LIB_DIR/libstellar_ffi.a를 검사하지 않음"
grep -Fq "✓ Release 구성 검사 통과" "$LOG" || fail "release_build.sh 완료 줄 없음"
grep '^✓' "$LOG" || true
echo "✓ release target 디렉터리: CARGO_TARGET_DIR의 staticlib을 링크·검사, Release 구성 검사 통과"
