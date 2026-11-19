// anyquake KWin script (generated, do not edit).
//
// Runs inside KWin. Owns one global shortcut per configured application and
// shows / hides / places its window. Talks back to the anyquake daemon over
// D-Bus to start applications and to persist the identity of the managed
// windows.

const CONFIG = __ANYQUAKE_CONFIG__;

const DBUS_SERVICE = "de.anyquake.Daemon";
const DBUS_PATH = "/de/anyquake/Daemon";
const DBUS_IFACE = "de.anyquake.Daemon";
const LAUNCH_TIMEOUT_MS = 30000;

// Runtime state per application (same order as CONFIG.apps).
const apps = CONFIG.apps.map(function (config) {
    return { config: config, managed: null, launchRequestedAt: 0, knownBeforeLaunch: [] };
});

function lower(v) {
    return v ? String(v).toLowerCase() : "";
}

function matches(app, w) {
    if (!w || !w.normalWindow) {
        return false;
    }
    const m = app.config.match;
    return lower(w.resourceClass) === m || lower(w.resourceName) === m || lower(w.desktopFileName) === m;
}

function managedByOther(app, w) {
    return apps.some(function (other) {
        return other !== app && other.managed === w;
    });
}

function setManaged(app, w) {
    app.managed = w;
    callDBus(DBUS_SERVICE, DBUS_PATH, DBUS_IFACE, "SetManagedApp", app.config.index, String(w.internalId));
}

function findWindow(app) {
    if (app.managed) {
        return app.managed;
    }
    const windows = workspace.windowList().filter(function (w) {
        return matches(app, w) && !managedByOther(app, w);
    });
    if (app.config.managedId) {
        for (const w of windows) {
            if (String(w.internalId) === app.config.managedId) {
                setManaged(app, w);
                return w;
            }
        }
    }
    if (app.config.reuseExisting && windows.length > 0) {
        setManaged(app, windows[0]);
        return windows[0];
    }
    return null;
}

function targetOutput(app) {
    const screens = workspace.screens;
    const monitor = app.config.monitor;
    if (monitor === "primary") {
        return screens[0];
    }
    if (monitor !== "cursor") {
        for (const s of screens) {
            if (lower(s.name) === lower(monitor)) {
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

function computeRect(config, area) {
    const pct = v => Math.min(Math.max(v, 1), 100) / 100;
    const w = Math.max(1, Math.round(area.width * pct(config.width)));
    const h = Math.max(1, Math.round(area.height * pct(config.height)));
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
    }[config.anchor] || [hcenter, top];
    return { x: pos[0], y: pos[1], width: w, height: h };
}

function place(app, w, output) {
    const config = app.config;
    if (config.anchor === "fullscreen") {
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
    const area = config.respectPanels
        ? workspace.clientArea(KWin.MaximizeArea, output, workspace.currentDesktop)
        : output.geometry;
    w.frameGeometry = computeRect(config, area);
}

function show(app, w) {
    const output = targetOutput(app);
    w.minimized = false;
    w.onAllDesktops = true;
    w.skipTaskbar = true;
    w.skipPager = true;
    w.skipSwitcher = true;
    w.keepAbove = app.config.alwaysOnTop;
    place(app, w, output);
    workspace.activeWindow = w;
}

function hide(w) {
    w.minimized = true;
}

// Tells the daemon what toggling did, for its log.
function report(app, action) {
    callDBus(DBUS_SERVICE, DBUS_PATH, DBUS_IFACE, "AppToggled", app.config.index, action);
}

function toggle(app) {
    if (app.launchRequestedAt && Date.now() - app.launchRequestedAt < LAUNCH_TIMEOUT_MS) {
        report(app, "still waiting for the application to start");
        return;
    }
    const w = findWindow(app);
    if (!w) {
        report(app, "no window, launching");
        app.knownBeforeLaunch = workspace.windowList().filter(function (w) {
            return matches(app, w);
        });
        app.launchRequestedAt = Date.now();
        callDBus(DBUS_SERVICE, DBUS_PATH, DBUS_IFACE, "LaunchApp", app.config.index);
        return;
    }
    const onTarget = w.output && w.output.name === targetOutput(app).name;
    if (!w.minimized && workspace.activeWindow === w && onTarget) {
        report(app, "hiding");
        hide(w);
    } else {
        report(app, "showing");
        show(app, w);
    }
}

function release() {
    for (const app of apps) {
        const w = app.managed;
        if (!w) {
            continue;
        }
        w.skipTaskbar = false;
        w.skipPager = false;
        w.skipSwitcher = false;
        w.keepAbove = false;
        w.onAllDesktops = false;
        app.managed = null;
    }
}

workspace.windowAdded.connect(function (w) {
    for (const app of apps) {
        if (!app.launchRequestedAt) {
            continue;
        }
        if (Date.now() - app.launchRequestedAt > LAUNCH_TIMEOUT_MS) {
            app.launchRequestedAt = 0;
            continue;
        }
        if (!matches(app, w) || app.knownBeforeLaunch.indexOf(w) >= 0 || managedByOther(app, w)) {
            continue;
        }
        app.launchRequestedAt = 0;
        app.knownBeforeLaunch = [];
        setManaged(app, w);
        show(app, w);
        return;
    }
});

workspace.windowRemoved.connect(function (w) {
    for (const app of apps) {
        if (w === app.managed) {
            app.managed = null;
        }
    }
});

function screensChanged() {
    for (const app of apps) {
        const w = app.managed;
        if (!w || w.minimized) {
            continue;
        }
        const output = (w.output && outputByName(w.output.name)) || targetOutput(app);
        place(app, w, output);
    }
}

workspace.screensChanged.connect(screensChanged);
workspace.virtualScreenGeometryChanged.connect(screensChanged);

for (const app of apps) {
    registerShortcut(app.config.shortcutName, app.config.shortcutText, app.config.hotkey, function () {
        toggle(app);
    });
}
registerShortcut(CONFIG.releaseShortcutName, CONFIG.releaseShortcutText, "", release);
