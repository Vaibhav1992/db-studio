#!/bin/sh
# Release DB Studio.app. Signs with $APPLE_SIGNING_IDENTITY, else the first "Apple Development" identity
# in the login keychain, else ad-hoc. A stable signature lets macOS keep "Always Allow" for Keychain
# passwords across rebuilds; an ad-hoc build asks again after every rebuild.
set -e
if [ -z "$APPLE_SIGNING_IDENTITY" ]; then
  id=$(security find-identity -v -p codesigning | sed -n 's/.*"\(Apple Development[^"]*\)".*/\1/p' | head -1)
  if [ -n "$id" ]; then export APPLE_SIGNING_IDENTITY="$id"; fi
fi
exec pnpm tauri build --bundles app "$@"
