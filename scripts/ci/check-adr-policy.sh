#!/usr/bin/env bash
#
# check-adr-policy.sh (I.4.4)
#   변경된 경로 집합에 ADR 보호 경로 정책(`cargo xtask validate-adr`)을 적용합니다.
#
#   범위 결정:
#     - PR: origin/<base>..HEAD
#     - BASELINE_REF 설정 시: <BASELINE_REF>..HEAD
#     - 그 외: 직전 commit 1개
#
set -euo pipefail

if [ -n "${GITHUB_BASE_REF:-}" ]; then
  git fetch --no-tags --depth=200 origin "$GITHUB_BASE_REF" >/dev/null 2>&1 || true
  base="origin/${GITHUB_BASE_REF}"
elif [ -n "${BASELINE_REF:-}" ]; then
  base="${BASELINE_REF}"
else
  base="HEAD~1"
fi

paths=()
while IFS= read -r path; do
  [ -n "$path" ] && paths+=("$path")
done < <(git diff --name-only "${base}..HEAD")

if [ "${#paths[@]}" -eq 0 ]; then
  echo "✓ 변경 경로 없음 — ADR 정책 통과"
  exit 0
fi

cargo xtask validate-adr "${paths[@]}"
