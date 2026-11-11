#!/bin/zsh
# Build (and optionally install) the mountd menu bar app.
#   ./build.sh            build menubar/build/Mountd Menu Bar.app
#   ./build.sh install    also copy to ~/Applications and register the
#                         login LaunchAgent (org.mountd.menubar)
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "building the menu bar app requires macOS" >&2
    exit 1
fi

cd "$(dirname "$0")"
APP_NAME="Mountd Menu Bar"
BUILD_DIR="build"
APP="$BUILD_DIR/$APP_NAME.app"
BIN="$APP/Contents/MacOS/mountd-menubar"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

swiftc -O -swift-version 5 -o "$BIN" Sources/main.swift Sources/Command.swift

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
    <string>0.4.0</string>
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
    mkdir -p "$HOME/Applications" "$HOME/Library/LaunchAgents"
    STAGE="$(mktemp -d "$HOME/Applications/.mountd-install.XXXXXX")"
    PLIST="$HOME/Library/LaunchAgents/org.mountd.menubar.plist"
    SERVICE="gui/$(id -u)/org.mountd.menubar"
    HAD_APP=0
    HAD_PLIST=0
    CHANGED=0
    STOPPED=0
    UNSETTLED=0
    service_loaded() {
        local details
        if details="$(launchctl print "$SERVICE" 2>&1)"; then
            return 0
        elif [[ "$details" == *'Could not find service "org.mountd.menubar"'* ]]; then
            return 1
        fi
        print -u2 -- "$details"
        return 2
    }
    stop_service() {
        launchctl bootout "$SERVICE" && return 0
        local launch_exit=$?
        if (( launch_exit > 128 )); then
            UNSETTLED=1
            return 1
        fi
        if service_loaded; then
            return 1
        else
            [[ "$?" == 1 ]]
        fi
    }
    start_service() {
        launchctl bootstrap "gui/$(id -u)" "$PLIST" && return 0
        local launch_exit=$?
        if (( launch_exit > 128 )); then
            UNSETTLED=1
            return 1
        fi
        # The request may have succeeded before its client reported failure.
        service_loaded
    }
    rollback() {
        if [[ "$UNSETTLED" == 1 ]]; then
            print -u2 -- "service state uncertain; recovery files retained at $STAGE"
            return 1
        fi
        local failed=0
        if [[ "$CHANGED" == 1 ]]; then
            if service_loaded; then
                stop_service || failed=1
            elif [[ "$?" != 1 ]]; then
                failed=1
            fi
            if [[ "$failed" == 1 ]]; then
                print -u2 -- "service state uncertain; recovery files retained at $STAGE"
                return 1
            fi
            if ! rm -rf "$DEST"; then
                failed=1
            elif [[ "$HAD_APP" == 1 ]] && ! mv "$STAGE/old.app" "$DEST"; then
                failed=1
            fi
            if [[ "$HAD_PLIST" == 1 ]]; then
                cp "$STAGE/old.plist" "$PLIST" || failed=1
            else
                rm -f "$PLIST" || failed=1
            fi
        fi
        if [[ "$STOPPED" == 1 && "$failed" == 0 ]]; then
            start_service || failed=1
        fi
        if [[ "$failed" == 1 ]]; then
            print -u2 -- "rollback incomplete; recovery files retained at $STAGE"
            return 1
        fi
        rm -rf "$STAGE"
    }
    trap rollback EXIT
    cp -R "$APP" "$STAGE/new.app"
    [[ ! -e "$PLIST" ]] || { cp "$PLIST" "$STAGE/old.plist"; HAD_PLIST=1; }
    # Never match process names: only this user's exact launchd service is ours.
    if service_loaded; then
        stop_service || exit 1
        STOPPED=1
    elif [[ "$?" != 1 ]]; then
        exit 1
    fi
    if [[ -e "$DEST" ]]; then
        mv "$DEST" "$STAGE/old.app"
        HAD_APP=1
    fi
    CHANGED=1
    mv "$STAGE/new.app" "$DEST"
    # XML text must preserve home paths containing ampersands or angle brackets.
    XML_BIN="$(print -r -- "$DEST/Contents/MacOS/mountd-menubar" | sed -e 's/\&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g')"
    cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>org.mountd.menubar</string>
    <key>ProgramArguments</key>
    <array>
        <string>$XML_BIN</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <!-- Restart if it crashes, but honor the Quit menu item (a clean exit).
         Without this a single crash silently removes the icon until the next
         login, while the daemon keeps mounting and unmounting unobserved. -->
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
</dict>
</plist>
EOF
    start_service || exit 1
    CHANGED=0
    STOPPED=0
    echo "installed: $DEST (login item via org.mountd.menubar)"
fi
