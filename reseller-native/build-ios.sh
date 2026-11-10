#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p dist/Payload/AccessHubDemo.app
APP="$PWD/dist/Payload/AccessHubDemo.app"
SDK="$(xcrun --sdk iphoneos --show-sdk-path)"
xcrun --sdk iphoneos swiftc -parse-as-library -O -target arm64-apple-ios16.0 -sdk "$SDK" ios/App.swift -o "$APP/AccessHubDemo"
python3 - "$APP" <<'PY'
import pathlib,plistlib,sys
app=pathlib.Path(sys.argv[1])
info={'CFBundleExecutable':'AccessHubDemo','CFBundleIdentifier':'app.accesshub.resellerdemo','CFBundleName':'AccessHub Demo','CFBundleDisplayName':'AccessHub Demo','CFBundlePackageType':'APPL','CFBundleShortVersionString':'1.0.0','CFBundleVersion':'1','LSRequiresIPhoneOS':True,'MinimumOSVersion':'16.0','UIDeviceFamily':[1,2],'UIRequiredDeviceCapabilities':['arm64'],'UISupportedInterfaceOrientations':['UIInterfaceOrientationPortrait','UIInterfaceOrientationLandscapeLeft','UIInterfaceOrientationLandscapeRight'],'UILaunchScreen':{},'NSPhotoLibraryUsageDescription':'Choose a photo for your fictional demo profile.','NSCameraUsageDescription':'Take a photo for your fictional demo profile.'}
(app/'Info.plist').write_bytes(plistlib.dumps(info))
PY
xcrun lipo "$APP/AccessHubDemo" -verify_arch arm64
(cd dist && /usr/bin/zip -qry AccessHub-Reseller-Demo.ipa Payload)
shasum -a 256 dist/AccessHub-Reseller-Demo.ipa
