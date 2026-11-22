#!/usr/bin/env python3

import argparse
import hashlib
import json
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import re
import shlex
import shutil
import signal
import subprocess
import tempfile
import threading
import time
import uuid


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def wait_for(read, message):
    for _ in range(100):
        value = read()
        if value:
            return value
        time.sleep(0.2)
    raise RuntimeError(message)


class Model(BaseHTTPRequestHandler):
    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        if self.path != "/v1/responses" or self.server.prompt not in json.dumps(request):
            self.send_error(400, "Unexpected verification request")
            return
        self.server.received = True
        item = {
            "id": "msg_verify", "type": "message", "role": "assistant", "status": "completed",
            "content": [{"type": "output_text", "text": "Timeline mock response", "annotations": []}],
        }
        response = {"id": "resp_verify", "model": "gpt-5.4", "output": []}
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        for kind, data in (
            ("response.created", {"response": {**response, "status": "in_progress"}}),
            ("response.output_item.added", {"output_index": 0, "item": {**item, "content": []}}),
            ("response.output_text.delta", {
                "output_index": 0, "content_index": 0, "item_id": item["id"],
                "delta": item["content"][0]["text"],
            }),
            ("response.output_item.done", {"output_index": 0, "item": item}),
            ("response.completed", {"response": {
                **response, "status": "completed", "output": [item],
                "usage": {"input_tokens": 12, "output_tokens": 4, "total_tokens": 16},
            }}),
        ):
            event = json.dumps({"type": kind, **data})
            self.wfile.write(f"event: {kind}\ndata: {event}\n\n".encode())


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("timeline", type=Path)
    parser.add_argument("evidence", type=Path, help="new directory")
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--release", nargs=2, metavar=("TAG", "SHA_FILE"))
    source.add_argument("--commit")
    args = parser.parse_args()
    timeline = args.timeline.resolve(strict=True)
    evidence = args.evidence.resolve()
    evidence.mkdir(mode=0o700)
    transcript = evidence / "commands.jsonl"

    def record(entry):
        with transcript.open("a") as log:
            log.write(json.dumps(entry, ensure_ascii=False) + "\n")

    sha = hashlib.sha256(timeline.read_bytes()).hexdigest()
    tag, checksum = args.release or (None, None)
    expected = Path(checksum).read_text().split()[0] if checksum else None
    record({"binary": str(timeline), "sha256": sha, "expected_sha256": expected,
            "release": tag, "commit": args.commit})
    require(not tag or sha == expected, "Release SHA-256 mismatch")

    herdr, codex = map(shutil.which, ("herdr", "codex"))
    require(herdr and codex, "herdr and codex must be on PATH")
    with tempfile.TemporaryDirectory(prefix="tl-", dir="/private/tmp") as temp:
        root = Path(temp)
        socket = root / "herdr.sock"
        codex_home = root / "codex"
        herdr_config = root / "herdr.toml"
        project = root / "project"
        for directory in (root / "home", codex_home, project):
            directory.mkdir()
        herdr_config.write_text(
            '[terminal]\ndefault_shell="/bin/bash"\nshell_mode="non_login"\n'
            '[update]\nversion_check=false\nmanifest_check=false\n'
        )
        env = {
            "HOME": str(root / "home"), "CODEX_HOME": str(codex_home),
            "XDG_CONFIG_HOME": str(root / "config"), "XDG_STATE_HOME": str(root / "state"),
            "XDG_CACHE_HOME": str(root / "cache"), "TMPDIR": temp,
            "HERDR_CONFIG_PATH": str(herdr_config), "HERDR_SOCKET_PATH": str(socket),
            "HERDR_TIMELINE_HERDR_DIR": temp,
            "PATH": os.pathsep.join((str(Path(herdr).parent), str(Path(codex).parent), "/usr/bin", "/bin")),
            "SHELL": "/bin/bash", "USER": os.environ["USER"], "TERM": "xterm-256color",
            "LANG": "en_US.UTF-8", "NO_COLOR": "1",
            "NO_PROXY": "127.0.0.1,localhost", "ALL_PROXY": "http://127.0.0.1:9",
            "HTTP_PROXY": "http://127.0.0.1:9", "HTTPS_PROXY": "http://127.0.0.1:9",
        }

        def command(*argv, check=True):
            try:
                result = subprocess.run(argv, env=env, cwd=project, text=True, capture_output=True, timeout=40)
            except subprocess.TimeoutExpired as error:
                record({"argv": argv, "exit": None, "error": "timeout", "timeout": error.timeout,
                        "stdout": (error.stdout or b"").decode("utf-8", errors="replace"),
                        "stderr": (error.stderr or b"").decode("utf-8", errors="replace")})
                raise
            except OSError as error:
                record({"argv": argv, "exit": None, "error": "launch_failed", "detail": str(error),
                        "stdout": "", "stderr": ""})
                raise
            record({"argv": argv, "exit": result.returncode, "stdout": result.stdout, "stderr": result.stderr})
            if check:
                require(result.returncode == 0, f"{shlex.join(argv)}: exit {result.returncode}")
            return result

        require(command("/usr/bin/uname", "-m").stdout.strip() == "arm64", "An arm64 Mac is required")
        require("Mach-O 64-bit executable arm64" in command("/usr/bin/file", str(timeline)).stdout,
                "Timeline must be a native arm64 executable")
        command(herdr, "--version")
        command(codex, "--version")

        def snapshot():
            return json.loads(command(str(timeline), "--json").stdout)

        disconnected = snapshot()
        require(not disconnected["panes"] and any("herdr pane list" in w for w in disconnected["warnings"]),
                "Timeline must warn when the isolated Herdr socket is missing")
        prompt = f"Timeline verify {uuid.uuid4()}"
        model = ThreadingHTTPServer(("127.0.0.1", 0), Model)
        model.prompt = prompt
        model.received = False
        thread = threading.Thread(target=model.serve_forever, daemon=True)
        thread.start()
        server = None
        try:
            (codex_home / "config.toml").write_text(
                'model = "gpt-5.4"\nmodel_provider = "loopback"\n'
                '[model_providers.loopback]\nname = "Timeline verification"\nwire_api = "responses"\n'
                f'base_url = "http://127.0.0.1:{model.server_port}/v1"\n'
                'request_max_retries = 0\nstream_max_retries = 0\n'
                '[analytics]\nenabled = false\n[tui]\nscreen_reader_detection_done = true\n'
                f'[projects."{project}"]\ntrust_level = "trusted"\n'
            )
            command(herdr, "integration", "install", "codex")
            command(herdr, "integration", "status")
            hooks = codex_home / "hooks.json"
            registry = json.loads(hooks.read_text())
            record({"hook_registry": registry})
            require(registry["hooks"] == {"SessionStart": [{"hooks": [{
                "command": f"bash '{codex_home / 'herdr-agent-state.sh'}' session",
                "timeout": 10, "type": "command",
            }]}]}, "Unexpected Codex hook registry")
            with (evidence / "herdr.log").open("w") as log:
                try:
                    server = subprocess.Popen(
                        [herdr, "server"], env=env, cwd=project, stdout=log, stderr=subprocess.STDOUT,
                        start_new_session=True,
                    )
                except OSError as error:
                    record({"argv": [herdr, "server"], "exit": None, "error": "launch_failed",
                            "detail": str(error), "stdout": "", "stderr": ""})
                    raise
            wait_for(socket.is_socket, "Isolated Herdr socket is unavailable")
            native_empty = json.loads(command(herdr, "pane", "list").stdout)["result"]["panes"]
            empty = snapshot()
            require(not native_empty and not empty["panes"] and not empty["warnings"],
                    "The new Herdr server must have an empty, warning-free list")
            created = command(herdr, "workspace", "create", "--cwd", str(project), "--label", "timeline-verify")
            pane = json.loads(created.stdout)["result"]["root_pane"]["pane_id"]
            command(herdr, "agent", "start", "timeline-verify", "--kind", "codex", "--pane", pane, "--", "--no-daemon")
            def pane_info():
                return json.loads(command(herdr, "pane", "get", pane).stdout)["result"]["pane"]

            before = pane_info()
            unbound = command(str(timeline), "--timeline", pane, "--width", "120").stdout
            require("transcript が見つからない" in unbound, "Timeline must diagnose missing Codex history")

            def screen():
                return command(herdr, "pane", "read", pane, "--source", "visible", "--lines", "40").stdout

            def approval_or_ready():
                visible = screen()
                if "Hooks need review" in visible or "Ask Codex to do anything" in visible:
                    return visible

            visible = wait_for(approval_or_ready, "Codex showed neither the hook review nor the ready prompt")
            if "Hooks need review" in visible:
                require(before["agent"] == "codex" and not before.get("agent_session"),
                        "Expected a Codex pane with no native ID before hook approval")
                require("1 hook is new or changed" in visible and "Trust all and continue" in visible,
                        "Unexpected Codex hook trust screen")
                command(herdr, "agent", "send-keys", pane, "down", "enter")
                wait_for(lambda: "Ask Codex to do anything" in screen(), "Codex hook is waiting for trust")
            command(herdr, "agent", "prompt", pane, prompt)

            def attached():
                pane_data = pane_info()
                session = pane_data.get("agent_session") or {}
                if session.get("value"):
                    require(pane_data["pane_id"] == pane and session.get("source") == "herdr:codex",
                            "Unexpected native session source")
                    return str(uuid.UUID(session["value"]))

            session_id = wait_for(attached, "No native Codex session ID")
            matches = wait_for(
                lambda: list((codex_home / "sessions").glob(f"*/*/*/rollout-*-{session_id}.jsonl")),
                "No rollout for native Codex session",
            )
            require(len(matches) == 1, "Multiple rollouts for native Codex session")
            rollout = matches[0]

            def completed():
                lines = rollout.read_text().splitlines()
                if not lines:
                    return None
                rows = [json.loads(line) for line in lines]
                require(rows[0]["type"] == "session_meta" and rows[0]["payload"]["id"] == session_id,
                        "Rollout metadata does not match native session ID")
                return next((row for row in rows if row.get("type") == "event_msg"
                             and row["payload"].get("type") == "item_completed"
                             and row["payload"].get("item", {}).get("type") == "UserMessage"
                             and row["payload"].get("thread_id") == session_id
                             and [part.get("text") for part in row["payload"]["item"]["content"]] == [prompt]), None)

            user_message = wait_for(completed, "Prompt not found in Codex rollout")
            wait_for(lambda: model.received, "Local model did not receive prompt")
            populated = snapshot()
            require(not populated["warnings"] and any(row["pane_id"] == pane for row in populated["panes"]),
                    "Timeline cannot enumerate the target pane")
            history = command(str(timeline), "--timeline", pane, "--width", "120").stdout
            require(len(re.findall(r"指示\s+" + re.escape(prompt) + r"$", history, re.MULTILINE)) == 1,
                    "Timeline does not show the exact prompt once")
            record({"pane": pane, "session_id": session_id, "rollout": rollout.name,
                    "rollout_header": json.loads(rollout.read_text().splitlines()[0])["payload"]["id"],
                    "rollout_user_message": user_message, "model_received": model.received})
        finally:
            try:
                if server:
                    stop = None
                    try:
                        if socket.is_socket():
                            stop = command(herdr, "server", "stop", check=False)
                    finally:
                        try:
                            status = server.wait(timeout=5)
                        except subprocess.TimeoutExpired:
                            os.killpg(server.pid, signal.SIGTERM)
                            status = server.wait(timeout=5)
                        record({"argv": [herdr, "server"], "exit": status, "socket_removed": not socket.exists()})
                    require(stop and stop.returncode == 0 and status == 0 and not socket.exists(),
                            "Isolated Herdr cleanup failed")
            finally:
                model.shutdown()
                thread.join()
                model.server_close()
    require(not root.exists() and transcript.is_file(), "Cleanup or evidence check failed")
    record({"status": "passed", "temporary_state_removed": True})
    print(evidence)


if __name__ == "__main__":
    main()
