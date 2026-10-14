// Wayland-Wheeltani Foreground - GNOME Shell extension (GNOME 45+, ESM).
//
// Publishes the currently focused window on the session bus so the
// Wayland-Wheeltani daemon's `gnome` provider can decide, per application,
// whether to run autoscroll. It exposes:
//
//   bus name : org.docloulou.WheeltaniForeground
//   object   : /org/docloulou/WheeltaniForeground
//   method   : GetFocused() -> s   (JSON of the focused window, or "{}")
//   signal   : FocusedChanged(s)   (same JSON, emitted when the focused
//                                   window or its title changes)
//
// The JSON shape is: {app_id, class, resource_class, title, pid}. All fields
// are optional. Nothing is persisted and no network is used.

import GLib from 'gi://GLib';
import Gio from 'gi://Gio';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const BUS_NAME = 'org.docloulou.WheeltaniForeground';
const OBJECT_PATH = '/org/docloulou/WheeltaniForeground';

const IFACE = `
<node>
  <interface name="org.docloulou.WheeltaniForeground">
    <method name="GetFocused">
      <arg type="s" direction="out" name="json"/>
    </method>
    <signal name="FocusedChanged">
      <arg type="s" name="json"/>
    </signal>
  </interface>
</node>`;

export default class WheeltaniForegroundExtension extends Extension {
    enable() {
        this._lastJson = null;
        this._dbusImpl = Gio.DBusExportedObject.wrapJSObject(IFACE, this);
        this._dbusImpl.export(Gio.DBus.session, OBJECT_PATH);
        this._ownerId = Gio.bus_own_name(
            Gio.BusType.SESSION,
            BUS_NAME,
            Gio.BusNameOwnerFlags.NONE,
            null,
            null,
            null);

        this._focusHandlerId = global.display.connect(
            'notify::focus-window',
            () => this._onFocusChanged());
        this._focusedWindow = null;
        this._titleHandlerId = null;

        // Publish the current window immediately so a consumer that connects
        // mid-session gets state without waiting for the next focus change.
        this._onFocusChanged();
    }

    disable() {
        if (this._focusHandlerId) {
            global.display.disconnect(this._focusHandlerId);
            this._focusHandlerId = null;
        }
        this._trackTitle(null);
        if (this._ownerId) {
            Gio.bus_unown_name(this._ownerId);
            this._ownerId = 0;
        }
        if (this._dbusImpl) {
            this._dbusImpl.unexport();
            this._dbusImpl = null;
        }
        this._lastJson = null;
    }

    // D-Bus method.
    GetFocused() {
        return this._describeFocused();
    }

    _describeFocused() {
        const win = global.display.focus_window;
        if (!win)
            return '{}';

        const obj = {};
        const appId = win.get_gtk_application_id?.();
        const wmClass = win.get_wm_class?.();
        const wmInstance = win.get_wm_class_instance?.();
        const title = win.get_title?.();
        const pid = win.get_pid?.();

        if (appId)
            obj.app_id = appId;
        if (wmClass)
            obj.class = wmClass;
        if (wmInstance)
            obj.resource_class = wmInstance;
        if (title)
            obj.title = title;
        if (typeof pid === 'number' && pid > 0)
            obj.pid = pid;

        return JSON.stringify(obj);
    }

    // Follows the focused window's title, so `match_title` rules see a tab
    // or document switch inside the same window.
    _trackTitle(win) {
        if (this._focusedWindow === win)
            return;
        if (this._focusedWindow && this._titleHandlerId)
            this._focusedWindow.disconnect(this._titleHandlerId);
        this._focusedWindow = win;
        this._titleHandlerId = win
            ? win.connect('notify::title', () => this._publish())
            : null;
    }

    _onFocusChanged() {
        this._trackTitle(global.display.focus_window);
        this._publish();
    }

    _publish() {
        const json = this._describeFocused();
        if (json === this._lastJson)
            return;
        this._lastJson = json;
        if (this._dbusImpl)
            this._dbusImpl.emit_signal('FocusedChanged', new GLib.Variant('(s)', [json]));
    }
}
