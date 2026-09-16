#!/usr/bin/python3
"""{{name}}: a desktop app with GTK 4 (python3-gobject, in the Genesis image).

A window with a header bar, an entry, an "Add" button and a list; items are kept in
~/.local/share/{{name}}/items.json. Extend `on_add` and the list row for your own thing.
Run: python3 app.py
"""
import json, os, sys

import gi
gi.require_version("Gtk", "4.0")
from gi.repository import Gtk, Gio, GLib  # noqa: E402

APP_ID = "org.genesis.made.{{name}}"
DATA_DIR = os.path.join(GLib.get_user_data_dir(), "{{name}}")
DATA = os.path.join(DATA_DIR, "items.json")


def load():
    try:
        return json.load(open(DATA))
    except (OSError, ValueError):
        return []


def save(items):
    os.makedirs(DATA_DIR, exist_ok=True)
    tmp = DATA + ".tmp"
    with open(tmp, "w") as f:
        json.dump(items, f, indent=1)
    os.replace(tmp, DATA)


class Window(Gtk.ApplicationWindow):
    def __init__(self, app):
        super().__init__(application=app, title="{{name}}", default_width=520, default_height=420)
        header = Gtk.HeaderBar()
        self.set_titlebar(header)
        clear = Gtk.Button(icon_name="edit-clear-all-symbolic", tooltip_text="Remove everything")
        clear.connect("clicked", self.on_clear)
        header.pack_end(clear)

        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8, margin_top=12, margin_bottom=12, margin_start=12, margin_end=12)
        self.set_child(box)
        row = Gtk.Box(spacing=8)
        self.entry = Gtk.Entry(placeholder_text="Something to remember", hexpand=True)
        self.entry.connect("activate", self.on_add)
        add = Gtk.Button(label="Add")
        add.add_css_class("suggested-action")
        add.connect("clicked", self.on_add)
        row.append(self.entry)
        row.append(add)
        box.append(row)

        self.list = Gtk.ListBox(selection_mode=Gtk.SelectionMode.NONE)
        self.list.add_css_class("boxed-list")
        scroller = Gtk.ScrolledWindow(vexpand=True, child=self.list)
        box.append(scroller)
        self.status = Gtk.Label(xalign=0)
        self.status.add_css_class("dim-label")
        box.append(self.status)

        self.items = load()
        self.refresh()

    def refresh(self):
        while child := self.list.get_first_child():
            self.list.remove(child)
        for i, item in enumerate(self.items):
            row = Gtk.Box(spacing=8, margin_top=6, margin_bottom=6, margin_start=8, margin_end=8)
            label = Gtk.Label(label=item["text"], xalign=0, hexpand=True, wrap=True)
            remove = Gtk.Button(icon_name="user-trash-symbolic", tooltip_text="Remove")
            remove.add_css_class("flat")
            remove.connect("clicked", self.on_remove, i)
            row.append(label)
            row.append(remove)
            self.list.append(row)
        n = len(self.items)
        self.status.set_text(f"{n} item{'s' if n != 1 else ''}")

    def on_add(self, *_):
        text = self.entry.get_text().strip()
        if not text:
            return
        self.items.append({"text": text})
        save(self.items)
        self.entry.set_text("")
        self.refresh()

    def on_remove(self, _button, index):
        del self.items[index]
        save(self.items)
        self.refresh()

    def on_clear(self, *_):
        self.items = []
        save(self.items)
        self.refresh()


class App(Gtk.Application):
    def __init__(self):
        super().__init__(application_id=APP_ID, flags=Gio.ApplicationFlags.DEFAULT_FLAGS)

    def do_activate(self):
        win = self.props.active_window or Window(self)
        win.present()


if __name__ == "__main__":
    sys.exit(App().run(sys.argv))
