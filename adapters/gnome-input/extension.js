import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Shell from 'gi://Shell';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

// This adapter is enabled only in the service-owned GNOME session mode. It
// reports application identity so a paste never turns into terminal Ctrl+V.
export default class LightOSInputContext extends Extension {
    enable() {
        const [, stat] = GLib.file_get_contents('/proc/self/stat');
        this._pid = Number(new TextDecoder().decode(stat).split(' ')[0]);
        this._file = Gio.File.new_for_path(`${GLib.get_user_runtime_dir()}/lightos-focus.json`);
        this._signal = global.display.connect('notify::focus-window', () => this._update());
        this._update();
    }

    _update() {
        const window = global.display.focus_window;
        const app = window ? Shell.WindowTracker.get_default().get_window_app(window) : null;
        const value = JSON.stringify({
            compositor_pid: this._pid,
            app_id: (app?.get_id() ?? '').slice(0, 256),
            wm_class: (window?.get_wm_class() ?? '').slice(0, 256),
        });
        this._file.replace_contents(new TextEncoder().encode(value), null, false,
            Gio.FileCreateFlags.REPLACE_DESTINATION, null);
    }

    disable() {
        if (this._signal) global.display.disconnect(this._signal);
        this._signal = null;
        try { this._file?.delete(null); } catch (_) { /* Session already removed. */ }
        this._file = null;
    }
}
