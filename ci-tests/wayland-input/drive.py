"""CI-only virtual input for a disposable Mutter 46 process on a private bus.

Mutter's private test/control D-Bus API is deliberately confined to this fixture;
the production client uses only public Wayland protocols and never invokes it.
No host account, profile, clipboard or network connection is used.
"""
import json
import sys
from gi.repository import Gio, GLib

bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
name = "org.gnome.Mutter.RemoteDesktop"
path = "/org/gnome/Mutter/RemoteDesktop"

def call(path, interface, method, args=None):
    return bus.call_sync(name, path, interface, method, args, None,
                         Gio.DBusCallFlags.NONE, 5000, None)

session = call(path, name, "CreateSession").unpack()[0]
interface = name + ".Session"
call(session, interface, "Start")
print("ready", flush=True)
try:
    for line in sys.stdin:
        cmd = json.loads(line)
        if cmd[0] == "move":
            call(session, interface, "NotifyPointerMotionRelative",
                 GLib.Variant("(dd)", (cmd[1], cmd[2])))
        elif cmd[0] == "button":
            call(session, interface, "NotifyPointerButton",
                 GLib.Variant("(ib)", (272, cmd[1])))
        elif cmd[0] == "escape":
            for pressed in (True, False):
                call(session, interface, "NotifyKeyboardKeysym",
                     GLib.Variant("(ub)", (65307, pressed)))
        elif cmd[0] == "quit":
            break
        else:
            raise ValueError("Unknown fixture input command")
        print("ok", flush=True)
finally:
    call(session, interface, "Stop")
