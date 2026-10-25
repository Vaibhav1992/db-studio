#!/usr/bin/env bash
# Build DB Studio on this Mac and publish it as a GitHub release on the personal account.
# Usage: ./release.sh <x.y.z>. No CI: everything runs locally. See "Release" in README.md.
set -euo pipefail

APP="DB Studio"
OWNER_REPO="Vaibhav1992/db-studio"
URL="git@github-personal:$OWNER_REPO.git"
export GH_CONFIG_DIR=~/.config/gh-personal

die() { echo "release: $*" >&2; exit 1; }
cd "$(dirname "$0")"
V="${1:-}"

# ---- checks: nothing is built or published until all pass ----
[[ "$V" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "version must be x.y.z (got '$V')"
login=$(gh api user --jq .login) || die "gh (GH_CONFIG_DIR=$GH_CONFIG_DIR) is not logged in"
[ "$login" = "Vaibhav1992" ] || die "gh user is '$login', not Vaibhav1992"
# origin is the work repo here; the personal remote may be named origin or personal.
REMOTE=""
for r in origin personal; do
  if [ "$(git remote get-url "$r" 2>/dev/null)" = "$URL" ] && [ "$(git remote get-url --push "$r")" = "$URL" ]; then REMOTE=$r; break; fi
done
[ -n "$REMOTE" ] || die "no remote (origin or personal) points to $URL"
[ "$(git branch --show-current)" = "main" ] || die "not on main"
[ -z "$(git status --porcelain)" ] || die "working tree is not clean"
git fetch -q "$REMOTE" main --tags
[ "$(git rev-parse HEAD)" = "$(git rev-parse "$REMOTE/main")" ] || die "HEAD is not equal to $REMOTE/main"
git rev-parse -q --verify "refs/tags/v$V" >/dev/null && die "tag v$V already exists"
[ -z "$(git ls-remote --tags "$REMOTE" "refs/tags/v$V")" ] || die "tag v$V already exists on $REMOTE"
pgrep -f "target/release/bundle/macos/$APP.app/" >/dev/null && die "quit the $APP built in target/ first (the build replaces it)"

# ---- build, version stamped in (tauri.conf.json is not edited, so the tree stays clean) ----
id=$(security find-identity -v -p codesigning | sed -n 's/^ *[0-9]*) [0-9A-F]* "\(.*\)"$/\1/p' | head -1)
export APPLE_SIGNING_IDENTITY="${id:--}" # "-" = ad hoc
echo "release: v$V, signing with: $APPLE_SIGNING_IDENTITY"
(cd app && pnpm install --frozen-lockfile && pnpm check && (cd src-tauri && cargo test -q) && sh scripts/build-app.sh --config "{\"version\":\"$V\"}")
BUNDLE="app/src-tauri/target/release/bundle/macos/$APP.app"

# ---- smoke test the built app ----
codesign --verify --deep --strict "$BUNDLE" || die "signature check failed"
got=$(/usr/libexec/PlistBuddy -c "Print CFBundleShortVersionString" "$BUNDLE/Contents/Info.plist")
[ "$got" = "$V" ] || die "built version is $got, expected $V"
"$BUNDLE/Contents/MacOS/app" >/dev/null 2>&1 & pid=$!
sleep 4
kill -0 "$pid" 2>/dev/null || die "the app exited at start"
kill "$pid"

# ---- package: <App>-<x.y.z>.dmg with an Applications link ----
mkdir -p dist
DMG="dist/${APP// /-}-$V.dmg"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
ditto "$BUNDLE" "$stage/$APP.app"
ln -s /Applications "$stage/Applications"
hdiutil create -quiet -volname "$APP $V" -srcfolder "$stage" -ov -format UDZO "$DMG"
hdiutil verify -quiet "$DMG" || die "dmg verify failed"
mnt=$(hdiutil attach -nobrowse -readonly "$DMG" | sed -n 's|.*\(/Volumes/.*\)$|\1|p')
codesign --verify --deep --strict "$mnt/$APP.app" || { hdiutil detach -quiet "$mnt"; die "app in dmg failed the signature check"; }
hdiutil detach -quiet "$mnt"

# ---- publish ----
git tag -a "v$V" -m "$APP $V"
git push "$REMOTE" "v$V"
gh release create "v$V" "$DMG" --repo "$OWNER_REPO" --title "$APP $V" --generate-notes
echo "release: done: $DMG ($(du -h "$DMG" | cut -f1))"
