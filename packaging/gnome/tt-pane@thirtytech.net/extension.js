// tt-pane GNOME Shell extension.
//
// GNOME on Wayland gives ordinary clients no way to move another application's
// window, so tt-pane asks the shell over the session bus. This extension only
// reports windows, and moves or focuses the window tt-pane names; all placement,
// navigation and tiling logic stays in tt-pane.
//
// The dock (X11 and Wayland) is drawn here too: tt-pane lays the panel out and
// sends the rectangles, and this extension shows live clones of the docked
// windows in them, glides windows in and out, and reports clicks and drags.
//
// tt-pane writes this file as is for GNOME 45 and later, and turns the imports
// and the class below into the older extension format for GNOME 42 to 44.

import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import Gio from 'gi://Gio';
import Meta from 'gi://Meta';
import St from 'gi://St';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const BUS_NAME = 'net.thirtytech.TtPane';
const OBJECT_PATH = '/net/thirtytech/TtPane';
const API_VERSION = 5;

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
    <method name="ListTilingWindows">
      <arg type="a(t(iiii)(iiii)ssu)" direction="out" name="windows"/>
      <arg type="at" direction="out" name="identities"/>
      <arg type="as" direction="out" name="workspaces"/>
      <arg type="b" direction="out" name="interacting"/>
    </method>
    <method name="DockMonitor">
      <arg type="(iiii)" direction="out" name="monitor"/>
      <arg type="(iiii)" direction="out" name="work_area"/>
      <arg type="a(iiii)" direction="out" name="monitors"/>
    </method>
    <method name="DockHold">
      <arg type="(iiii)" direction="out" name="monitor"/>
      <arg type="(iiii)" direction="out" name="work_area"/>
      <arg type="b" direction="out" name="shown"/>
    </method>
    <method name="DockWindow">
      <arg type="t" direction="in" name="handle"/>
      <arg type="b" direction="out" name="exists"/>
      <arg type="(iiii)" direction="out" name="frame"/>
      <arg type="b" direction="out" name="minimized"/>
    </method>
    <method name="DockMark">
      <arg type="t" direction="in" name="handle"/>
      <arg type="b" direction="in" name="docked"/>
    </method>
    <method name="DockPlace">
      <arg type="t" direction="in" name="handle"/>
      <arg type="(iiii)" direction="in" name="target"/>
      <arg type="b" direction="in" name="hidden"/>
      <arg type="(iiii)" direction="out" name="frame"/>
    </method>
    <method name="DockStow">
      <arg type="t" direction="in" name="handle"/>
      <arg type="(iiii)" direction="in" name="park"/>
      <arg type="(iiii)" direction="in" name="slot"/>
      <arg type="u" direction="in" name="duration"/>
      <arg type="(iiii)" direction="out" name="frame"/>
    </method>
    <method name="DockLift">
      <arg type="t" direction="in" name="handle"/>
      <arg type="(iiii)" direction="in" name="slot"/>
      <arg type="(iiii)" direction="in" name="target"/>
      <arg type="u" direction="in" name="duration"/>
    </method>
    <method name="DockActivate">
      <arg type="t" direction="in" name="handle"/>
    </method>
    <method name="DockPanel">
      <arg type="u" direction="in" name="edge"/>
      <arg type="(iiii)" direction="in" name="panel"/>
      <arg type="(iiii)" direction="in" name="reserve"/>
      <arg type="a(t(iiii))" direction="in" name="slots"/>
      <arg type="(iiii)" direction="in" name="drop"/>
      <arg type="t" direction="in" name="active"/>
      <arg type="b" direction="in" name="highlight"/>
      <arg type="b" direction="in" name="numbered"/>
    </method>
    <method name="DockHidePanel"/>
    <method name="DockEvents">
      <arg type="at" direction="out" name="clicks"/>
      <arg type="a(tu)" direction="out" name="moves"/>
    </method>
    <method name="DockState">
      <arg type="t" direction="out" name="moving"/>
      <arg type="(ii)" direction="out" name="pointer"/>
      <arg type="b" direction="out" name="running"/>
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

// Windows that were already maximized when first shown: GNOME auto-maximizes a
// window that opens at nearly the size of the screen, and apps restore a saved
// maximized state. Tiling arranges these like restored windows; maximizing a
// window later still keeps it out of the layout.
const openedMaximized = new Set();

function maximizedOnOpen(window) {
    const id = window.get_id();
    if (!openedMaximized.has(id))
        return false;
    // Once restored, by tt-pane or the user, a later maximize is deliberate.
    if (!isMaximized(window)) {
        openedMaximized.delete(id);
        return false;
    }
    return true;
}

// Docked windows belong to the dock: tiling, centered groups and focus leave them alone.
let dock = null;

function isDocked(window) {
    return dock !== null && dock.marked.has(dockHandle(window));
}

// A restored application window tt-pane may move or focus without restoring it.
function isAvailable(window) {
    return isApplication(window) && isShowing(window) && !window.is_fullscreen() && !isDocked(window) &&
        (!isMaximized(window) || maximizedOnOpen(window));
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

// Windows on every workspace (including secondary monitors when workspaces
// span the primary display only) share one layout per monitor.
function workspaceKey(window, active) {
    return window.is_on_all_workspaces() ? 'gnome:all' : `gnome:${active.index()}`;
}

// A top-level, resizable, restored application window automatic tiling may arrange.
function isTileable(window) {
    return window.get_window_type() === Meta.WindowType.NORMAL &&
        !window.get_transient_for() && isAvailable(window) &&
        // Mutter reports maximized windows as not resizable, but only resizable
        // windows can be maximized.
        (window.allows_resize() || isMaximized(window));
}

// ---- Dock ----

const NO_RECT = [0, 0, 0, 0];
const SLOT_COLOR = '#3c3c42';
const ACCENT_COLOR = '#f2f0e8';
// Edges as tt-pane numbers them.
const EDGE_TOP = 0;
const EDGE_BOTTOM = 3;

// Meta.is_wayland_compositor went away with X11 sessions in GNOME 49.
const IS_X11 = typeof Meta.is_wayland_compositor === 'function' && !Meta.is_wayland_compositor();

// The handle tt-pane knows a window by in the dock: its X window ID on X11,
// which the rest of tt-pane uses there, or Mutter's window ID on Wayland.
function dockHandle(window) {
    if (!IS_X11)
        return window.get_id();
    // An X11 window's description starts with its client window ID, "0x1e00003".
    const xid = parseInt(window.get_description(), 16);
    return Number.isFinite(xid) ? xid : 0;
}

function findDocked(handle) {
    for (const actor of global.get_window_actors()) {
        const window = actor.get_meta_window();
        if (window && dockHandle(window) === handle)
            return window;
    }
    return null;
}

function hasArea(r) {
    return r[2] > 0 && r[3] > 0;
}

function contains(r, x, y) {
    return x >= r[0] && y >= r[1] && x < r[0] + r[2] && y < r[1] + r[3];
}

// The middle of `source` in the shape of a width × height slot (tt_pane_platform::dock::crop_to_aspect).
function cropToAspect(source, width, height) {
    const [x, y, sw, sh] = source;
    if (sw <= 0 || sh <= 0 || width <= 0 || height <= 0)
        return source;
    if (sw * height > width * sh) {
        const cropped = Math.max(1, Math.floor(sh * width / height));
        return [x + Math.floor((sw - cropped) / 2), y, cropped, sh];
    }
    const cropped = Math.max(1, Math.floor(sw * height / width));
    return [x, y + Math.floor((sh - cropped) / 2), sw, cropped];
}

// Where a clone of `window` goes inside a width × height box so the box shows
// the window's frame (not its shadows), cropped to the box's shape around its middle.
function cloneGeometry(window, width, height) {
    const frame = window.get_frame_rect();
    const buffer = window.get_buffer_rect();
    const inner = [frame.x - buffer.x, frame.y - buffer.y, frame.width, frame.height];
    const [cx, cy, cw] = cropToAspect(inner, width, height);
    const scale = cw > 0 ? width / cw : 1;
    return [-cx * scale, -cy * scale, buffer.width * scale, buffer.height * scale];
}

// The slot nearest `x`, `y` along the edge (tt_pane_platform::dock::nearest_slot).
function nearestSlot(rects, edge, x, y) {
    const horizontal = edge === EDGE_TOP || edge === EDGE_BOTTOM;
    let best = -1;
    let bestDistance = Infinity;
    rects.forEach((r, index) => {
        const distance = horizontal ? Math.abs(r[0] + r[2] / 2 - x) : Math.abs(r[1] + r[3] / 2 - y);
        if (distance < bestDistance) {
            best = index;
            bestDistance = distance;
        }
    });
    return best;
}

function reorder(order, item, to) {
    const from = order.indexOf(item);
    if (from < 0)
        return;
    order.splice(from, 1);
    order.splice(Math.min(to, order.length), 0, item);
}

class Dock {
    constructor() {
        // Docked windows by handle: the window and where it was when docked,
        // for putting it back if tt-pane goes away without undocking it.
        this.marked = new Map();
        // Windows parked out of sight, by handle: their actor, hidden, and the
        // handler that hides it again when Mutter shows it.
        this.hidden = new Map();
        this.panel = null;
        this.strut = null;
        this.dropSlot = null;
        this.frame = null;
        this.previews = new Map();
        this.layout = null;
        this.members = [];
        this.shown = [];
        this.clicks = [];
        this.moves = [];
        this.pressed = null;
        this.reorderTo = null;
        this.glides = new Set();
        this.moving = 0;
        this.owner = null;
        this.watch = 0;
    }

    destroy() {
        this.release();
        for (const box of this.glides)
            box.destroy();
        this.glides.clear();
        this.unwatch();
    }

    // The tray that shows the panel. When it goes away without cleaning up,
    // such as after a crash, docked windows come back where they were.
    own(sender) {
        if (!sender || sender === this.owner)
            return;
        this.unwatch();
        this.owner = sender;
        this.watch = Gio.bus_watch_name_on_connection(
            Gio.DBus.session, sender, Gio.BusNameWatcherFlags.NONE, null,
            () => {
                this.unwatch();
                this.release();
            });
    }

    unwatch() {
        if (this.watch)
            Gio.bus_unwatch_name(this.watch);
        this.watch = 0;
        this.owner = null;
    }

    // Put every docked window back where it was when docked, and remove the panel.
    release() {
        for (const [handle, entry] of this.marked) {
            if (findDocked(handle) === entry.window)
                this.place(entry.window, entry.restore, false);
        }
        this.marked.clear();
        for (const handle of [...this.hidden.keys()]) {
            const window = findDocked(handle);
            if (window)
                this.setHidden(window, handle, false);
        }
        this.hidden.clear();
        this.hidePanel();
    }

    mark(handle, docked) {
        const window = findDocked(handle);
        if (!docked) {
            this.marked.delete(handle);
            if (window)
                this.setHidden(window, handle, false);
            return;
        }
        if (!window)
            throw new Error('The window to dock is no longer open');
        // Docked again, as after GNOME Shell restarts: keep where it came from.
        if (this.marked.get(handle)?.window === window)
            return;
        this.marked.set(handle, {window, restore: rect(window.get_frame_rect())});
    }

    // A hidden actor isn't drawn, takes no clicks on Wayland (on X11 tt-pane
    // clears the window's input shape), and, unlike a transparent one, doesn't
    // cut the desktop out from under it. Its clones still show it, live.
    setHidden(window, handle, hidden) {
        const actor = window.get_compositor_private();
        const entry = this.hidden.get(handle);
        if (hidden) {
            if (actor && entry?.actor !== actor) {
                if (entry)
                    entry.actor.disconnect(entry.id);
                // Mutter shows a window's actor again when the window is
                // mapped or changes workspace.
                const id = actor.connect('notify::visible', () => {
                    if (actor.visible && this.hidden.get(handle)?.actor === actor)
                        actor.hide();
                });
                this.hidden.set(handle, {actor, id});
            }
            actor?.hide();
            // Below everything else, so whatever corner Mutter keeps on screen
            // never covers another window. A window already hidden stays where
            // it is: one being brought out has just been raised and focused.
            if (!entry)
                window.lower();
        } else if (entry) {
            this.hidden.delete(handle);
            entry.actor.disconnect(entry.id);
            entry.actor.show();
        }
    }

    // Mutter keeps part of every window on screen, so a parked window still has
    // a corner there; it is hidden and lets clicks through instead.
    place(window, [x, y, width, height], hidden) {
        if (window.is_fullscreen())
            window.unmake_fullscreen();
        unmaximize(window);
        window.move_resize_frame(true, x, y, width, height);
        this.setHidden(window, dockHandle(window), hidden);
        return rect(window.get_frame_rect());
    }

    // A live stand-in for `window` that glides from one rectangle to another,
    // under the panel, then calls `done` with it.
    glide(window, from, to, duration, done) {
        const actor = window.get_compositor_private();
        if (!actor) {
            done(null);
            return;
        }
        const box = new St.Widget({
            clip_to_allocation: true,
            x: from[0], y: from[1], width: from[2], height: from[3],
        });
        const clone = new Clutter.Clone({source: actor});
        const [sx, sy, sw, sh] = cloneGeometry(window, from[2], from[3]);
        clone.set_position(sx, sy);
        clone.set_size(sw, sh);
        box.add_child(clone);
        const group = Main.layoutManager.uiGroup;
        if (this.panel && this.panel.get_parent() === group)
            group.insert_child_below(box, this.panel);
        else
            group.add_child(box);
        this.glides.add(box);
        const [ex, ey, ew, eh] = cloneGeometry(window, to[2], to[3]);
        const mode = Clutter.AnimationMode.EASE_OUT_CUBIC;
        clone.ease({x: ex, y: ey, width: ew, height: eh, duration, mode});
        box.ease({
            x: to[0], y: to[1], width: to[2], height: to[3], duration, mode,
            onComplete: () => done(box),
        });
    }

    finishGlide(box) {
        if (box && this.glides.delete(box))
            box.destroy();
    }

    // Send a window back to the dock: park it, and glide a stand-in from where
    // it was into its preview slot.
    stow(window, park, slot, duration) {
        const from = rect(window.get_frame_rect());
        const parked = this.place(window, park, true);
        if (duration > 0 && hasArea(slot))
            this.glide(window, from, slot, duration, box => this.finishGlide(box));
        return parked;
    }

    // Bring a docked window out: a stand-in grows from its preview slot to
    // `target`, then the window takes its place there once it has its new size.
    lift(window, slot, target, duration) {
        const handle = dockHandle(window);
        if (duration <= 0 || !hasArea(slot)) {
            this.place(window, target, false);
            return;
        }
        this.glide(window, slot, target, duration, box => {
            if (findDocked(handle) !== window) {
                this.finishGlide(box);
                return;
            }
            this.place(window, target, true);
            let waited = 0;
            GLib.timeout_add(GLib.PRIORITY_DEFAULT, 10, () => {
                waited += 10;
                const frame = window.get_frame_rect();
                const sized = Math.abs(frame.width - target[2]) <= 2 &&
                    Math.abs(frame.height - target[3]) <= 2;
                if (!sized && waited < 200 && findDocked(handle) === window)
                    return GLib.SOURCE_CONTINUE;
                if (findDocked(handle) === window)
                    this.setHidden(window, handle, false);
                this.finishGlide(box);
                return GLib.SOURCE_REMOVE;
            });
        });
    }

    showPanel(edge, panel, reserve, slots, drop, active, highlight, numbered) {
        if (!this.panel) {
            this.panel = new St.Widget({reactive: true, name: 'tt-pane-dock'});
            this.panel.connect('button-press-event', (_actor, event) => this.press(event));
            this.panel.connect('motion-event', (_actor, event) => this.motion(event));
            this.panel.connect('button-release-event', (_actor, event) => this.releaseButton(event));
            this.panel.connect('leave-event', () => this.cancelDrag());
            this.dropSlot = new St.Widget({visible: false});
            this.frame = new St.Widget({
                visible: false,
                style: `border: 4px solid ${ACCENT_COLOR};`,
            });
            this.panel.add_child(this.dropSlot);
            this.panel.add_child(this.frame);
            Main.layoutManager.addChrome(this.panel, {affectsInputRegion: true, trackFullscreen: true});
        }
        this.panel.set_position(panel[0], panel[1]);
        this.panel.set_size(panel[2], panel[3]);
        // GNOME reserves a chrome actor's whole rectangle; tt-pane asks for
        // the full strip only once the docked windows fill the edge.
        if (hasArea(reserve)) {
            if (!this.strut) {
                this.strut = new St.Widget({reactive: false});
                Main.layoutManager.addChrome(this.strut, {affectsStruts: true, affectsInputRegion: false});
            }
            this.strut.set_position(reserve[0], reserve[1]);
            this.strut.set_size(reserve[2], reserve[3]);
        } else if (this.strut) {
            Main.layoutManager.removeChrome(this.strut);
            this.strut.destroy();
            this.strut = null;
        }
        this.layout = {edge, panel, slots: slots.map(([, r]) => r), drop, active, highlight, numbered};
        this.members = slots.map(([handle]) => handle);
        if (this.pressed && !this.members.includes(this.pressed.handle))
            this.cancelDrag();
        this.redraw();
    }

    hidePanel() {
        for (const preview of this.previews.values())
            this.dropPreview(preview);
        this.previews.clear();
        if (this.panel) {
            Main.layoutManager.removeChrome(this.panel);
            this.panel.destroy();
            this.panel = null;
        }
        if (this.strut) {
            Main.layoutManager.removeChrome(this.strut);
            this.strut.destroy();
            this.strut = null;
        }
        this.layout = null;
        this.members = [];
        this.shown = [];
        this.pressed = null;
        this.reorderTo = null;
    }

    dropPreview(preview) {
        if (preview.window && preview.sizeId)
            preview.window.disconnect(preview.sizeId);
        preview.box.destroy();
    }

    preview(handle) {
        const window = findDocked(handle);
        const actor = window?.get_compositor_private();
        let preview = this.previews.get(handle);
        if (preview && preview.window === window)
            return preview;
        if (preview) {
            this.dropPreview(preview);
            this.previews.delete(handle);
        }
        if (!actor)
            return null;
        const box = new St.Widget({
            clip_to_allocation: true,
            style: `background-color: ${SLOT_COLOR};`,
        });
        const clone = new Clutter.Clone({source: actor});
        const outline = new St.Widget({visible: false, style: `border: 3px solid ${ACCENT_COLOR};`});
        const label = new St.Label({
            x_align: Clutter.ActorAlign.CENTER,
            y_align: Clutter.ActorAlign.CENTER,
        });
        const number = new St.Bin({child: label, visible: false});
        box.add_child(clone);
        box.add_child(outline);
        box.add_child(number);
        preview = {window, box, clone, outline, number, label, size: [0, 0]};
        // A window that resizes (brought out, or sent back) is recropped.
        preview.sizeId = window.connect('size-changed', () => this.fit(preview));
        this.panel.insert_child_below(box, this.dropSlot);
        this.previews.set(handle, preview);
        return preview;
    }

    fit(preview) {
        const [width, height] = preview.size;
        if (width <= 0 || height <= 0)
            return;
        const [x, y, w, h] = cloneGeometry(preview.window, width, height);
        preview.clone.set_position(x, y);
        preview.clone.set_size(w, h);
    }

    redraw() {
        if (!this.panel || !this.layout)
            return;
        const {panel, slots, drop, active, highlight, numbered} = this.layout;
        const order = this.members.slice();
        const dragged = this.pressed && this.reorderTo !== null ? this.pressed.handle : 0;
        if (dragged)
            reorder(order, dragged, this.reorderTo);
        this.shown = order;
        const local = r => [r[0] - panel[0], r[1] - panel[1], r[2], r[3]];
        order.forEach((handle, index) => {
            const preview = this.preview(handle);
            const slot = slots[index];
            if (!preview || !slot)
                return;
            const [x, y, width, height] = local(slot);
            preview.box.set_position(x, y);
            preview.box.set_size(width, height);
            preview.size = [width, height];
            this.fit(preview);
            preview.outline.set_size(width, height);
            preview.outline.visible = handle === (dragged || active);
            preview.number.set_size(width, height);
            preview.number.visible = numbered && index < 9;
            if (preview.number.visible) {
                const size = Math.max(18, Math.min(120, Math.floor(height * 2 / 5)));
                preview.label.text = String(index + 1);
                preview.label.style = `font-size: ${size}px; font-weight: bold; color: white; ` +
                    'text-shadow: 0 0 4px black;';
            }
        });
        for (const [handle, preview] of this.previews) {
            if (!order.includes(handle)) {
                this.dropPreview(preview);
                this.previews.delete(handle);
            }
        }
        this.dropSlot.visible = hasArea(drop);
        if (this.dropSlot.visible) {
            const [x, y, width, height] = local(drop);
            this.dropSlot.set_position(x, y);
            this.dropSlot.set_size(width, height);
            this.dropSlot.style = `border: 2px solid ${highlight ? ACCENT_COLOR : SLOT_COLOR};`;
        }
        this.frame.visible = highlight;
        this.frame.set_size(panel[2], panel[3]);
    }

    press(event) {
        if (event.get_button() !== Clutter.BUTTON_PRIMARY || !this.layout)
            return Clutter.EVENT_PROPAGATE;
        const [x, y] = event.get_coords();
        const index = this.layout.slots.findIndex(r => contains(r, x, y));
        if (index >= 0 && index < this.shown.length) {
            this.pressed = {handle: this.shown[index], x, y};
            this.reorderTo = null;
        }
        return Clutter.EVENT_STOP;
    }

    // Past the drag threshold, a pressed preview takes the place nearest the
    // pointer, the others making room for it.
    motion(event) {
        if (!this.pressed || !this.layout)
            return Clutter.EVENT_PROPAGATE;
        const [x, y] = event.get_coords();
        const threshold = Clutter.Settings.get_default().dnd_drag_threshold;
        if (this.reorderTo === null &&
            Math.abs(x - this.pressed.x) <= threshold && Math.abs(y - this.pressed.y) <= threshold)
            return Clutter.EVENT_STOP;
        const slots = this.layout.slots.slice(0, this.members.length);
        const to = nearestSlot(slots, this.layout.edge, x, y);
        if (to >= 0 && to !== this.reorderTo) {
            this.reorderTo = to;
            this.redraw();
        }
        return Clutter.EVENT_STOP;
    }

    releaseButton(event) {
        if (event.get_button() !== Clutter.BUTTON_PRIMARY || !this.pressed)
            return Clutter.EVENT_PROPAGATE;
        const [x, y] = event.get_coords();
        const {handle} = this.pressed;
        const to = this.reorderTo;
        this.pressed = null;
        this.reorderTo = null;
        if (to !== null) {
            // Dropped on the panel: the dock takes the new order, shown until
            // tt-pane's next layout confirms it.
            if (contains(this.layout.panel, x, y)) {
                this.moves.push([handle, to]);
                reorder(this.members, handle, to);
            }
        } else {
            const index = this.layout.slots.findIndex(r => contains(r, x, y));
            if (index >= 0 && index < this.shown.length)
                this.clicks.push(this.shown[index]);
        }
        this.redraw();
        return Clutter.EVENT_STOP;
    }

    cancelDrag() {
        if (!this.pressed)
            return;
        const dragging = this.reorderTo !== null;
        this.pressed = null;
        this.reorderTo = null;
        if (dragging)
            this.redraw();
    }
}

function dockWindow(handle) {
    const window = findDocked(handle);
    if (!window)
        throw new Error('Window is no longer available');
    return window;
}

class Service {
    constructor() {
        this.grabbing = false;
    }

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
        openedMaximized.delete(id);
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
            .filter(window => window && isShowing(window) && !isDocked(window));
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

    // Tileable windows on the active workspace, every application window there
    // (minimized ones included, so floating choices survive a restore), the
    // workspace keys in view, and whether the user is dragging or resizing.
    ListTilingWindows() {
        const active = global.workspace_manager.get_active_workspace();
        const windows = global.get_window_actors()
            .map(actor => actor.get_meta_window())
            .filter(window => window && isApplication(window) &&
                window.located_on_workspace(active));
        return [
            windows.filter(isTileable).map(window => [
                window.get_id(),
                rect(window.get_frame_rect()),
                rect(window.get_work_area_current_monitor()),
                workspaceKey(window, active),
                window.get_wm_class() ?? '',
                Math.max(0, window.get_pid()),
            ]),
            windows.map(window => window.get_id()),
            [`gnome:${active.index()}`, 'gnome:all'],
            this.grabbing,
        ];
    }

    // The primary monitor the dock sits on, its work area, and every monitor.
    DockMonitor() {
        const layout = Main.layoutManager;
        const monitor = layout.primaryMonitor;
        if (!monitor)
            throw new Error('No primary monitor');
        const work = global.workspace_manager.get_active_workspace()
            .get_work_area_for_monitor(layout.primaryIndex);
        return [
            [monitor.x, monitor.y, monitor.width, monitor.height],
            rect(work),
            layout.monitors.map(m => [m.x, m.y, m.width, m.height]),
        ];
    }

    // The dock's monitor and work area, and whether the panel is shown, for
    // the tt-pane tray that shows the dock. The caller becomes the dock's
    // owner: if it goes away, the docked windows come back.
    DockHoldAsync(_params, invocation) {
        try {
            dock.own(invocation.get_sender());
            const [monitor, work] = this.DockMonitor();
            invocation.return_value(new GLib.Variant('((iiii)(iiii)b)', [monitor, work, dock.panel !== null]));
        } catch (error) {
            invocation.return_error_literal(Gio.DBusError, Gio.DBusError.FAILED, String(error));
        }
    }

    DockWindow(handle) {
        const window = findDocked(handle);
        if (!window)
            return [false, NO_RECT, false];
        return [true, rect(window.get_frame_rect()), window.minimized];
    }

    DockMark(handle, docked) {
        dock.mark(handle, docked);
    }

    DockPlace(handle, target, hidden) {
        return dock.place(dockWindow(handle), target, hidden);
    }

    DockStow(handle, park, slot, duration) {
        return dock.stow(dockWindow(handle), park, slot, duration);
    }

    DockLift(handle, slot, target, duration) {
        dock.lift(dockWindow(handle), slot, target, duration);
    }

    DockActivate(handle) {
        Main.activateWindow(dockWindow(handle));
    }

    DockPanel(edge, panel, reserve, slots, drop, active, highlight, numbered) {
        dock.showPanel(edge, panel, reserve, slots, drop, active, highlight, numbered);
    }

    DockHidePanel() {
        dock.hidePanel();
    }

    // Previews clicked and moved since the last call.
    DockEvents() {
        const events = [dock.clicks, dock.moves];
        dock.clicks = [];
        dock.moves = [];
        return events;
    }

    // The window the user is moving with the mouse, the pointer, and whether a
    // tt-pane tray shows the dock.
    DockState() {
        const [x, y] = global.get_pointer();
        return [dock.moving, [x, y], dock.owner !== null];
    }

    get Version() {
        return API_VERSION;
    }
}

export default class TtPaneExtension extends Extension {
    enable() {
        this._service = new Service();
        dock = new Dock();
        const moves = [Meta.GrabOp.MOVING, Meta.GrabOp.KEYBOARD_MOVING, Meta.GrabOp.MOVING_UNCONSTRAINED]
            .filter(op => op !== undefined);
        this._grabs = [
            global.display.connect('grab-op-begin', (_display, window, op) => {
                this._service.grabbing = true;
                // A window dragged with the mouse can be dropped on the dock.
                dock.moving = window && moves.includes(op) ? dockHandle(window) : 0;
            }),
            global.display.connect('grab-op-end', () => {
                this._service.grabbing = false;
                dock.moving = 0;
            }),
            // GNOME applies auto-maximize while placing a window, before it is shown.
            global.display.connect('window-created', (_display, window) => {
                const id = window.get_id();
                const forget = () => {
                    window.disconnectObject(this);
                    this._tracked.delete(window);
                    openedMaximized.delete(id);
                };
                this._tracked.add(window);
                window.connectObject('shown', () => {
                    window.disconnectObject(this);
                    if (!isMaximized(window)) {
                        this._tracked.delete(window);
                        return;
                    }
                    openedMaximized.add(id);
                    window.connectObject('unmanaged', forget, this);
                }, 'unmanaged', forget, this);
            }),
        ];
        this._tracked = new Set();
        this._dbus = Gio.DBusExportedObject.wrapJSObject(INTERFACE, this._service);
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
        // Docked windows come back on screen, where they were.
        dock?.destroy();
        dock = null;
        for (const window of this._tracked ?? [])
            window.disconnectObject(this);
        this._tracked = null;
        openedMaximized.clear();
        for (const id of this._grabs ?? [])
            global.display.disconnect(id);
        this._grabs = null;
        this._service = null;
    }
}
