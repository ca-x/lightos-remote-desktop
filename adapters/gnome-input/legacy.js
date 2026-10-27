const {Gio, GLib, Shell} = imports.gi;
const ByteArray = imports.byteArray;

class LightOSInputContext {
    enable() {
        const [, stat] = GLib.file_get_contents('/proc/self/stat');
        this._pid = Number(ByteArray.toString(stat).split(' ')[0]);
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
        this._file.replace_contents(ByteArray.fromString(value), null, false,
            Gio.FileCreateFlags.REPLACE_DESTINATION, null);
    }

    disable() {
        if (this._signal) global.display.disconnect(this._signal);
        this._signal = null;
        try { this._file?.delete(null); } catch (_) { /* Session already removed. */ }
        this._file = null;
    }
}

function init() { return new LightOSInputContext(); }
