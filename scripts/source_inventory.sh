#!/bin/sh
#
# source_inventory.sh
#   추적 파일의 확장자별 수와 자동 생성물·vendored code를 결정론적으로 집계해
#   docs/status/source-inventory.md 를 생성합니다.
#
#   - 입력은 `git ls-files`(추적 파일 집합)뿐입니다.
#   - 출력에 커밋 해시·시각·절대 경로를 포함하지 않습니다.
#   - 따라서 동일 파일 집합에서 두 번 실행하면 동일한 보고서가 생성됩니다.
#
set -eu

cd "$(git rev-parse --show-toplevel)"

OUT="docs/status/source-inventory.md"

# 자동 생성물 경로 (AGENTS.md의 source-of-truth 정의 기준)
GEN_RE='^(README\.md|resources/readme-hero\.svg)$'
# vendored code 경로 패턴
VEN_RE='^(vendor|third_party|third-party|vendored)/|/vendor/|/third_party/'

files_file=$(mktemp)
ext_table=$(mktemp)
trap 'rm -f "$files_file" "$ext_table"' EXIT

git ls-files | LC_ALL=C sort > "$files_file"

total=$(grep -c '' "$files_file" || true)
gen=$(grep -cE "$GEN_RE" "$files_file" || true)
ven=$(grep -cE "$VEN_RE" "$files_file" || true)
gen_list=$(grep -E "$GEN_RE" "$files_file" || true)
ven_list=$(grep -E "$VEN_RE" "$files_file" || true)

# 확장자별 집계 (awk로 분류, LC_ALL=C sort로 순서 고정)
awk '
{
  n = split($0, parts, "/"); base = parts[n];
  if (base ~ /^\./) {
    rest = substr(base, 2);
    if (rest ~ /\./) { k = split(base, d, "."); ext = d[k]; }
    else { ext = "(dotfile)"; }
  } else if (base ~ /\./) {
    k = split(base, d, "."); ext = d[k];
  } else { ext = "(no-ext)"; }
  count[ext]++;
}
END { for (e in count) printf "%s\t%d\n", e, count[e]; }
' "$files_file" | LC_ALL=C sort > "$ext_table"

{
  echo "# Source Inventory"
  echo
  echo "<!-- 이 파일은 scripts/source_inventory.sh 가 생성합니다. 직접 수정하지 마세요. -->"
  echo "<!-- 내용은 추적 파일 집합에서만 도출되며 커밋 해시·시각을 포함하지 않습니다. -->"
  echo
  echo "## 요약"
  echo
  echo "- 추적 파일 수: ${total}"
  echo "- 자동 생성물: ${gen}"
  echo "- vendored code: ${ven}"
  echo "- 순수 소스/문서: $((total - gen - ven))"
  echo
  echo "## 확장자별 파일 수"
  echo
  echo "| 확장자 | 파일 수 |"
  echo "| --- | --- |"
  while IFS="$(printf '\t')" read -r e c; do
    [ -n "$e" ] || continue
    echo "| \`${e}\` | ${c} |"
  done < "$ext_table"
  echo
  echo "## 자동 생성물 (별도 집계)"
  echo
  if [ "${gen}" -eq 0 ]; then
    echo "- (없음)"
  else
    printf '%s\n' "$gen_list" | while IFS= read -r p; do [ -n "$p" ] && echo "- \`${p}\`"; done
  fi
  echo
  echo "## vendored code (별도 집계)"
  echo
  if [ "${ven}" -eq 0 ]; then
    echo "- (없음)"
  else
    printf '%s\n' "$ven_list" | while IFS= read -r p; do [ -n "$p" ] && echo "- \`${p}\`"; done
  fi
} > "$OUT"
