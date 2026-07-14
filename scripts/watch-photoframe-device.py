#!/usr/bin/env python3
"""Poll photoframe device state and print OTA/render evidence.

This is intended for a long-running tmux session while the frame is in deep sleep.
It only reads orchestrator APIs and never mutates server state.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import ssl
import time
import urllib.parse
import urllib.request
from typing import Any


INTERESTING_LOG_NEEDLES = (
    "firmware version changed",
    "firmware_update",
    "ota:",
    "OTA",
    "boot rtc sleep snapshot",
    "irq od",
    "irq pushpull",
    "render rails ready",
    "after_panel_flush",
    "panel:",
    "panel_flush",
    "busy timeout",
    "render status=",
    "photo_history",
    "checkin:",
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--token", required=True)
    parser.add_argument("--device-id", required=True)
    parser.add_argument("--target-version", default="")
    parser.add_argument("--log-request-id", type=int, default=0)
    parser.add_argument("--insecure", action="store_true")
    return parser.parse_args()


def epoch_label(value: Any) -> str:
    try:
        epoch = int(value)
    except (TypeError, ValueError):
        return "-"
    if epoch <= 0:
        return "-"
    return dt.datetime.fromtimestamp(epoch).strftime("%Y-%m-%d %H:%M:%S %Z")


class Client:
    def __init__(self, base_url: str, token: str, insecure: bool) -> None:
        self.base_url = base_url.rstrip("/")
        self.token = token
        self.context = ssl._create_unverified_context() if insecure else None

    def get(self, path: str, query: dict[str, Any] | None = None) -> dict[str, Any]:
        url = f"{self.base_url}{path}"
        if query:
            url = f"{url}?{urllib.parse.urlencode(query)}"
        request = urllib.request.Request(url, headers={"X-PhotoFrame-Token": self.token})
        with urllib.request.urlopen(request, context=self.context, timeout=20) as response:
            return json.loads(response.read().decode("utf-8"))


def newest(items: dict[str, Any]) -> dict[str, Any] | None:
    rows = items.get("items") or []
    return rows[0] if rows else None


def print_log_upload(upload: dict[str, Any]) -> None:
    payload = upload.get("payload") or {}
    lines = payload.get("lines") or []
    print(
        "[upload] "
        f"id={upload.get('id')} request={upload.get('request_id')} "
        f"lines={upload.get('line_count')} truncated={upload.get('truncated')} "
        f"uploaded={epoch_label(upload.get('uploaded_epoch'))}",
        flush=True,
    )
    interesting = [
        line for line in lines if any(needle in line for needle in INTERESTING_LOG_NEEDLES)
    ]
    if not interesting:
        interesting = lines[-40:]
    print("[upload] interesting excerpts:", flush=True)
    for line in interesting[-140:]:
        print(f"  {line}", flush=True)


def main() -> int:
    args = parse_args()
    client = Client(args.base_url, args.token, args.insecure)
    last_stage_id: Any = None
    last_upload_id: Any = None
    last_version: Any = None
    last_log_status: Any = None

    while True:
        now_label = dt.datetime.now().strftime("%Y-%m-%d %H:%M:%S %Z")
        sleep_for = 300
        try:
            devices = client.get("/api/v1/devices")
            device = next(
                (
                    row
                    for row in devices.get("devices", [])
                    if row.get("device_id") == args.device_id
                ),
                None,
            )
            stages = client.get(
                "/api/v1/device-debug-stages",
                {"device_id": args.device_id, "limit": 20},
            )
            requests = client.get(
                "/api/v1/device-log-requests",
                {"device_id": args.device_id, "limit": 10},
            )
            uploads = client.get(
                "/api/v1/device-log-uploads",
                {"device_id": args.device_id, "limit": 5},
            )
            request_row = next(
                (
                    row
                    for row in requests.get("items", [])
                    if int(row.get("request_id", -1)) == args.log_request_id
                ),
                None,
            )
            newest_stage = newest(stages)
            newest_upload = newest(uploads)

            print(f"\n===== {now_label} =====", flush=True)
            if device is None:
                print(f"device not found: {args.device_id}", flush=True)
            else:
                version = device.get("firmware_version")
                eta = int(device.get("eta_seconds") or 300)
                sleep_for = 300
                if eta <= 900:
                    sleep_for = 60
                if eta <= 180:
                    sleep_for = 20
                if eta <= 30:
                    sleep_for = 10
                print(
                    "device "
                    f"version={version} target={args.target_version or '-'} "
                    f"partition={device.get('running_partition')} "
                    f"ota_state={device.get('ota_state')} "
                    f"ota_target={device.get('ota_target_version')} "
                    f"eta={eta}s next={epoch_label(device.get('next_wakeup_epoch'))} "
                    f"last_stage={device.get('last_debug_stage')}",
                    flush=True,
                )
                if version != last_version:
                    print(f"[version-change] {last_version} -> {version}", flush=True)
                    last_version = version

            if newest_stage and newest_stage.get("id") != last_stage_id:
                print("[stages] latest:", flush=True)
                for item in (stages.get("items") or [])[:12]:
                    print(
                        f"  id={item.get('id')} "
                        f"{epoch_label(item.get('stage_epoch'))} {item.get('stage')}",
                        flush=True,
                    )
                last_stage_id = newest_stage.get("id")

            if request_row is not None:
                status = request_row.get("status")
                if status != last_log_status:
                    print(
                        "[log-request] "
                        f"id={args.log_request_id} status={status} "
                        f"completed={epoch_label(request_row.get('completed_epoch'))} "
                        f"lines={request_row.get('uploaded_line_count')} "
                        f"truncated={request_row.get('uploaded_truncated')}",
                        flush=True,
                    )
                    last_log_status = status

            if newest_upload and newest_upload.get("id") != last_upload_id:
                last_upload_id = newest_upload.get("id")
                print_log_upload(newest_upload)

            time.sleep(max(10, sleep_for))
        except Exception as exc:  # noqa: BLE001 - watcher must survive transient failures.
            print(
                f"\n===== {now_label} ERROR ===== {type(exc).__name__}: {exc}",
                flush=True,
            )
            time.sleep(60)


if __name__ == "__main__":
    raise SystemExit(main())
