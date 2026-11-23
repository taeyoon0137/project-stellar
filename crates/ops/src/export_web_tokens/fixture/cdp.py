"""Chrome DevTools Protocol 최소 client(표준 라이브러리만, 개발 도구).

브라우저 기대값 fixture 생성에만 씁니다(XXII.2.3, XXII.2.6). test는 브라우저를 실행하지 않습니다.
media feature emulation(`Emulation.setEmulatedMedia`)과 pseudo-class 강제(`CSS.forcePseudoState`)가 필요해서
`--dump-dom` 대신 DevTools 연결을 씁니다.
"""

import base64
import json
import os
import shutil
import socket
import struct
import subprocess
import tempfile
import time
import urllib.parse
import urllib.request

DEFAULT_CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"


def find_chrome(explicit=None):
    return explicit or os.environ.get("STELLAR_CHROME") or DEFAULT_CHROME


def browser_version(chrome):
    out = subprocess.run([chrome, "--version"], check=True, capture_output=True, text=True)
    return out.stdout.strip()


class WebSocket:
    def __init__(self, url):
        parsed = urllib.parse.urlparse(url)
        self.sock = socket.create_connection((parsed.hostname, parsed.port), timeout=30)
        key = base64.b64encode(os.urandom(16)).decode()
        request = (
            f"GET {parsed.path} HTTP/1.1\r\nHost: {parsed.hostname}:{parsed.port}\r\n"
            "Upgrade: websocket\r\nConnection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
        self.sock.sendall(request.encode())
        response = b""
        while b"\r\n\r\n" not in response:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise RuntimeError("websocket handshake 실패")
            response += chunk
        head, _, rest = response.partition(b"\r\n\r\n")
        if b" 101 " not in head.split(b"\r\n")[0]:
            raise RuntimeError(head.decode(errors="replace"))
        self.buffer = rest

    def send(self, text):
        payload = text.encode()
        header = bytearray([0x81])
        length = len(payload)
        if length < 126:
            header.append(0x80 | length)
        elif length < 65536:
            header.append(0x80 | 126)
            header += struct.pack(">H", length)
        else:
            header.append(0x80 | 127)
            header += struct.pack(">Q", length)
        mask = os.urandom(4)
        header += mask
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        self.sock.sendall(bytes(header) + masked)

    def _read(self, n):
        while len(self.buffer) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise RuntimeError("websocket 연결 끊김")
            self.buffer += chunk
        data, self.buffer = self.buffer[:n], self.buffer[n:]
        return data

    def recv(self):
        message = b""
        while True:
            first, second = self._read(2)
            fin = first & 0x80
            opcode = first & 0x0F
            length = second & 0x7F
            if length == 126:
                (length,) = struct.unpack(">H", self._read(2))
            elif length == 127:
                (length,) = struct.unpack(">Q", self._read(8))
            payload = self._read(length)
            if opcode == 0x8:
                raise RuntimeError("websocket 닫힘")
            if opcode in (0x1, 0x0):
                message += payload
                if fin:
                    return message.decode()


class Page:
    """headless Chrome 한 개와 page target 하나."""

    def __init__(self, chrome):
        self.profile = tempfile.mkdtemp(prefix="stellar-cdp-")
        self.process = subprocess.Popen(
            [
                chrome,
                "--headless=new",
                "--disable-gpu",
                "--no-first-run",
                "--no-default-browser-check",
                "--remote-debugging-port=0",
                "--user-data-dir=" + self.profile,
                "--window-size=800,600",
                "about:blank",
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True,
        )
        port = None
        deadline = time.time() + 30
        while time.time() < deadline and port is None:
            line = self.process.stderr.readline()
            if "DevTools listening on ws://" in line:
                port = int(line.split("ws://")[1].split("/")[0].split(":")[1])
        if port is None:
            raise RuntimeError("DevTools port를 찾지 못했습니다")
        targets = json.load(urllib.request.urlopen(f"http://127.0.0.1:{port}/json/list"))
        page = next(t for t in targets if t["type"] == "page")
        self.ws = WebSocket(page["webSocketDebuggerUrl"])
        self.next_id = 0

    def call(self, method, **params):
        self.next_id += 1
        ident = self.next_id
        self.ws.send(json.dumps({"id": ident, "method": method, "params": params}))
        while True:
            message = json.loads(self.ws.recv())
            if message.get("id") == ident:
                if "error" in message:
                    raise RuntimeError(f"{method}: {message['error']}")
                return message.get("result", {})

    def load_html(self, html):
        self.call("Page.enable")
        self.call("Page.navigate", url="data:text/html;charset=utf-8," + urllib.parse.quote(html))
        deadline = time.time() + 30
        while time.time() < deadline:
            if self.evaluate("document.readyState") == "complete":
                return
            time.sleep(0.05)
        raise RuntimeError("page load 시간 초과")

    def evaluate(self, expression):
        result = self.call("Runtime.evaluate", expression=expression, returnByValue=True)
        return result["result"].get("value")

    def emulate_media(self, features):
        self.call(
            "Emulation.setEmulatedMedia",
            features=[{"name": name, "value": value} for name, value in features],
        )

    def force_pseudo(self, selector, classes):
        self.call("DOM.enable")
        self.call("CSS.enable")
        root = self.call("DOM.getDocument")["root"]["nodeId"]
        node = self.call("DOM.querySelector", nodeId=root, selector=selector)["nodeId"]
        self.call("CSS.forcePseudoState", nodeId=node, forcedPseudoClasses=classes)

    def close(self):
        self.process.terminate()
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.kill()
        shutil.rmtree(self.profile, ignore_errors=True)
