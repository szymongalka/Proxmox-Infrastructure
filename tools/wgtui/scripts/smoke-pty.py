#!/usr/bin/env python3
"""Exercise actual terminal setup/teardown in demo mode; no screen log on disk."""
import fcntl
import os
import pathlib
import pty
import select
import signal
import struct
import sys
import termios
import time

root = pathlib.Path(__file__).resolve().parents[1]
binary = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else root / 'target/debug/wgtui'
command = 'wgtui_before=$(stty -g); "$1" --demo; wgtui_status=$?; wgtui_after=$(stty -g); [ "$wgtui_before" = "$wgtui_after" ] || exit 42; printf "\\nWGTUI_EXIT=%s\\n" "$wgtui_status"; exit "$wgtui_status"'
pid, master = pty.fork()
if pid == 0:
    os.execve('/bin/sh', ['/bin/sh', '-c', command, 'wgtui-smoke', str(binary)],
              {**os.environ, 'TERM': 'xterm-256color'})
fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack('HHHH', 32, 120, 0, 0))
screen = bytearray()
quit_sent = False
finished = False
deadline = time.monotonic() + 10
try:
    while time.monotonic() < deadline:
        if select.select([master], [], [], .1)[0]:
            try:
                chunk = os.read(master, 65536)
            except OSError:
                break
            if not chunk:
                break
            screen.extend(chunk)
        if not quit_sent and b'macbook-01' in screen:
            os.write(master, b'q')
            quit_sent = True
        child, status = os.waitpid(pid, os.WNOHANG)
        if child:
            finished = True
            assert os.waitstatus_to_exitcode(status) == 0, 'TUI or terminal restoration failed'
            break
    assert quit_sent, 'Missing actual TUI frame'
    assert b'WGTUI_EXIT=0' in screen, 'TUI did not exit or terminal was not restored'
    print('PTY smoke: rzeczywisty ekran i przywrócenie terminala OK')
finally:
    if not finished:
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        os.waitpid(pid, 0)
    screen.clear()
    os.close(master)
