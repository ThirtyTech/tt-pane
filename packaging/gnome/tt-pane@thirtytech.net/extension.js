// tt-pane GNOME Shell extension.
//
// GNOME on Wayland gives ordinary clients no way to move another application's
// window, so tt-pane asks the shell over the session bus. This extension only
// reports windows, and moves or focuses the window tt-pane names; all placement
// and navigation logic stays in tt-pane.

import GLib from 'gi://GLib';
import Gio from 'gi://Gio';
import Meta from 'gi://Meta';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const BUS_NAME = 'net.thirtytech.TtPane';
const OBJECT_PATH = '/net/thirtytech/TtPane';
const API_VERSION = 2;

const INTERFACE = `<node>
  <interface name="net.thirtytech.TtPane">
    <method name="GetFocusedWindow">
      <arg type="t" direction="out" name="id"/>
      <arg type="(iiii)" direction="out" name="frame"/>
      <arg type="(iiii)" direction="out" name="work_area"/>
      <arg type="b" direction="out" name="fullscreen"/>
    </method>
    <method name="MoveResize">
      <arg type="t" direction="in" name="id"/>
      <arg type="i" direction="in" name="x"/>
      <arg type="i" direction="in" name="y"/>
      <arg type="i" direction="in" name="width"/>
      <arg type="i" direction="in" name="height"/>
    </method>
    <method name="GetFocusedId">
      <arg type="t" direction="out" name="id"/>
    </method>
    <method name="GetWindow">
      <arg type="t" direction="in" name="id"/>
      <arg type="b" direction="out" name="available"/>
      <arg type="(iiii)" direction="out" name="frame"/>
      <arg type="(iiii)" direction="out" name="work_area"/>
    </method>
    <method name="ListWindows">
      <arg type="a(t(iiii)(iiii)ib)" direction="out" name="windows"/>
      <arg type="a(iiii)" direction="out" name="monitors"/>
    </method>
    <method name="Activate">
      <arg type="t" direction="in" name="id"/>
    </method>
    <property name="Version" type="u" access="read"/>
  </interface>
</node>`;

const MOVABLE_TYPES = [
    Meta.WindowType.NORMAL,
    Meta.WindowType.DIALOG,
    Meta.WindowType.MODAL_DIALOG,
    Meta.WindowType.UTILITY,
];

function rect(r) {
    return [r.x, r.y, r.width, r.height];
}

function findWindow(id) {
    for (const actor of global.get_window_actors()) {
        const window = actor.get_meta_window();
        if (window && window.get_id() === id)
            return window;
    }
    return null;
}

// GNOME 49 replaced get_maximized() with is_maximized().
function isMaximized(window) {
    if (typeof window.get_maximized === 'function')
        return window.get_maximized() !== 0;
    return window.is_maximized();
}

function isApplication(window) {
    return MOVABLE_TYPES.includes(window.get_window_type()) && !window.is_skip_taskbar();
}

// Visible on the active workspace, not minimized: what a user could focus by sight.
function isShowing(window) {
    const workspace = global.workspace_manager.get_active_workspace();
    return !window.minimized && window.located_on_workspace(workspace);
}

// A restored application window tt-pane may move or focus without restoring it.
function isAvailable(window) {
    return isApplication(window) && isShowing(window) &&
        !window.is_fullscreen() && !isMaximized(window);
}

// GNOME 49 replaced get_maximized()/unmaximize(flags) with is_maximized()/unmaximize().
function unmaximize(window) {
    if (typeof window.get_maximized === 'function') {
        const flags = window.get_maximized();
        if (flags)
            window.unmaximize(flags);
        return flags !== 0;
    }
    if (window.is_maximized()) {
        window.unmaximize();
        return true;
    }
    return false;
}

class Service {
    GetFocusedWindow() {
        const window = global.display.get_focus_window();
        if (!window || !MOVABLE_TYPES.includes(window.get_window_type()))
            throw new Error('No focused application window');
        return [
            window.get_id(),
            rect(window.get_frame_rect()),
            rect(window.get_work_area_current_monitor()),
            window.is_fullscreen(),
        ];
    }

    MoveResize(id, x, y, width, height) {
        const window = findWindow(id);
        if (!window)
            throw new Error('The window closed before it could be positioned');
        if (window.is_fullscreen())
            throw new Error('Exit fullscreen before positioning this window');
        const wasMaximized = unmaximize(window);
        window.move_resize_frame(true, x, y, width, height);
        // A Wayland client applies an unmaximize asynchronously and can restore
        // its previous size afterwards, so place it once more when idle.
        if (wasMaximized) {
            GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
                if (findWindow(id) === window)
                    window.move_resize_frame(true, x, y, width, height);
                return GLib.SOURCE_REMOVE;
            });
        }
    }

    GetFocusedId() {
        const window = global.display.get_focus_window();
        return window && isApplication(window) ? window.get_id() : 0;
    }

    GetWindow(id) {
        const window = findWindow(id);
        if (!window || !isAvailable(window))
            return [false, [0, 0, 0, 0], [0, 0, 0, 0]];
        return [
            true,
            rect(window.get_frame_rect()),
            rect(window.get_work_area_current_monitor()),
        ];
    }

    // Showing windows on the active workspace, topmost first, flagged when they
    // are application windows tt-pane may focus. Others still cover what is below.
    ListWindows() {
        const windows = global.get_window_actors()
            .map(actor => actor.get_meta_window())
            .filter(window => window && isShowing(window));
        const stacked = global.display.sort_windows_by_stacking(windows).reverse();
        return [
            stacked.map(window => [
                window.get_id(),
                rect(window.get_frame_rect()),
                rect(window.get_work_area_current_monitor()),
                window.get_monitor(),
                isApplication(window),
            ]),
            Main.layoutManager.monitors.map(m => [m.x, m.y, m.width, m.height]),
        ];
    }

    Activate(id) {
        const window = findWindow(id);
        if (!window || !isShowing(window))
            throw new Error('The window closed before it could be focused');
        Main.activateWindow(window);
    }

    get Version() {
        return API_VERSION;
    }
}

export default class TtPaneExtension extends Extension {
    enable() {
        this._dbus = Gio.DBusExportedObject.wrapJSObject(INTERFACE, new Service());
        this._dbus.export(Gio.DBus.session, OBJECT_PATH);
        this._owner = Gio.bus_own_name_on_connection(
            Gio.DBus.session,
            BUS_NAME,
            Gio.BusNameOwnerFlags.NONE,
            null,
            null
        );
    }

    disable() {
        if (this._owner) {
            Gio.bus_unown_name(this._owner);
            this._owner = 0;
        }
        this._dbus?.unexport();
        this._dbus = null;
    }
}
