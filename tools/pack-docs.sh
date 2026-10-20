#!/bin/bash
# Packs the results of the last tools/docs.sh run for GitHub Pages: site/results/ (the test results)
# and site/screenshots/ (the screenshots the manual shows, shrunk to 960 px wide). Commit both; the Pages
# workflow can't run the tests, so it builds the manual from these. Run after tools/docs.sh.
set -e
cd "$(dirname "$0")/.."
rm -rf site/results site/screenshots
cp -r docs/results site/results
# only what the generated pages show: the old runs' leftovers stay behind
rg -oI 'screenshots/[^"]*\.(jpg|svg)' docs/site/*.html | sort -u | while read -r f; do
  [ -n "$f" ] || continue
  mkdir -p "site/$(dirname "$f")"
  case "$f" in
    *.jpg) sips -s format jpeg -s formatOptions 60 --resampleWidth 960 "docs/$f" --out "site/$f" >/dev/null </dev/null ;;
    *) cp "docs/$f" "site/$f" ;;
  esac
done
du -sh site/results site/screenshots
