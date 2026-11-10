#!/bin/sh
# CI-only installation smoke test. Tickets below are inert test data.
set -eu

if [ "$#" -ne 3 ] || [ "$(/usr/bin/uname -s)" != Darwin ]; then
  printf '%s\n' 'Usage on macOS: test-pkg.sh package.pkg AccessHub Reseller.app expected-version' >&2
  exit 1
fi
package=$1
source_app=$2
expected_version=$3
bootstrap='/Library/Application Support/AccessHub Reseller/activation-ticket'
installed_app='/Applications/AccessHub Reseller.app'
dummy_ticket='TESTONLY_0123456789-ABCDEFGHIJKLMNOPQRSTUVW'
test "${#dummy_ticket}" -eq 43
if [ -e "$bootstrap" ] || [ -L "$bootstrap" ]; then
  printf '%s\n' 'Refusing to overwrite an existing activation bootstrap during CI testing.' >&2
  exit 1
fi
work=$(/usr/bin/mktemp -d "${TMPDIR:-/tmp}/accesshub-pkg-test.XXXXXX")
cleanup() {
  if [ -f "$bootstrap" ] && [ ! -L "$bootstrap" ] && [ "$(/bin/cat "$bootstrap")" = "$dummy_ticket" ]; then
    sudo /bin/rm -f "$bootstrap"
  fi
  /bin/rm -rf "$work"
}
trap cleanup 0
trap 'exit 1' HUP INT TERM

code_hash() {
  /usr/bin/codesign --display --arch "$2" --verbose=4 "$1" 2>&1 | /usr/bin/sed -n 's/^CDHash=//p'
}
assert_ticket() {
  test "$(/bin/cat "$bootstrap")" = "$dummy_ticket"
  test "$(/usr/bin/stat -f '%z' "$bootstrap")" -eq 43
  test "$(/usr/bin/stat -f '%Su:%Sg:%Lp' "$bootstrap")" = 'root:wheel:644'
}

/usr/bin/codesign --verify --deep --strict --all-architectures "$source_app"
for arch in arm64 x86_64; do
  hash=$(code_hash "$source_app" "$arch")
  test -n "$hash"
  printf '%s' "$hash" > "$work/original-$arch.hash"
done
test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$source_app/Contents/Info.plist")" = "$expected_version"

# A manual filename must not create an activation bootstrap.
/bin/cp "$package" "$work/AccessHub-Setup.pkg"
sudo /usr/sbin/installer -pkg "$work/AccessHub-Setup.pkg" -target /
test ! -e "$bootstrap"

# The original filename and browser/Finder duplicate forms must be recognized.
for suffix in '' ' (1)' '(1)' ' 2'; do
  ticket_package="$work/AccessHub-Setup--$dummy_ticket$suffix.pkg"
  /bin/cp "$package" "$ticket_package"
  sudo /usr/sbin/installer -pkg "$ticket_package" -target /
  assert_ticket
  sudo /bin/rm -f "$bootstrap"
done

# A manual re-install also leaves an existing bootstrap unchanged.
sudo /usr/sbin/installer -pkg "$work/AccessHub-Setup--$dummy_ticket.pkg" -target /
assert_ticket
sudo /usr/sbin/installer -pkg "$work/AccessHub-Setup.pkg" -target /
assert_ticket

test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$installed_app/Contents/Info.plist")" = "$expected_version"
/usr/bin/codesign --verify --deep --strict --all-architectures "$installed_app"
/usr/bin/codesign --verify --deep --strict --all-architectures "$source_app"
for arch in arm64 x86_64; do
  test "$(code_hash "$source_app" "$arch")" = "$(/bin/cat "$work/original-$arch.hash")"
  test "$(code_hash "$installed_app" "$arch")" = "$(/bin/cat "$work/original-$arch.hash")"
done
printf '%s\n' 'Mac package installation, ticket filename handling, ownership, version, and app code seals passed.'
