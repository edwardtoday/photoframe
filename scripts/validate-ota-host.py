#!/usr/bin/env python3
"""宿主机侧 OTA 验证脚本。

支持两类场景：
- 成功升级：上传 artifact、创建 rollout、可选验证日志上传，等待设备切到目标版本
- 阶段注入故障：等待设备发出指定 debug stage，再通过 USB 串口触发 reset
"""

from __future__ import annotations

import argparse
import json
import mimetypes
import ssl
import subprocess
import sys
import time
import urllib.parse
import urllib.request
from urllib.error import HTTPError
from urllib.error import URLError
import uuid
from pathlib import Path
from typing import Any


def _build_context(insecure: bool) -> ssl.SSLContext | None:
    if not insecure:
        return None
    return ssl._create_unverified_context()


class ApiClient:
    def __init__(self, base_url: str, admin_token: str, insecure: bool) -> None:
        self.base_url = base_url.rstrip("/")
        self.admin_token = admin_token
        self.context = _build_context(insecure)
        self.max_attempts = 5

    def _request(
        self,
        method: str,
        path: str,
        *,
        query: dict[str, Any] | None = None,
        json_body: dict[str, Any] | None = None,
        data: bytes | None = None,
        content_type: str | None = None,
        timeout: float = 30.0,
    ) -> Any:
        url = f"{self.base_url}{path}"
        if query:
            clean = {key: value for key, value in query.items() if value is not None}
            if clean:
                url = f"{url}?{urllib.parse.urlencode(clean)}"
        body: bytes | None = data
        headers = {"X-PhotoFrame-Token": self.admin_token}
        if json_body is not None:
            body = json.dumps(json_body).encode("utf-8")
            headers["Content-Type"] = "application/json"
        elif content_type:
            headers["Content-Type"] = content_type
        request = urllib.request.Request(url, data=body, headers=headers, method=method)
        last_error: Exception | None = None
        for attempt in range(1, self.max_attempts + 1):
            try:
                with urllib.request.urlopen(request, timeout=timeout, context=self.context) as response:
                    payload = response.read()
                break
            except HTTPError:
                raise
            except URLError as exc:
                last_error = exc
                if attempt >= self.max_attempts:
                    raise
                time.sleep(min(2.0 * attempt, 5.0))
            except ConnectionResetError as exc:
                last_error = exc
                if attempt >= self.max_attempts:
                    raise
                time.sleep(min(2.0 * attempt, 5.0))
        else:
            assert last_error is not None
            raise last_error
        if not payload:
            return {}
        return json.loads(payload.decode("utf-8"))

    def get(self, path: str, *, query: dict[str, Any] | None = None) -> Any:
        return self._request("GET", path, query=query)

    def post_json(self, path: str, payload: dict[str, Any]) -> Any:
        return self._request("POST", path, json_body=payload)

    def delete(self, path: str) -> Any:
        return self._request("DELETE", path)

    def upload_artifact(self, version: str, note: str, artifact_path: Path) -> Any:
        boundary = f"photoframe-{uuid.uuid4().hex}"
        content_type = f"multipart/form-data; boundary={boundary}"
        file_bytes = artifact_path.read_bytes()
        file_name = artifact_path.name
        file_mime = mimetypes.guess_type(file_name)[0] or "application/octet-stream"
        fields = [
            (
                "version",
                None,
                str(version).encode("utf-8"),
                None,
            ),
            (
                "note",
                None,
                str(note).encode("utf-8"),
                None,
            ),
            (
                "file",
                file_name,
                file_bytes,
                file_mime,
            ),
        ]
        body_parts: list[bytes] = []
        for name, filename, payload, mime in fields:
            body_parts.append(f"--{boundary}\r\n".encode("utf-8"))
            disposition = f'Content-Disposition: form-data; name="{name}"'
            if filename:
                disposition += f'; filename="{filename}"'
            body_parts.append(f"{disposition}\r\n".encode("utf-8"))
            if mime:
                body_parts.append(f"Content-Type: {mime}\r\n".encode("utf-8"))
            body_parts.append(b"\r\n")
            body_parts.append(payload)
            body_parts.append(b"\r\n")
        body_parts.append(f"--{boundary}--\r\n".encode("utf-8"))
        body = b"".join(body_parts)
        return self._request(
            "POST",
            "/api/v1/firmware-artifacts/upload",
            data=body,
            content_type=content_type,
            timeout=120.0,
        )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="宿主机侧 OTA 成功/故障注入验证")
    parser.add_argument("--base-url", required=True, help="orchestrator base url")
    parser.add_argument("--admin-token", required=True, help="管理端 token")
    parser.add_argument("--device-id", required=True, help="设备 ID")
    parser.add_argument("--port", required=True, help="串口，例如 /dev/cu.usbmodem111201")
    parser.add_argument("--artifact-path", required=True, help="app.bin 路径")
    parser.add_argument("--version", required=True, help="目标固件版本")
    parser.add_argument("--note", default="", help="artifact/rollout note")
    parser.add_argument("--min-battery-percent", type=int, default=50)
    parser.add_argument("--requires-vbus", action=argparse.BooleanOptionalAction, default=True)
    parser.add_argument("--poll-interval-seconds", type=float, default=2.0)
    parser.add_argument("--timeout-seconds", type=float, default=600.0)
    parser.add_argument("--reset-stage", default="", help="等待该 debug stage 后执行 USB reset")
    parser.add_argument("--expect-stage", default="", help="等待该 debug stage，并验证设备保持原版本/分区")
    parser.add_argument(
        "--expect-version-unchanged",
        action="store_true",
        help="配合 --expect-stage 使用，要求设备版本与分区保持初始状态",
    )
    parser.add_argument("--log-reason", default="", help="同时创建日志采集请求")
    parser.add_argument("--log-max-lines", type=int, default=120)
    parser.add_argument("--log-max-bytes", type=int, default=8192)
    parser.add_argument("--log-expires-minutes", type=int, default=1440)
    parser.add_argument(
        "--esptool-python",
        default=str(Path(__file__).resolve().parent.parent / ".venv-host-tools" / "bin" / "python"),
        help="用于执行 esptool 的 Python 解释器",
    )
    parser.add_argument("--insecure", action="store_true", help="忽略 HTTPS 证书校验")
    parser.add_argument(
        "--skip-initial-reset",
        action="store_true",
        help="不在开始时主动 reset 设备，适用于设备刚唤醒时",
    )
    return parser.parse_args()


def print_json(prefix: str, payload: dict[str, Any]) -> None:
    print(prefix)
    print(json.dumps(payload, ensure_ascii=False, indent=2))


def esptool_reset(python_bin: str, port: str) -> None:
    cmd = [
        python_bin,
        "-m",
        "esptool",
        "--chip",
        "esp32s3",
        "--port",
        port,
        "--baud",
        "115200",
        "read-mac",
    ]
    print(f"[reset] {' '.join(cmd)}")
    subprocess.run(cmd, check=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)


def get_device_row(client: ApiClient, device_id: str) -> dict[str, Any]:
    payload = client.get("/api/v1/devices")
    for item in payload.get("devices", []):
        if str(item.get("device_id")) == device_id:
            return item
    raise RuntimeError(f"device not found: {device_id}")


def get_latest_stage_id(client: ApiClient, device_id: str) -> int:
    payload = client.get("/api/v1/device-debug-stages", query={"device_id": device_id, "limit": 1})
    items = payload.get("items", [])
    if not items:
        return 0
    return int(items[0]["id"])


def wait_for_stage(
    client: ApiClient,
    device_id: str,
    stage: str,
    *,
    after_id: int,
    timeout_seconds: float,
    poll_interval_seconds: float,
) -> dict[str, Any]:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        payload = client.get(
            "/api/v1/device-debug-stages",
            query={"device_id": device_id, "limit": 50},
        )
        for item in payload.get("items", []):
            if int(item["id"]) <= after_id:
                break
            if str(item["stage"]) == stage:
                return item
        time.sleep(poll_interval_seconds)
    raise TimeoutError(f"wait_for_stage timeout: {stage}")


def wait_for_log_upload(
    client: ApiClient,
    device_id: str,
    request_id: int,
    *,
    timeout_seconds: float,
    poll_interval_seconds: float,
) -> dict[str, Any]:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        payload = client.get(
            "/api/v1/device-log-uploads",
            query={"device_id": device_id, "limit": 20},
        )
        for item in payload.get("items", []):
            if int(item["request_id"]) == request_id:
                return item
        time.sleep(poll_interval_seconds)
    raise TimeoutError(f"log upload not received: request_id={request_id}")


def wait_for_success(
    client: ApiClient,
    device_id: str,
    target_version: str,
    *,
    initial_partition: str,
    timeout_seconds: float,
    poll_interval_seconds: float,
) -> dict[str, Any]:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        row = get_device_row(client, device_id)
        if (
            str(row.get("firmware_version") or "") == target_version
            and str(row.get("ota_state") or "") == "valid"
            and str(row.get("running_partition") or "") not in ("", initial_partition)
        ):
            return row
        time.sleep(poll_interval_seconds)
    raise TimeoutError(f"device did not reach target version: {target_version}")


def wait_for_reset_recovery(
    client: ApiClient,
    device_id: str,
    *,
    initial_version: str,
    initial_partition: str,
    timeout_seconds: float,
    poll_interval_seconds: float,
) -> dict[str, Any]:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        row = get_device_row(client, device_id)
        if (
            str(row.get("firmware_version") or "") == initial_version
            and str(row.get("running_partition") or "") == initial_partition
            and str(row.get("ota_state") or "") == "valid"
        ):
            return row
        time.sleep(poll_interval_seconds)
    raise TimeoutError("device did not recover to original partition/version after reset injection")


def main() -> int:
    args = parse_args()
    client = ApiClient(args.base_url, args.admin_token, args.insecure)
    artifact_path = Path(args.artifact_path).expanduser().resolve()
    if not artifact_path.is_file():
        raise SystemExit(f"artifact not found: {artifact_path}")

    initial = get_device_row(client, args.device_id)
    print_json("[device] initial", initial)

    artifact = client.upload_artifact(args.version, args.note, artifact_path)
    print_json("[artifact] uploaded", artifact)
    artifact_id = int(artifact["id"])

    rollout = client.post_json(
        "/api/v1/firmware-rollouts",
        {
            "device_id": args.device_id,
            "firmware_artifact_id": artifact_id,
            "min_battery_percent": int(args.min_battery_percent),
            "requires_vbus": bool(args.requires_vbus),
            "note": args.note,
        },
    )
    print_json("[rollout] created", rollout)
    rollout_id = int(rollout["id"])

    log_request_id: int | None = None
    if args.log_reason.strip():
        log_request = client.post_json(
            "/api/v1/device-log-requests",
            {
                "device_id": args.device_id,
                "reason": args.log_reason,
                "max_lines": int(args.log_max_lines),
                "max_bytes": int(args.log_max_bytes),
                "expires_in_minutes": int(args.log_expires_minutes),
            },
        )
        print_json("[log-request] created", log_request)
        log_request_id = int(log_request["request_id"])

    latest_stage_id = get_latest_stage_id(client, args.device_id)
    print(f"[debug-stage] latest known id before trigger: {latest_stage_id}")

    try:
        if not args.skip_initial_reset:
            esptool_reset(args.esptool_python, args.port)
            print("[reset] initial wake trigger sent")

        if args.reset_stage.strip():
            stage_item = wait_for_stage(
                client,
                args.device_id,
                args.reset_stage.strip(),
                after_id=latest_stage_id,
                timeout_seconds=args.timeout_seconds,
                poll_interval_seconds=args.poll_interval_seconds,
            )
            print_json("[debug-stage] matched", stage_item)
            esptool_reset(args.esptool_python, args.port)
            print("[reset] fault injected")
            recovered = wait_for_reset_recovery(
                client,
                args.device_id,
                initial_version=str(initial.get("firmware_version") or ""),
                initial_partition=str(initial.get("running_partition") or ""),
                timeout_seconds=args.timeout_seconds,
                poll_interval_seconds=args.poll_interval_seconds,
            )
            print_json("[device] recovered", recovered)
        elif args.expect_stage.strip():
            stage_item = wait_for_stage(
                client,
                args.device_id,
                args.expect_stage.strip(),
                after_id=latest_stage_id,
                timeout_seconds=args.timeout_seconds,
                poll_interval_seconds=args.poll_interval_seconds,
            )
            print_json("[debug-stage] matched", stage_item)
            if args.expect_version_unchanged:
                recovered = wait_for_reset_recovery(
                    client,
                    args.device_id,
                    initial_version=str(initial.get("firmware_version") or ""),
                    initial_partition=str(initial.get("running_partition") or ""),
                    timeout_seconds=args.timeout_seconds,
                    poll_interval_seconds=args.poll_interval_seconds,
                )
                print_json("[device] unchanged", recovered)
        else:
            final_row = wait_for_success(
                client,
                args.device_id,
                args.version,
                initial_partition=str(initial.get("running_partition") or ""),
                timeout_seconds=args.timeout_seconds,
                poll_interval_seconds=args.poll_interval_seconds,
            )
            print_json("[device] upgraded", final_row)
            if log_request_id is not None:
                upload = wait_for_log_upload(
                    client,
                    args.device_id,
                    log_request_id,
                    timeout_seconds=args.timeout_seconds,
                    poll_interval_seconds=args.poll_interval_seconds,
                )
                print_json("[log-upload] received", upload)
    finally:
        try:
            cleanup_rollout = client.delete(f"/api/v1/firmware-rollouts/{rollout_id}")
            print_json("[rollout] cleanup", cleanup_rollout)
        except Exception as exc:
            print(f"[warn] rollout cleanup failed: {exc}", file=sys.stderr)
        if log_request_id is not None:
            try:
                cleanup_request = client.delete(f"/api/v1/device-log-requests/{log_request_id}")
                print_json("[log-request] cleanup", cleanup_request)
            except HTTPError as exc:
                if exc.code == 404:
                    print(f"[log-request] cleanup skipped: request_id={log_request_id} already completed/cancelled")
                else:
                    print(f"[warn] log request cleanup failed: {exc}", file=sys.stderr)
            except Exception as exc:
                print(f"[warn] log request cleanup failed: {exc}", file=sys.stderr)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
