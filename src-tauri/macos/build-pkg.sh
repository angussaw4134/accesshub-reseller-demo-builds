#!/bin/sh
# Package an already signed universal app without changing its contents.
set -eu

if [ "$#" -ne 2 ] || [ "$(/usr/bin/uname -s)" != Darwin ]; then
  printf '%s\n' 'Usage on macOS: build-pkg.sh /path/AccessHub Reseller.app /path/AccessHub-Setup.pkg' >&2
  exit 1
fi
app_path=${1%/}
output_pkg=$2
case "$app_path" in /*) ;; *) app_path="$(pwd)/$app_path" ;; esac
case "$output_pkg" in /*) ;; *) output_pkg="$(pwd)/$output_pkg" ;; esac
if [ ! -d "$app_path" ] || [ "${app_path##*/}" != "AccessHub Reseller.app" ]; then
  printf '%s\n' 'The input must be the built AccessHub Reseller.app bundle.' >&2
  exit 1
fi
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
app_version=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app_path/Contents/Info.plist")
case "$app_version" in
  *-beta.*) package_version="${app_version%-beta.*}.${app_version##*-beta.}" ;;
  *) package_version=$app_version ;;
esac
case "$package_version" in ''|*[!0-9.]*) printf '%s\n' 'The package version must be numeric.' >&2; exit 1 ;; esac

/usr/bin/codesign --verify --deep --strict --all-architectures "$app_path"
stage=$(/usr/bin/mktemp -d "${TMPDIR:-/tmp}/accesshub-pkg.XXXXXX")
trap 'if [ -n "${stage:-}" ]; then /bin/rm -rf "$stage"; fi' 0
trap 'exit 1' HUP INT TERM
/bin/mkdir -p "$stage/payload/Applications" "$stage/scripts" "$(dirname -- "$output_pkg")"
/usr/bin/ditto --rsrc --extattr "$app_path" "$stage/payload/Applications/AccessHub Reseller.app"
/usr/bin/install -m 0755 "$script_dir/postinstall" "$stage/scripts/postinstall"
/usr/bin/codesign --verify --deep --strict --all-architectures "$stage/payload/Applications/AccessHub Reseller.app"

# Fix the destination at /Applications, rather than relocating to a found copy.
cat > "$stage/components.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><array><dict>
  <key>RootRelativeBundlePath</key><string>Applications/AccessHub Reseller.app</string>
  <key>BundleIsRelocatable</key><false/>
  <key>BundleHasStrictIdentifier</key><true/>
  <key>BundleIsVersionChecked</key><false/>
  <key>BundleOverwriteAction</key><string>upgrade</string>
</dict></array></plist>
PLIST

/usr/bin/pkgbuild \
  --root "$stage/payload" \
  --component-plist "$stage/components.plist" \
  --install-location / \
  --identifier app.accesshub.resellerloader.pkg \
  --version "$package_version" \
  --ownership recommended \
  --scripts "$stage/scripts" \
  "$output_pkg"

# Packaging must preserve the original app's signature.
/usr/bin/codesign --verify --deep --strict --all-architectures "$app_path"
