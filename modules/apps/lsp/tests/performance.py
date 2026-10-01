#!/usr/bin/env python3
"""Repeatable stdio latency probe; generated inputs and reports live under .folio."""

import argparse
import hashlib
import json
from pathlib import Path
import queue
import statistics
import subprocess
import threading
import time


class Client:
    def __init__(self, binary, project, log_path, log_filter="info"):
        self.log = log_path.open("w")
        self.process = subprocess.Popen(
            [str(binary), "--log-filter", log_filter, "--log-format", "json", "lsp"],
            cwd=project, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log,
        )
        self.messages = queue.Queue()
        self.next_id = 0
        self.notifications = []
        threading.Thread(target=self.read, daemon=True).start()

    def read(self):
        try:
            while True:
                header = self.process.stdout.readline()
                if not header:
                    raise EOFError("language server exited")
                length = int(header.split(b":")[1])
                while self.process.stdout.readline().strip():
                    pass
                data = self.process.stdout.read(length)
                self.messages.put(json.loads(data))
        except Exception as error:
            self.messages.put(error)

    def send(self, method, params=None, request=False):
        message = {"jsonrpc": "2.0", "method": method, "params": params or {}}
        if request:
            self.next_id += 1
            message["id"] = self.next_id
        self.write(message)
        return message.get("id")

    def write(self, message):
        data = json.dumps(message).encode()
        self.process.stdin.write(f"Content-Length: {len(data)}\r\n\r\n".encode() + data)
        self.process.stdin.flush()

    def wait(self, ids):
        results = {}
        deadline = time.monotonic() + 120
        while set(results) != set(ids):
            message = self.messages.get(timeout=max(0.01, deadline - time.monotonic()))
            if isinstance(message, Exception):
                raise message
            if "id" in message and "method" in message:
                assert message["method"] in ["client/registerCapability", "client/unregisterCapability"], message
                self.notifications.append(message)
                self.write({"jsonrpc": "2.0", "id": message["id"], "result": None})
            elif "id" in message:
                assert message["id"] in ids, message
                results[message["id"]] = message
            else:
                self.notifications.append(message)
        return [results[identifier] for identifier in ids]

    def request(self, method, params=None):
        return self.wait([self.send(method, params, True)])[0]

    def barrier(self):
        # An unknown request provides a queue barrier without triggering analysis.
        result = self.request("folio/benchmarkBarrier")
        assert result["error"]["code"] == -32601, result

    def finish(self):
        self.request("shutdown")
        self.send("exit")
        self.process.wait(timeout=10)
        self.log.close()
        assert self.process.returncode == 0
        # Old servers can attach the whole project to every log event. Keep useful
        # fields while discarding repeated span dumps from the disposable report.
        path = Path(self.log.name)
        compact = path.with_suffix(".compact")
        with path.open() as source, compact.open("w") as destination:
            for line in source:
                event = json.loads(line)
                event.pop("span", None)
                event.pop("spans", None)
                destination.write(json.dumps(event) + "\n")
        compact.replace(path)


def script(name, methods, parent=""):
    header = f"ScriptName {name}{parent}\n; Unicode: 雪 🦊\nInt Property Counter Auto\n"
    functions = [
        f"Int Function Work{i}(Int value = 1)\n    Return value + Counter\nEndFunction\n"
        for i in range(methods)
    ]
    return header + "\n".join(functions) + "\nFunction Run()\n    Int current = Work0(1)\nEndFunction\n"


def make_project(root, count, methods, dependencies):
    sources = root / "Source/Scripts"
    sources.mkdir(parents=True, exist_ok=True)
    manifest = '''schema = 3
[package]
name = "lsp-benchmark"
version = "0.1.0"
[languages.papyrus]
dialect = "skyrim"
extensions = ["psc"]
[build]
target = "skyrim-se"
profile = "dev"
emit = ["pex"]
'''
    if dependencies:
        manifest += '''[[dependencies]]
name = "api"
kind = "psc"
path = "api"
[[dependencies]]
name = "ck-1.6.1170"
kind = "builtin"
path = "ck-1.6.1170"
'''
        api = root / "api"
        api.mkdir(exist_ok=True)
        for i in range(120):
            (api / f"Api{i:03}.psc").write_text(
                f"ScriptName Api{i:03}\n" + "".join(
                    f"Int Function Value{j}(Int count = 1) Native\n" for j in range(20)
                )
            )
    (root / "folio.toml").write_text(manifest)
    for i in range(count):
        (sources / f"Bench{i:03}.psc").write_text(script(f"Bench{i:03}", methods))
    return sources / "Bench000.psc"


def position(text, marker):
    prefix = text[:text.rindex(marker)]
    return {"line": prefix.count("\n"), "character": len(prefix.rsplit("\n", 1)[-1])}


def bundle(client, uri, text):
    common = {"textDocument": {"uri": uri}}
    requests = [
        ("textDocument/documentSymbol", common),
        ("textDocument/semanticTokens/full", common),
        ("textDocument/hover", {**common, "position": position(text, "Work0(1)")}),
        ("textDocument/definition", {**common, "position": position(text, "Work0(1)")}),
        ("textDocument/signatureHelp", {**common, "position": position(text, "1)\nEndFunction")}),
    ]
    responses = client.wait([client.send(method, params, True) for method, params in requests])
    for response in responses:
        assert "error" not in response, response
    result = [response["result"] for response in responses]
    assert result[0][0]["name"] == "Bench000", result[0]
    assert result[1]["data"] and result[2] and result[3] and result[4], result
    assert "Int Function Work0" in result[2]["contents"]["value"], result[2]
    assert result[3]["range"]["start"]["line"] == 3, result[3]
    return result


def measured(action):
    started = time.perf_counter()
    result = action()
    return (time.perf_counter() - started) * 1000, result


def run(binary, root, focus, log_path, encoding, log_filter):
    started = time.perf_counter()
    client = Client(binary, root, log_path, log_filter)
    initialized = client.request("initialize", {
        "rootUri": root.as_uri(),
        "capabilities": {"general": {"positionEncodings": [encoding]}},
    })
    assert initialized["result"]["capabilities"]["positionEncoding"] == encoding
    metrics = {"initialize_reply_ms": (time.perf_counter() - started) * 1000}
    client.send("initialized")
    metrics["initial_load_ms"], _ = measured(client.barrier)
    text = focus.read_text()
    uri = focus.as_uri()
    document = {"uri": uri, "languageId": "papyrus", "version": 1, "text": text}

    def opened():
        client.send("textDocument/didOpen", {"textDocument": document})
        return bundle(client, uri, text)

    metrics["first_open_ms"], first = measured(opened)
    metrics["cold_symbols_ms"] = (time.perf_counter() - started) * 1000
    original_diagnostics = next(
        message["params"]["diagnostics"] for message in reversed(client.notifications)
        if message["method"] == "textDocument/publishDiagnostics" and message["params"]["uri"] == uri
    )
    assert not original_diagnostics, original_diagnostics
    metrics["warm_requests_ms"], warm = measured(lambda: bundle(client, uri, text))
    assert first == warm

    def closed():
        client.send("textDocument/didClose", {"textDocument": {"uri": uri}})
        client.barrier()

    metrics["close_ms"], _ = measured(closed)
    metrics["reopen_ms"], reopened = measured(opened)
    assert first == reopened
    changed = text.replace("value + Counter", "value + missing", 1)

    def change():
        client.send("textDocument/didChange", {
            "textDocument": {"uri": uri, "version": 2},
            "contentChanges": [{"text": changed}],
        })
        return bundle(client, uri, changed)

    metrics["change_ms"], _ = measured(change)
    changed_diagnostics = next(
        message["params"] for message in reversed(client.notifications)
        if message["method"] == "textDocument/publishDiagnostics" and message["params"]["uri"] == uri
    )
    assert changed_diagnostics["version"] == 2 and any(
        "missing" in diagnostic["message"] for diagnostic in changed_diagnostics["diagnostics"]
    ), changed_diagnostics
    metrics["discard_close_ms"], _ = measured(closed)
    metrics["restored_open_ms"], restored = measured(opened)
    assert first == restored
    assert next(
        message["params"]["diagnostics"] for message in reversed(client.notifications)
        if message["method"] == "textDocument/publishDiagnostics" and message["params"]["uri"] == uri
    ) == original_diagnostics
    client.finish()
    payload = json.dumps(first, sort_keys=True, ensure_ascii=False).encode()
    return metrics, hashlib.sha256(payload).hexdigest(), first


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--compare", type=Path)
    parser.add_argument("--samples", type=int, default=3)
    parser.add_argument("--encoding", choices=["utf-8", "utf-16"], default="utf-16")
    parser.add_argument("--log-filter", default="info")
    args = parser.parse_args()
    repository = Path(__file__).resolve().parents[4]
    work = repository / ".folio/lsp-perf"
    work.mkdir(parents=True, exist_ok=True)
    reports = {}
    for name, count, methods, dependencies in [
        ("small", 1, 20, False),
        ("project", 64, 24, True),
        ("long", 1, 800, False),
    ]:
        root = work / name
        focus = make_project(root, count, methods, dependencies)
        runs = []
        hashes = []
        for sample in range(args.samples):
            log = work / f"{args.output.stem}-{name}-{sample}.jsonl"
            metrics, digest, payload = run(args.binary.resolve(), root, focus, log, args.encoding, args.log_filter)
            runs.append(metrics)
            hashes.append(digest)
            (work / f"{args.output.stem}-{name}-symbols.json").write_text(
                json.dumps(payload, ensure_ascii=False, indent=2)
            )
        assert len(set(hashes)) == 1
        reports[name] = {
            "median_ms": {key: round(statistics.median(run[key] for run in runs), 3) for key in runs[0]},
            "runs": runs, "symbol_hash": hashes[0],
            "sources": count, "functions_per_source": methods,
            "encoding": args.encoding, "log_filter": args.log_filter,
            "log_format": "json",
        }
        print(name, json.dumps(reports[name]["median_ms"]), flush=True)
    if args.compare:
        baseline = json.loads(args.compare.read_text())
        for name in reports:
            assert reports[name]["symbol_hash"] == baseline[name]["symbol_hash"], name
        print("Symbols, token spans, hover, definition and signature match the baseline.")
    args.output.write_text(json.dumps(reports, indent=2) + "\n")


if __name__ == "__main__":
    main()
