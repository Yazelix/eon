import os
import socket
import subprocess
import sys
import time

SOCKET = "/run/eon-boat-observer/kernel.sock"
LIMIT = 4 * 1024 * 1024
WAIT = 5


def socket_sample():
    deadline = time.monotonic() + WAIT
    with socket.socket(socket.AF_UNIX) as reader:
        reader.settimeout(WAIT)
        reader.connect(SOCKET)
        sample = bytearray()
        while len(sample) <= LIMIT:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError
            reader.settimeout(remaining)
            block = reader.recv(min(65536, LIMIT + 2 - len(sample)))
            if not block:
                break
            sample.extend(block)
        if not sample.endswith(b"\0") or b"\0" in sample[:-1]:
            raise ValueError
        return sample[:-1]


def direct_sample():
    command = ["/usr/bin/timeout", "--kill-after=1s", "4s", "/usr/bin/dmesg", "--raw"]
    with subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                          stderr=subprocess.DEVNULL) as process:
        data = process.stdout.read(LIMIT + 1)
        if len(data) > LIMIT:
            process.stdout.close()
        if process.wait(timeout=WAIT) != 0:
            raise ValueError
    return data


if __name__ == "__main__":
    try:
        data = socket_sample() if os.path.lexists(SOCKET) else direct_sample()
        if not data or len(data) > LIMIT or not data.endswith(b"\n") or b"\0" in data:
            raise ValueError
        sys.stdout.buffer.write(data)
    except (OSError, ValueError, subprocess.SubprocessError):
        sys.exit("kernel sample unavailable or incomplete")
