// AnyQuake GNOME Shell extension (installed by anyquake, do not edit).
//
// Runs inside GNOME Shell. Owns one global shortcut per configured
// application and shows / hides / places its window. The anyquake daemon
// configures it over D-Bus and is called back to start the applications and
// to persist the identity of the managed windows.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

// Must match EXTENSION_VERSION in gnome.rs.
const VERSION = 2;

const BUS_NAME = 'de.anyquake.GnomeShell';
const OBJECT_PATH = '/de/anyquake/GnomeShell';
const IFACE = `
<node>
  <interface name="de.anyquake.GnomeShell">
    <method name="Configure">
      <arg type="s" direction="in" name="config"/>
      <arg type="s" direction="out" name="warning"/>
    </method>
    <method name="Toggle"/>
    <method name="ToggleApp">
      <arg type="u" direction="in" name="app"/>
    </method>
    <method name="Release"/>
    <property name="Version" type="u" access="read"/>
  </interface>
</node>`;

const DAEMON_NAME = 'de.anyquake.Daemon';
const DAEMON_PATH = '/de/anyquake/Daemon';
const DAEMON_IFACE = 'de.anyquake.Daemon';

const LAUNCH_TIMEOUT_MS = 30000;
const MONITORS_CHANGED_DELAY_MS = 250;

// One configured application: its configuration as sent by the daemon and
// the window managed for it.
class App {
    constructor(controller, config) {
        this._controller = controller;
        this.config = config;
        this.action = Meta.KeyBindingAction.NONE;
        this.managed = null;
        this._managedUnmanagedId = 0;
        this.launchRequestedAt = 0;
        this.knownBeforeLaunch = [];
    }

    matches(window) {
        if (!window || window.get_window_type() !== Meta.WindowType.NORMAL)
            return false;
        const app = Shell.WindowTracker.get_default().get_window_app(window);
        const appId = app?.get_id()?.replace(/\.desktop$/, '');
        const m = this.config.match;
        return [
            window.get_wm_class(),
            window.get_wm_class_instance(),
            window.get_gtk_application_id(),
            window.get_sandboxed_app_id(),
            appId,
        ].some(v => v && v.toLowerCase() === m);
    }

    windows() {
        return global.get_window_actors()
            .map(actor => actor.get_meta_window())
            .filter(window => window && this.matches(window));
    }

    setManaged(window, report = true) {
        if (this.managed && this._managedUnmanagedId)
            this.managed.disconnect(this._managedUnmanagedId);
        this.managed = window;
        this._managedUnmanagedId = 0;
        if (!window)
            return;
        this._managedUnmanagedId = window.connect('unmanaged', () => {
            if (this.managed === window) {
                this.managed = null;
                this._managedUnmanagedId = 0;
            }
        });
        if (report) {
            this._controller.callDaemon('SetManagedApp',
                new GLib.Variant('(ss)', [this.config.index, String(window.get_id())]));
        }
    }

    findWindow() {
        if (this.managed)
            return this.managed;
        const windows = this.windows().filter(w => !this._controller.managedByOther(this, w));
        let window = null;
        if (this.config.managedId)
            window = windows.find(w => String(w.get_id()) === this.config.managedId) ?? null;
        if (!window && this.config.reuseExisting)
            window = windows[0] ?? null;
        if (window)
            this.setManaged(window);
        return window;
    }

    targetMonitor() {
        const display = global.display;
        const monitor = this.config.monitor;
        if (monitor === 'primary')
            return display.get_primary_monitor();
        if (monitor !== 'cursor') {
            const index = global.backend.get_monitor_manager().get_monitor_for_connector(monitor);
            if (index >= 0)
                return index;
        }
        return display.get_current_monitor();
    }

    _computeRect(area) {
        const pct = v => Math.min(Math.max(v, 1), 100) / 100;
        const w = Math.max(1, Math.round(area.width * pct(this.config.width)));
        const h = Math.max(1, Math.round(area.height * pct(this.config.height)));
        const left = area.x;
        const right = area.x + area.width - w;
        const hcenter = area.x + Math.floor((area.width - w) / 2);
        const top = area.y;
        const bottom = area.y + area.height - h;
        const vcenter = area.y + Math.floor((area.height - h) / 2);
        const [x, y] = {
            'top': [hcenter, top],
            'bottom': [hcenter, bottom],
            'left': [left, vcenter],
            'right': [right, vcenter],
            'center': [hcenter, vcenter],
            'top-left': [left, top],
            'top-right': [right, top],
            'bottom-left': [left, bottom],
            'bottom-right': [right, bottom],
        }[this.config.anchor] ?? [hcenter, top];
        return {x, y, width: w, height: h};
    }

    place(window, monitor) {
        if (window.get_monitor() !== monitor)
            window.move_to_monitor(monitor);
        if (this.config.anchor === 'fullscreen') {
            if (!window.is_fullscreen())
                window.make_fullscreen();
            return;
        }
        if (window.is_fullscreen())
            window.unmake_fullscreen();
        if (window.maximized_horizontally || window.maximized_vertically) {
            // GNOME 49 replaced unmaximize(flags) by set_unmaximize_flags().
            if (window.set_unmaximize_flags)
                window.set_unmaximize_flags(Meta.MaximizeFlags.BOTH);
            else
                window.unmaximize(Meta.MaximizeFlags.BOTH);
        }
        const area = this.config.respectPanels
            ? global.workspace_manager.get_active_workspace().get_work_area_for_monitor(monitor)
            : global.display.get_monitor_geometry(monitor);
        const rect = this._computeRect(area);
        window.move_resize_frame(false, rect.x, rect.y, rect.width, rect.height);
    }

    show(window, timestamp) {
        const monitor = this.targetMonitor();
        if (window.minimized)
            window.unminimize();
        if (!window.is_on_all_workspaces())
            window.stick();
        if (this.config.alwaysOnTop && !window.is_above())
            window.make_above();
        else if (!this.config.alwaysOnTop && window.is_above())
            window.unmake_above();
        this.place(window, monitor);
        Main.activateWindow(window, timestamp);
    }

    release() {
        const window = this.managed;
        if (!window)
            return;
        if (window.is_on_all_workspaces())
            window.unstick();
        if (window.is_above())
            window.unmake_above();
    }

    toggle(timestamp) {
        if (this.launchRequestedAt && Date.now() - this.launchRequestedAt < LAUNCH_TIMEOUT_MS)
            return; // still waiting for the application to start
        const window = this.findWindow();
        if (!window) {
            this.knownBeforeLaunch = this.windows();
            this.launchRequestedAt = Date.now();
            this._controller.callDaemon('LaunchApp', new GLib.Variant('(s)', [this.config.index]));
            return;
        }
        const visible = !window.minimized && window.has_focus() && !Main.overview.visible;
        if (visible && window.get_monitor() === this.targetMonitor())
            window.minimize();
        else
            this.show(window, timestamp);
    }

    resetLaunch() {
        this.launchRequestedAt = 0;
        this.knownBeforeLaunch = [];
    }

    // Shortcut

    grab() {
        const accelerator = this.config.accelerator;
        const action = global.display.grab_accelerator(accelerator, Meta.KeyBindingFlags.NONE);
        if (action === Meta.KeyBindingAction.NONE) {
            return `GNOME refused the hotkey ${accelerator}; it is probably used by another ` +
                'shortcut. Free it in Settings → Keyboard → Keyboard Shortcuts or pick another one.';
        }
        Main.wm.allowKeybinding(Meta.external_binding_name_for_action(action),
            Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW);
        this.action = action;
        return '';
    }

    ungrab() {
        if (this.action !== Meta.KeyBindingAction.NONE) {
            global.display.ungrab_accelerator(this.action);
            this.action = Meta.KeyBindingAction.NONE;
        }
    }

    // Stops managing the window (without giving it back, see release()).
    destroy() {
        this.ungrab();
        this.setManaged(null, false);
        this.resetLaunch();
    }
}

class Controller {
    constructor() {
        this._apps = [];
        this._pendingShown = new Map();
        this._monitorsTimeoutId = 0;
    }

    enable() {
        this._dbus = Gio.DBusExportedObject.wrapJSObject(IFACE, this);
        this._dbus.export(Gio.DBus.session, OBJECT_PATH);
        this._ownerId = Gio.bus_own_name_on_connection(
            Gio.DBus.session, BUS_NAME, Gio.BusNameOwnerFlags.NONE, null, null);

        this._acceleratorId = global.display.connect('accelerator-activated',
            (display, action, device, timestamp) => {
                if (action === Meta.KeyBindingAction.NONE)
                    return;
                this._apps.find(app => app.action === action)?.toggle(timestamp);
            });
        this._windowCreatedId = global.display.connect('window-created',
            (display, window) => this._onWindowCreated(window));
        this._monitorsChangedId = Main.layoutManager.connect('monitors-changed',
            () => this._onMonitorsChanged());

        // Give the windows back and drop the shortcuts if the daemon goes
        // away without saying goodbye (crash, kill).
        this._watchId = Gio.bus_watch_name(Gio.BusType.SESSION, DAEMON_NAME,
            Gio.BusNameWatcherFlags.NONE, null, () => this._unconfigure());

        // After unlocking the screen (extensions are disabled while it is
        // locked) the daemon has to send the configuration again.
        this.callDaemon('ExtensionReady');
    }

    disable() {
        // The screen lock disables all extensions; keep the windows as they
        // are in that case, they are picked up again after unlocking.
        const locking = Main.sessionMode.isLocked;
        for (const app of this._apps) {
            if (!locking)
                app.release();
            app.destroy();
        }
        this._apps = [];

        for (const [window, id] of this._pendingShown)
            window.disconnect(id);
        this._pendingShown.clear();
        if (this._monitorsTimeoutId) {
            GLib.source_remove(this._monitorsTimeoutId);
            this._monitorsTimeoutId = 0;
        }
        Gio.bus_unwatch_name(this._watchId);
        Main.layoutManager.disconnect(this._monitorsChangedId);
        global.display.disconnect(this._windowCreatedId);
        global.display.disconnect(this._acceleratorId);
        Gio.bus_unown_name(this._ownerId);
        this._dbus.unexport();
        this._dbus = null;
    }

    // D-Bus interface

    get Version() {
        return VERSION;
    }

    Configure(json) {
        const configs = JSON.parse(json).apps ?? [];
        const old = this._apps;
        for (const app of old)
            app.ungrab();
        this._apps = configs.map((config, i) => {
            const app = new App(this, config);
            // Keep the window of an application that is still configured at
            // the same position; give it back otherwise.
            const prev = old[i];
            if (prev?.managed && prev.config.match === config.match)
                app.setManaged(prev.managed, false);
            else
                prev?.release();
            if (prev && prev.config.match === config.match) {
                app.launchRequestedAt = prev.launchRequestedAt;
                app.knownBeforeLaunch = prev.knownBeforeLaunch;
            }
            return app;
        });
        for (const prev of old.slice(configs.length))
            prev.release();
        for (const prev of old)
            prev.destroy();
        return this._apps.map(app => app.grab()).filter(warning => warning).join(' ');
    }

    Toggle() {
        this.ToggleApp(0);
    }

    ToggleApp(index) {
        this._apps[index]?.toggle(global.display.get_current_time_roundtrip());
    }

    Release() {
        this._unconfigure();
    }

    _unconfigure() {
        for (const app of this._apps) {
            app.release();
            app.destroy();
        }
        this._apps = [];
    }

    managedByOther(app, window) {
        return this._apps.some(other => other !== app && other.managed === window);
    }

    // Windows

    _onWindowCreated(window) {
        const now = Date.now();
        let waiting = false;
        for (const app of this._apps) {
            if (!app.launchRequestedAt)
                continue;
            if (now - app.launchRequestedAt > LAUNCH_TIMEOUT_MS)
                app.resetLaunch();
            else
                waiting = true;
        }
        if (!waiting)
            return;
        // On Wayland the app id may only be known once the window is mapped,
        // and placing it before that is overridden by the initial placement.
        const id = window.connect('shown', () => {
            window.disconnect(id);
            this._pendingShown.delete(window);
            this._onLaunchedWindowShown(window);
        });
        this._pendingShown.set(window, id);
        window.connect('unmanaged', () => {
            const pending = this._pendingShown.get(window);
            if (pending) {
                window.disconnect(pending);
                this._pendingShown.delete(window);
            }
        });
    }

    _onLaunchedWindowShown(window) {
        const app = this._apps.find(app => app.launchRequestedAt && app.matches(window) &&
            !app.knownBeforeLaunch.includes(window) && !this.managedByOther(app, window));
        if (!app)
            return;
        app.resetLaunch();
        app.setManaged(window);
        app.show(window, global.display.get_current_time_roundtrip());
    }

    _onMonitorsChanged() {
        // Let mutter finish moving windows to the new layout first.
        if (this._monitorsTimeoutId)
            GLib.source_remove(this._monitorsTimeoutId);
        this._monitorsTimeoutId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, MONITORS_CHANGED_DELAY_MS, () => {
            this._monitorsTimeoutId = 0;
            for (const app of this._apps) {
                const window = app.managed;
                if (window && !window.minimized) {
                    const monitor = window.get_monitor();
                    app.place(window, monitor >= 0 ? monitor : app.targetMonitor());
                }
            }
            return GLib.SOURCE_REMOVE;
        });
    }

    callDaemon(method, params = null) {
        Gio.DBus.session.call(DAEMON_NAME, DAEMON_PATH, DAEMON_IFACE, method, params, null,
            Gio.DBusCallFlags.NO_AUTO_START, -1, null, (connection, result) => {
                try {
                    connection.call_finish(result);
                } catch (e) {
                    if (!e.matches(Gio.DBusError, Gio.DBusError.SERVICE_UNKNOWN) &&
                        !e.matches(Gio.DBusError, Gio.DBusError.NAME_HAS_NO_OWNER))
                        console.warn(`AnyQuake: calling daemon ${method}: ${e.message}`);
                }
            });
    }
}

export default class AnyQuakeExtension extends Extension {
    enable() {
        this._controller = new Controller();
        this._controller.enable();
    }

    disable() {
        this._controller.disable();
        this._controller = null;
    }
}
