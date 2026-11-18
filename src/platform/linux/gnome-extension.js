// AnyQuake GNOME Shell extension (installed by anyquake, do not edit).
//
// Runs inside GNOME Shell. Owns the global shortcut and shows / hides /
// places the target window. The anyquake daemon configures it over D-Bus
// and is called back to start the application and to persist the identity
// of the managed window.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

// Must match EXTENSION_VERSION in gnome.rs.
const VERSION = 1;

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
    <method name="Release"/>
    <property name="Version" type="u" access="read"/>
  </interface>
</node>`;

const DAEMON_NAME = 'de.anyquake.Daemon';
const DAEMON_PATH = '/de/anyquake/Daemon';
const DAEMON_IFACE = 'de.anyquake.Daemon';

const LAUNCH_TIMEOUT_MS = 30000;
const MONITORS_CHANGED_DELAY_MS = 250;

class Controller {
    constructor() {
        this._config = null;
        this._action = Meta.KeyBindingAction.NONE;
        this._managed = null;
        this._managedUnmanagedId = 0;
        this._launchRequestedAt = 0;
        this._knownBeforeLaunch = [];
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
                if (action !== Meta.KeyBindingAction.NONE && action === this._action)
                    this._toggle(timestamp);
            });
        this._windowCreatedId = global.display.connect('window-created',
            (display, window) => this._onWindowCreated(window));
        this._monitorsChangedId = Main.layoutManager.connect('monitors-changed',
            () => this._onMonitorsChanged());

        // Give the window back and drop the shortcut if the daemon goes away
        // without saying goodbye (crash, kill).
        this._watchId = Gio.bus_watch_name(Gio.BusType.SESSION, DAEMON_NAME,
            Gio.BusNameWatcherFlags.NONE, null, () => this._unconfigure());

        // After unlocking the screen (extensions are disabled while it is
        // locked) the daemon has to send the configuration again.
        this._callDaemon('ExtensionReady');
    }

    disable() {
        // The screen lock disables all extensions; keep the window as it is
        // in that case, it is picked up again after unlocking.
        const locking = Main.sessionMode.isLocked;
        this._ungrab();
        if (!locking)
            this._release();
        this._config = null;
        this._setManaged(null, false);
        this._resetLaunch();

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
        this._config = JSON.parse(json);
        if (this._managed && !this._matches(this._managed)) {
            this._release();
            this._setManaged(null, false);
        }
        this._ungrab();
        return this._grab(this._config.accelerator);
    }

    Toggle() {
        this._toggle(global.display.get_current_time_roundtrip());
    }

    Release() {
        this._unconfigure();
    }

    // Shortcut

    _grab(accelerator) {
        const action = global.display.grab_accelerator(accelerator, Meta.KeyBindingFlags.NONE);
        if (action === Meta.KeyBindingAction.NONE) {
            return `GNOME refused the hotkey ${accelerator}; it is probably used by another ` +
                'shortcut. Free it in Settings → Keyboard → Keyboard Shortcuts or pick another one.';
        }
        Main.wm.allowKeybinding(Meta.external_binding_name_for_action(action),
            Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW);
        this._action = action;
        return '';
    }

    _ungrab() {
        if (this._action !== Meta.KeyBindingAction.NONE) {
            global.display.ungrab_accelerator(this._action);
            this._action = Meta.KeyBindingAction.NONE;
        }
    }

    _unconfigure() {
        this._ungrab();
        this._release();
        this._setManaged(null, false);
        this._resetLaunch();
        this._config = null;
    }

    // Windows

    _matches(window) {
        if (!window || window.get_window_type() !== Meta.WindowType.NORMAL)
            return false;
        const app = Shell.WindowTracker.get_default().get_window_app(window);
        const appId = app?.get_id()?.replace(/\.desktop$/, '');
        const m = this._config.match;
        return [
            window.get_wm_class(),
            window.get_wm_class_instance(),
            window.get_gtk_application_id(),
            window.get_sandboxed_app_id(),
            appId,
        ].some(v => v && v.toLowerCase() === m);
    }

    _windows() {
        return global.get_window_actors()
            .map(actor => actor.get_meta_window())
            .filter(window => window && this._matches(window));
    }

    _setManaged(window, report = true) {
        if (this._managed && this._managedUnmanagedId)
            this._managed.disconnect(this._managedUnmanagedId);
        this._managed = window;
        this._managedUnmanagedId = 0;
        if (!window)
            return;
        this._managedUnmanagedId = window.connect('unmanaged', () => {
            if (this._managed === window) {
                this._managed = null;
                this._managedUnmanagedId = 0;
            }
        });
        if (report)
            this._callDaemon('SetManaged', new GLib.Variant('(s)', [String(window.get_id())]));
    }

    _findWindow() {
        if (this._managed)
            return this._managed;
        const windows = this._windows();
        let window = null;
        if (this._config.managedId)
            window = windows.find(w => String(w.get_id()) === this._config.managedId) ?? null;
        if (!window && this._config.reuseExisting)
            window = windows[0] ?? null;
        if (window)
            this._setManaged(window);
        return window;
    }

    _targetMonitor() {
        const display = global.display;
        const monitor = this._config.monitor;
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
        const w = Math.max(1, Math.round(area.width * pct(this._config.width)));
        const h = Math.max(1, Math.round(area.height * pct(this._config.height)));
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
        }[this._config.anchor] ?? [hcenter, top];
        return {x, y, width: w, height: h};
    }

    _place(window, monitor) {
        if (window.get_monitor() !== monitor)
            window.move_to_monitor(monitor);
        if (this._config.anchor === 'fullscreen') {
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
        const area = this._config.respectPanels
            ? global.workspace_manager.get_active_workspace().get_work_area_for_monitor(monitor)
            : global.display.get_monitor_geometry(monitor);
        const rect = this._computeRect(area);
        window.move_resize_frame(false, rect.x, rect.y, rect.width, rect.height);
    }

    _show(window, timestamp) {
        const monitor = this._targetMonitor();
        if (window.minimized)
            window.unminimize();
        if (!window.is_on_all_workspaces())
            window.stick();
        if (this._config.alwaysOnTop && !window.is_above())
            window.make_above();
        else if (!this._config.alwaysOnTop && window.is_above())
            window.unmake_above();
        this._place(window, monitor);
        Main.activateWindow(window, timestamp);
    }

    _release() {
        const window = this._managed;
        if (!window)
            return;
        if (window.is_on_all_workspaces())
            window.unstick();
        if (window.is_above())
            window.unmake_above();
    }

    _toggle(timestamp) {
        if (!this._config)
            return;
        if (this._launchRequestedAt && Date.now() - this._launchRequestedAt < LAUNCH_TIMEOUT_MS)
            return; // still waiting for the application to start
        const window = this._findWindow();
        if (!window) {
            this._knownBeforeLaunch = this._windows();
            this._launchRequestedAt = Date.now();
            this._callDaemon('Launch');
            return;
        }
        const visible = !window.minimized && window.has_focus() && !Main.overview.visible;
        if (visible && window.get_monitor() === this._targetMonitor())
            window.minimize();
        else
            this._show(window, timestamp);
    }

    _resetLaunch() {
        this._launchRequestedAt = 0;
        this._knownBeforeLaunch = [];
    }

    _onWindowCreated(window) {
        if (!this._launchRequestedAt)
            return;
        if (Date.now() - this._launchRequestedAt > LAUNCH_TIMEOUT_MS) {
            this._resetLaunch();
            return;
        }
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
        if (!this._config || !this._launchRequestedAt)
            return;
        if (!this._matches(window) || this._knownBeforeLaunch.includes(window))
            return;
        this._resetLaunch();
        this._setManaged(window);
        this._show(window, global.display.get_current_time_roundtrip());
    }

    _onMonitorsChanged() {
        // Let mutter finish moving windows to the new layout first.
        if (this._monitorsTimeoutId)
            GLib.source_remove(this._monitorsTimeoutId);
        this._monitorsTimeoutId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, MONITORS_CHANGED_DELAY_MS, () => {
            this._monitorsTimeoutId = 0;
            const window = this._managed;
            if (this._config && window && !window.minimized) {
                const monitor = window.get_monitor();
                this._place(window, monitor >= 0 ? monitor : this._targetMonitor());
            }
            return GLib.SOURCE_REMOVE;
        });
    }

    _callDaemon(method, params = null) {
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
