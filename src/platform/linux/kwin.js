// anyquake KWin script (generated, do not edit).
//
// Runs inside KWin. Owns the global shortcut and shows / hides / places the
// target window. Talks back to the anyquake daemon over D-Bus to start the
// application and to persist the identity of the managed window.

const CONFIG = __ANYQUAKE_CONFIG__;

const DBUS_SERVICE = "de.anyquake.Daemon";
const DBUS_PATH = "/de/anyquake/Daemon";
const DBUS_IFACE = "de.anyquake.Daemon";
const LAUNCH_TIMEOUT_MS = 30000;

let managed = null;
let launchRequestedAt = 0;
let knownBeforeLaunch = [];

function lower(v) {
    return v ? String(v).toLowerCase() : "";
}

function matches(w) {
    if (!w || !w.normalWindow) {
        return false;
    }
    const m = CONFIG.match;
    return lower(w.resourceClass) === m || lower(w.resourceName) === m || lower(w.desktopFileName) === m;
}

function setManaged(w) {
    managed = w;
    callDBus(DBUS_SERVICE, DBUS_PATH, DBUS_IFACE, "SetManaged", String(w.internalId));
}

function findWindow() {
    if (managed) {
        return managed;
    }
    const windows = workspace.windowList();
    if (CONFIG.managedId) {
        for (const w of windows) {
            if (String(w.internalId) === CONFIG.managedId && matches(w)) {
                setManaged(w);
                return w;
            }
        }
    }
    if (CONFIG.reuseExisting) {
        for (const w of windows) {
            if (matches(w)) {
                setManaged(w);
                return w;
            }
        }
    }
    return null;
}

function targetOutput() {
    const screens = workspace.screens;
    if (CONFIG.monitor === "primary") {
        return screens[0];
    }
    if (CONFIG.monitor !== "cursor") {
        for (const s of screens) {
            if (lower(s.name) === lower(CONFIG.monitor)) {
                return s;
            }
        }
    }
    return workspace.screenAt(workspace.cursorPos) || workspace.activeScreen;
}

function outputByName(name) {
    for (const s of workspace.screens) {
        if (s.name === name) {
            return s;
        }
    }
    return null;
}

function computeRect(area) {
    const pct = v => Math.min(Math.max(v, 1), 100) / 100;
    const w = Math.max(1, Math.round(area.width * pct(CONFIG.width)));
    const h = Math.max(1, Math.round(area.height * pct(CONFIG.height)));
    const left = area.x;
    const right = area.x + area.width - w;
    const hcenter = area.x + Math.floor((area.width - w) / 2);
    const top = area.y;
    const bottom = area.y + area.height - h;
    const vcenter = area.y + Math.floor((area.height - h) / 2);
    const pos = {
        "top": [hcenter, top],
        "bottom": [hcenter, bottom],
        "left": [left, vcenter],
        "right": [right, vcenter],
        "center": [hcenter, vcenter],
        "top-left": [left, top],
        "top-right": [right, top],
        "bottom-left": [left, bottom],
        "bottom-right": [right, bottom],
    }[CONFIG.anchor] || [hcenter, top];
    return { x: pos[0], y: pos[1], width: w, height: h };
}

function place(w, output) {
    if (CONFIG.anchor === "fullscreen") {
        w.frameGeometry = output.geometry;
        w.fullScreen = true;
        return;
    }
    if (w.fullScreen) {
        w.fullScreen = false;
    }
    if (w.maximizable) {
        w.setMaximize(false, false);
    }
    const area = CONFIG.respectPanels
        ? workspace.clientArea(KWin.MaximizeArea, output, workspace.currentDesktop)
        : output.geometry;
    w.frameGeometry = computeRect(area);
}

function show(w) {
    const output = targetOutput();
    w.minimized = false;
    w.onAllDesktops = true;
    w.skipTaskbar = true;
    w.skipPager = true;
    w.skipSwitcher = true;
    w.keepAbove = CONFIG.alwaysOnTop;
    place(w, output);
    workspace.activeWindow = w;
}

function hide(w) {
    w.minimized = true;
}

function toggle() {
    if (launchRequestedAt && Date.now() - launchRequestedAt < LAUNCH_TIMEOUT_MS) {
        return; // still waiting for the application to start
    }
    const w = findWindow();
    if (!w) {
        knownBeforeLaunch = workspace.windowList().filter(matches);
        launchRequestedAt = Date.now();
        callDBus(DBUS_SERVICE, DBUS_PATH, DBUS_IFACE, "Launch");
        return;
    }
    const onTarget = w.output && w.output.name === targetOutput().name;
    if (!w.minimized && workspace.activeWindow === w && onTarget) {
        hide(w);
    } else {
        show(w);
    }
}

function release() {
    const w = managed;
    if (!w) {
        return;
    }
    w.skipTaskbar = false;
    w.skipPager = false;
    w.skipSwitcher = false;
    w.keepAbove = false;
    w.onAllDesktops = false;
    managed = null;
}

workspace.windowAdded.connect(function (w) {
    if (!launchRequestedAt) {
        return;
    }
    if (Date.now() - launchRequestedAt > LAUNCH_TIMEOUT_MS) {
        launchRequestedAt = 0;
        return;
    }
    if (!matches(w) || knownBeforeLaunch.indexOf(w) >= 0) {
        return;
    }
    launchRequestedAt = 0;
    knownBeforeLaunch = [];
    setManaged(w);
    show(w);
});

workspace.windowRemoved.connect(function (w) {
    if (w === managed) {
        managed = null;
    }
});

function screensChanged() {
    const w = managed;
    if (!w || w.minimized) {
        return;
    }
    const output = (w.output && outputByName(w.output.name)) || targetOutput();
    place(w, output);
}

workspace.screensChanged.connect(screensChanged);
workspace.virtualScreenGeometryChanged.connect(screensChanged);

registerShortcut(CONFIG.shortcutName, CONFIG.shortcutText, CONFIG.hotkey, toggle);
registerShortcut(CONFIG.releaseShortcutName, CONFIG.releaseShortcutText, "", release);
