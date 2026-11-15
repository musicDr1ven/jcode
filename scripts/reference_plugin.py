#!/usr/bin/env python3
"""jcode reference plugin.

Exercises every extension point so the protocol is validated by something other
than the first consumer:

- observes lifecycle events (session/turn/tool) by logging them to stderr
- contributes one labelled line of context per turn
- registers one tool (`reference__echo`)
- gates `bash`: denies ``rm -rf`` and rewrites the command ``rewrite me``

Run it from a `[[plugins]]` entry:

    [[plugins]]
    name = "reference"
    command = "python3"
    args = ["scripts/reference_plugin.py"]
    capabilities = ["observe", "tool_events", "context", "tools", "gate"]

It writes protocol messages to stdout and diagnostics to stderr; stderr is not
parsed as protocol.
"""

import json
import sys

PROTOCOL_VERSION = 1
PLUGIN_NAME = "reference"
PLUGIN_VERSION = "1.0.0"
CAPABILITIES = ["observe", "tool_events", "context", "tools", "gate"]


def send(message: dict) -> None:
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def log(text: str) -> None:
    print(f"[reference-plugin] {text}", file=sys.stderr, flush=True)


def handle_request(request_id, method: str, params: dict) -> None:
    if method == "context":
        send(
            {
                "type": "response",
                "id": request_id,
                "ok": True,
                "result": {
                    "text": "Reference plugin context: the reference plugin is active for this turn."
                },
            }
        )
    elif method == "tools/list":
        send(
            {
                "type": "response",
                "id": request_id,
                "ok": True,
                "result": {
                    "tools": [
                        {
                            "name": "echo",
                            "description": "Echo the provided text back (reference plugin).",
                            "input_schema": {
                                "type": "object",
                                "properties": {"text": {"type": "string"}},
                                "required": ["text"],
                            },
                        }
                    ]
                },
            }
        )
    elif method == "tools/call":
        tool_input = params.get("input") or {}
        send(
            {
                "type": "response",
                "id": request_id,
                "ok": True,
                "result": {"output": f"reference echo: {tool_input.get('text', '')}"},
            }
        )
    elif method == "gate":
        tool_name = params.get("tool_name")
        tool_input = params.get("input") or {}
        command = str(tool_input.get("command", ""))
        if tool_name == "bash" and "rm -rf" in command:
            send(
                {
                    "type": "response",
                    "id": request_id,
                    "ok": True,
                    "result": {
                        "decision": "deny",
                        "reason": "the reference plugin refuses recursive force deletes",
                    },
                }
            )
        elif tool_name == "bash" and command == "rewrite me":
            send(
                {
                    "type": "response",
                    "id": request_id,
                    "ok": True,
                    "result": {
                        "decision": "rewrite",
                        "input": {"command": "echo rewritten by reference plugin"},
                    },
                }
            )
        else:
            send(
                {
                    "type": "response",
                    "id": request_id,
                    "ok": True,
                    "result": {"decision": "allow"},
                }
            )
    else:
        send(
            {
                "type": "response",
                "id": request_id,
                "ok": False,
                "error": f"unsupported method {method}",
            }
        )


def main() -> None:
    for raw_line in sys.stdin:
        line = raw_line.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            log("ignored a non-protocol line")
            continue

        kind = message.get("type")
        if kind == "hello":
            send(
                {
                    "type": "hello",
                    "protocol": PROTOCOL_VERSION,
                    "name": PLUGIN_NAME,
                    "version": PLUGIN_VERSION,
                    "capabilities": CAPABILITIES,
                }
            )
        elif kind == "event":
            log(f"event {message.get('event')}")
        elif kind == "request":
            handle_request(message.get("id"), message.get("method", ""), message.get("params") or {})
        else:
            log(f"ignored message type {kind!r}")
    log("stdin closed; exiting")


if __name__ == "__main__":
    main()
