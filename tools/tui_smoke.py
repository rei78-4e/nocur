"""Linux/macOS PTY integration check. Run after cargo build: python3 tools/tui_smoke.py."""
import fcntl
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import tempfile
import termios
import time


binary = Path(__file__).resolve().parents[1] / "target/debug/nocur"
with tempfile.TemporaryDirectory() as directory:
    source = Path(directory) / "input.txt"
    output = Path(str(source) + ".edited")
    source.write_text("foo\n")
    child, master = pty.fork()
    if child == 0:
        os.environ["TERM"] = "xterm-256color"
        os.execv(str(binary), [str(binary), str(source)])
    fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 100, 0, 0))

    def wait_for(needle):
        received = b""
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if select.select([master], [], [], 0.1)[0]:
                received += os.read(master, 65536)
                if needle.encode() in received:
                    return
        raise AssertionError(f"TUI did not render {needle!r}: {received!r}")

    def send(text, needle):
        os.write(master, text.encode())
        wait_for(needle)

    def paste(text):
        return "\x1b[200~" + text + "\x1b[201~"

    finished = False
    try:
        wait_for("Function Input")
        send(paste('replace(/foo/, "bar")'), "bar")
        send("\x13", "Committed and saved")
        assert output.read_text() == "bar\n"
        send("\x1a", "Undo")
        send("\x13", "Saved")
        assert output.read_text() == "foo\n"
        send("\x19", "Redo")
        send("\x13", "Saved")
        assert output.read_text() == "bar\n"
        send(paste('replace(/bar/, "baz")'), "baz")
        send("\x18", "Committed")  # Ctrl+X
        assert output.read_text() == "bar\n", "Ctrl+X saved without Ctrl+S"
        send("\x13", "Saved")
        assert output.read_text() == "baz\n"
        send(paste("replace("), "Preview [error]")
        send("\x1bOQ", "Commit blocked")
        send("\x13", "Commit blocked")
        assert output.read_text() == "baz\n"
        send("\x05", "Exported")
        script = Path(str(output) + ".transform").read_text()
        replay = subprocess.check_output([str(binary), str(source), "--eval", script])
        assert replay == b"baz\n"
        os.write(master, b"\x11")
        _, status = os.waitpid(child, 0)
        assert os.waitstatus_to_exitcode(status) == 0
        finished = True
        print("PASS: live preview, commit, undo/redo, invalid commit, save, export/replay, quit")
    finally:
        if not finished:
            os.kill(child, signal.SIGKILL)
            os.waitpid(child, 0)
        os.close(master)
