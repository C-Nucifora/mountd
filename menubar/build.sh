#!/bin/zsh
# Build (and optionally install) the mountd menu bar app.
#   ./build.sh            build menubar/build/Mountd Menu Bar.app
#   ./build.sh install    also copy to ~/Applications and register the
#                         login LaunchAgent (org.mountd.menubar)
set -euo pipefail

cd "$(dirname "$0")"
APP_NAME="Mountd Menu Bar"
BUILD_DIR="build"
APP="$BUILD_DIR/$APP_NAME.app"
BIN="$APP/Contents/MacOS/mountd-menubar"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

swiftc -O -swift-version 5 -o "$BIN" Sources/main.swift

cat > "$APP/Contents/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>org.mountd.menubar</string>
    <key>CFBundleName</key>
    <string>Mountd Menu Bar</string>
    <key>CFBundleExecutable</key>
    <string>mountd-menubar</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>0.3.0</string>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
    <key>LSUIElement</key>
    <true/>
</dict>
</plist>
EOF

# Roundtrip through ditto to shed resource forks / Finder detritus that
# codesign rejects (xattr -cr alone can't clear com.apple.provenance).
STAGED="$BUILD_DIR/staged.app"
rm -rf "$STAGED"
ditto --norsrc --noextattr --noqtn "$APP" "$STAGED"
rm -rf "$APP"
mv "$STAGED" "$APP"
codesign --force --sign - "$APP"
echo "built: $APP"

if [[ "${1:-}" == "install" ]]; then
    DEST="$HOME/Applications/$APP_NAME.app"
    # bootout first so the running app doesn't hold the old binary
    launchctl bootout "gui/$(id -u)/org.mountd.menubar" 2>/dev/null || true
    pkill -f "$APP_NAME.app/Contents/MacOS/mountd-menubar" 2>/dev/null || true
    rm -rf "$DEST"
    mkdir -p "$HOME/Applications"
    cp -R "$APP" "$DEST"

    PLIST="$HOME/Library/LaunchAgents/org.mountd.menubar.plist"
    cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>org.mountd.menubar</string>
    <key>ProgramArguments</key>
    <array>
        <string>$DEST/Contents/MacOS/mountd-menubar</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
EOF
    launchctl bootstrap "gui/$(id -u)" "$PLIST"
    echo "installed: $DEST (login item via org.mountd.menubar)"
fi
