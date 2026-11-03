# rofication-rs

A Rust port of [regolith-rofication](https://github.com/regolith-linux/regolith-rofication),
Regolith's notification system: `rofication-daemon` (the `org.freedesktop.Notifications` D-Bus
service and its Unix-socket server), `rofication-gui` (the rofi viewer) and `rofication-status`
(the i3xrocks status block).

## Status

Experimental port; not a drop-in replacement yet.

- **Snapshot.** The port reproduces regolith-rofication as of
  [`2e0908e`](https://github.com/regolith-linux/regolith-rofication/commit/2e0908ea658f7b9672270f899345b76a07b1bcf5)
  (2026-10-30: release 1.5.3 plus fixes #25 to #33), with the two fixes listed under "What is
  different". It is feature-complete for that snapshot and was verified against a build of the
  original (see "How it was verified").
- **Fixes still to port.** Any change merged into regolith-rofication after `2e0908e` has to be
  ported. None is merged as of 2026-11-03. Known open work on the original that the port does
  not include (this list may not be complete):
  - The open pull request #18 (show the oldest notifications first, configurable).
  - The two fixes under "What is different" are not in the original yet. If regolith-rofication
    fixes them differently, the port should follow.
- **Testing.** Very deep testing is needed before anyone considers it as a replacement for the
  original.
- **Reviews.** It has not yet had deep code reviews by the major AI coding agents and reviewers.
  Those reviews are planned before any replacement decision.
- **Upstreaming** into regolith-linux is undecided.

## What is identical

The port follows the Python implementation at `2e0908e` byte for byte where it can be observed:
D-Bus replies, capabilities, server information and introspection (`Notify` takes
`susssasa{sv}i`), `NotificationClosed` and `ActionInvoked` signals and their order, ids
(including restored counters and ids above 2^32), replacement and the single-notification
application list, urgency handling, expiry, the socket protocol (`num`, `list`, `del`, `delm`,
`dela`, `see`) and its reply bytes, the list document (Python `json` output, every hint type,
`image-data`), the saved queue and counter files under `~/.cache/rofication/`, saving on SIGTERM,
SIGINT and SIGHUP, the rofi command line and rows, the status block output, and the Xresource and
environment lookups (`xrescat`, then `trawlcat`; reversed on Wayland).

The package keeps its name (`regolith-rofication`), its three binaries in `/usr/bin/`, and
Provides/Replaces `notification-daemon`, Conflicts `notify-osd, dunst`. Like the original, it
ships no systemd or D-Bus activation unit: Regolith's window manager configuration starts
`/usr/bin/rofication-daemon`.

## What is different

- **Dismissing by application name.** `dela:<application>` matches the whole name after the
  first colon. The original split the request at every colon, so dismissing `Chat: Work`
  dismissed `Chat` instead. This affects `lago rm "<app>"`, ilia's notification view and the
  viewer's "dismiss all of this application" key.
- **Hints in the viewer.** `rofication-gui` checks only the listed notifications. The original
  decoded every JSON object in the list as a notification, nested hint dictionaries included, so
  a hint dictionary carrying an `urgency` or `actions` key it did not accept stopped the viewer.
- **No Python.** The package no longer installs the `rofication` Python module or depends on
  python3-gi and python3-dbus. Nothing else in Regolith imports that module (checked:
  regolith-wm-config, regolith-i3xrocks-config, regolith-session, i3status-rust, ilia).
- **Error output.** Where the original exits with a Python traceback, the port prints a one-line
  error. Exit statuses are the same.
- **Concurrent socket requests** are applied in the order the clients connected (a client that
  stalls for 200 ms lets the next one go first).

## Building

```
cargo build --release --locked     # target/release/rofication-{daemon,gui,status}
cargo test --locked                # unit and socket tests
dpkg-buildpackage -b -us -uc       # the regolith-rofication package
```

The locked crates (zbus 5, serde, signal-hook, chrono) need Rust 1.89 or newer. Ubuntu 26.04's
`cargo` is new enough. On Ubuntu 24.04, install `cargo-1.89 rustc-1.89` (or newer); `debian/rules`
picks the newest versioned toolchain it finds. The build downloads the crates from crates.io; for
an offline build, run `cargo vendor` and add the `[source]` configuration it prints to
`.cargo/config.toml` (or to `$CARGO_HOME/config.toml`).

## How it was verified

- `cargo test`: 15 unit tests and 2 socket tests, including dismissing `Chat: Work` next to
  `Chat` and a list that keeps nested hints intact.
- A differential suite (292 checks) that runs every scenario against a compiled build of the
  original and against the port, each with a private D-Bus session, and compares D-Bus replies
  and signals, socket replies, lago and rofi transcripts, exit statuses and the saved files byte
  for byte: D-Bus, signals, socket, listing, order, persistence, viewer, status block, resource
  lookups, lago, 30 seeded random sequences, and the Debian build. 289 of the 292 checks pass;
  the other three are the `dela` cases with a colon in the name, where the original dismisses
  the wrong application (see "What is different").
- `dpkg-buildpackage -b` on Ubuntu 26.04 and on 24.04 (with `cargo-1.89`); the package installs
  on both.
- An installed-package session test in headless sway on Ubuntu 26.04: sway starts the daemon as
  Regolith's configuration does; `notify-send` and raw D-Bus calls (including a 128x128
  `image-data` hint, a transient hint and a nested hint dictionary); lago (lago-rs) `ls`,
  `rm <id>`, `rm "Chat: Work"`, `clear` with 16 notifications; `rofication-status`; `num` as the
  i3xrocks script and i3status-rust send it; the real rofi viewer (dismiss with Delete, close
  with Escape); SIGTERM saves the queue and a restart restores it without the transient entry.

## License

MIT, like the original. See `LICENSE`.

# About Rofication


**Rofication** is a minimalistic notification system. It is loosely modeled after notification
centers on Android and Windows 10. The main idea is to not annoy (e.g. overlap parts of my
desktop) but still keep a persistent list of messages. Notifications are useful, however too many
notifications showing up too often (e.g. work mail popups) distract from the current task and are
therefore dismissed without proper reviewing. The goal is to show (in a single clear place) there
are notifications but not pop them up. It is up to the user to review the notifications when time
is available. If things are critical one should use a more direct media (e.g. face to face or phone). 

To summarize **Rofications**, it has:

 * 1 point to see if there are notifications. For example the status bar of your Desktop
   Environment.
![Notification Bar](https://raw.githubusercontent.com/DaveDavenport/Rofication/master/Picture/bar.png)
 * An application to (re)view all open notifications.
![Notification Bar](https://raw.githubusercontent.com/DaveDavenport/Rofication/master/Picture/client.png)
 * Persistent notifications, they stay around until dismissed. They should stick around, even over reboots.

# Techniques used

**Rofication** implements a notification daemon following the [Galago Desktop Notification
standard](http://www.galago-project.org/specs/notification/).  This is used by most Linux desktop
environments and most relevant applications support this. It is easily scriptable using
`notify-send`.  **Rofication** aims to be a drop-in replacement for existing notification daemons.

**Rofication** tries to re-use existing code as much as possible. It uses **Rofi** for displaying,
i3blocks for the unintrusive notification and Rust (with `zbus`) for the `d-bus` implementation of
the notification daemon.

# Structure
The main structure is like depicted below. The daemon acts as a storage point for the dbus messages
generated by the applications. The unix-socket interface allows clients to query the storage. A
proper protocol still has to be defined.


```
┌─────────────┐           ┌───────────────────┐                      ┌────────────────┐
│ application │ --dbus--> │ rofication-daemon │ <--- unix-socket --> │ rofication-gui │
└─────────────┘           └───────────────────┘       │              └────────────────┘
                                |                     │             ┌──────────────────────────┐
                                |                     \-----------> │ rofication-statusi3blocks│
                            <hdd:json-db>                           └──────────────────────────┘
```

The components consist of:

## Daemon

**Rofication** daemon is a small Rust program that listens for notification messages on the local
dbus. Notifications are internally queued (and preserved) and can be viewed by a client via a
unix-socket. It is just a simple storage for notifications.

Some applications should only ever have one notification in the queue: a new notification from
them replaces the previous one. The daemon reads this list of application names (comma-separated)
at startup from the Xresource `rofication.notify.single_notification_apps`, or from the environment
variable `rofication_notify_single_notification_apps`. The default is `VLC media player`. Example:

```
rofication.notify.single_notification_apps: VLC media player, Spotify
```

## Notification

**Rofication** does not implement its own 'widget' to display notifications. Instead it can be
easily integrated into existing tools.  Currently we ship a small script to integrate into
[`i3-blocks`](https://github.com/vivien/i3blocks). This shows the number of notifications and
signals when there are critical notifications. Ideally there should be several small scripts that
implement this for different desktop environments.

## GUI

The **Rofication** GUI consists of a small Rust program wrapping **Rofi**. The GUI allows the user
to view notifications, mark them as seen and dismiss them.

The keys are passed to rofi as `-kb-*` options. Each one can be changed with the Xresource
`rofication.notify.<option>`, or the environment variable `rofication_notify_<option>` with `-`
replaced by `_` (for example `rofication_notify_kb_custom_1`). The values use rofi's key binding
syntax. A comma separates alternative keys, and an empty value unbinds the action.

| Xresource                                  | Action                                    | Default                        |
|--------------------------------------------|-------------------------------------------|--------------------------------|
| `rofication.notify.kb-custom-1`            | Dismiss the notification                  | `Delete`                       |
| `rofication.notify.kb-custom-2`            | Mark the notification as seen             | `Return`                       |
| `rofication.notify.kb-custom-3`            | Refresh the list                          | `Alt+r`                        |
| `rofication.notify.kb-custom-4`            | Dismiss all notifications of the app      | `Shift+Delete`                 |
| `rofication.notify.kb-accept-entry`        | rofi `kb-accept-entry` (close the viewer) | `Control+j,Control+m,KP_Enter` |
| `rofication.notify.kb-remove-char-forward` | rofi `kb-remove-char-forward`             | `Control+d`                    |
| `rofication.notify.kb-delete-entry`        | rofi `kb-delete-entry`                    | (none)                         |

The last three free `Return`, `Delete` and `Shift+Delete` from rofi's default bindings. A key
must be bound to only one rofi action: if you pick a key that your rofi configuration already uses
for another action (see `rofi -dump-config`), unbind it there. Example:

```
rofication.notify.kb-custom-1: Control+x
rofication.notify.kb-custom-2: Control+s,Return
```

## CLI

*This still needs to be written*

