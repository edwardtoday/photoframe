from __future__ import annotations

from typing import Any, Iterable


SEVERITY_RANK = {"info": 0, "success": 1, "warning": 2, "critical": 3}


def make_device_event(
    *,
    event_id: str,
    device_id: str,
    epoch: int,
    kind: str,
    severity: str,
    title: str,
    detail: str,
    source: str,
    metadata: dict[str, Any] | None = None,
) -> dict[str, Any]:
  normalized_severity = severity if severity in SEVERITY_RANK else "info"
  return {
      "id": event_id,
      "device_id": device_id,
      "epoch": max(0, int(epoch)),
      "kind": kind,
      "severity": normalized_severity,
      "severity_rank": SEVERITY_RANK[normalized_severity],
      "title": title,
      "detail": detail,
      "source": source,
      "metadata": metadata or {},
  }


def merge_device_events(events: Iterable[dict[str, Any]], limit: int) -> list[dict[str, Any]]:
  deduplicated: dict[str, dict[str, Any]] = {}
  for event in events:
    event_id = str(event.get("id") or "")
    if not event_id:
      continue
    existing = deduplicated.get(event_id)
    if existing is None or int(event.get("epoch") or 0) > int(existing.get("epoch") or 0):
      deduplicated[event_id] = event
  ordered = sorted(
      deduplicated.values(),
      key=lambda item: (int(item.get("epoch") or 0), int(item.get("severity_rank") or 0), str(item.get("id") or "")),
      reverse=True,
  )
  return ordered[: max(1, limit)]


def compact_device_events(events: Iterable[dict[str, Any]], limit: int) -> list[dict[str, Any]]:
  compacted: list[dict[str, Any]] = []
  for source_event in events:
    event = dict(source_event)
    previous = compacted[-1] if compacted else None
    can_group = (
        previous is not None
        and event.get("kind") == "photo_sent"
        and previous.get("kind") == event.get("kind")
        and previous.get("metadata", {}).get("delivery_status") == event.get("metadata", {}).get("delivery_status")
        and previous.get("metadata", {}).get("image_source") == event.get("metadata", {}).get("image_source")
    )
    if can_group:
      metadata = dict(previous.get("metadata") or {})
      count = int(metadata.get("group_count") or 1) + 1
      metadata["group_count"] = count
      previous["metadata"] = metadata
      base_title = str(previous.get("title") or "图片下发")
      previous["title"] = base_title.split(" ×", 1)[0] + f" × {count}"
      previous["detail"] = f"连续 {count} 条同类下发记录；最近一条：{source_event.get('detail') or '-'}"
      continue
    compacted.append(event)
    if len(compacted) >= max(1, limit):
      break
  return compacted[: max(1, limit)]
