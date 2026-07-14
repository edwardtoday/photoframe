import sqlite3
import sys
import unittest
from pathlib import Path


SERVICE_ROOT = Path(__file__).resolve().parents[1]
if str(SERVICE_ROOT) not in sys.path:
  sys.path.insert(0, str(SERVICE_ROOT))

from app.admin_v2.dashboard import build_admin_dashboard
from app.admin_v2.timeline import load_device_timeline
from app.domains.events import compact_device_events, make_device_event
from app.domains.health import assess_device_health


class DeviceHealthAssessmentTests(unittest.TestCase):

  def test_overdue_wakeup_has_priority_over_low_battery(self) -> None:
    health = assess_device_health(
        {
            "device_id": "pf-demo",
            "last_seen_epoch": 1_000,
            "next_wakeup_epoch": 2_000,
            "poll_interval_seconds": 3_600,
            "battery_percent": 8,
            "battery_mv": 3_500,
            "vbus_good": 0,
            "charging": 0,
        },
        now_epoch=30_000,
    )
    self.assertEqual(health["status"], "critical")
    self.assertEqual(health["code"], "wake_overdue")

  def test_device_sleeping_before_next_wakeup_is_healthy(self) -> None:
    health = assess_device_health(
        {
            "device_id": "pf-demo",
            "last_seen_epoch": 10_000,
            "next_wakeup_epoch": 14_000,
            "poll_interval_seconds": 3_600,
            "battery_percent": 76,
            "battery_mv": 4_020,
            "vbus_good": 0,
            "charging": 0,
            "failure_count": 0,
        },
        now_epoch=11_000,
    )
    self.assertEqual(health["status"], "sleeping")
    self.assertEqual(health["code"], "sleeping_as_expected")

  def test_ota_waiting_for_battery_is_explained(self) -> None:
    health = assess_device_health(
        {
            "device_id": "pf-demo",
            "last_seen_epoch": 10_000,
            "next_wakeup_epoch": 14_000,
            "poll_interval_seconds": 3_600,
            "battery_percent": 25,
            "battery_mv": 3_750,
            "vbus_good": 0,
            "charging": 0,
        },
        now_epoch=11_000,
        active_rollout={"version": "0.2.0", "min_battery_percent": 50, "requires_vbus": False},
    )
    self.assertEqual(health["status"], "warning")
    self.assertEqual(health["code"], "ota_blocked_low_battery")

  def test_unconfirmed_delivery_is_not_reported_as_healthy(self) -> None:
    health = assess_device_health(
        {
            "device_id": "pf-demo",
            "last_seen_epoch": 20_000,
            "next_wakeup_epoch": 25_000,
            "poll_interval_seconds": 3_600,
            "battery_percent": 80,
            "battery_mv": 4_080,
            "vbus_good": 0,
            "charging": 0,
        },
        now_epoch=21_000,
        latest_delivery={"status": "sent", "issued_epoch": 10_000},
    )
    self.assertEqual(health["code"], "display_unconfirmed")


class DeviceTimelineTests(unittest.TestCase):

  def setUp(self) -> None:
    self.conn = sqlite3.connect(":memory:")
    self.conn.row_factory = sqlite3.Row
    self.conn.executescript(
        """
        CREATE TABLE publish_history (
          id INTEGER PRIMARY KEY,
          device_id TEXT NOT NULL,
          issued_epoch INTEGER NOT NULL,
          source TEXT NOT NULL,
          image_url TEXT NOT NULL DEFAULT '',
          override_id INTEGER,
          poll_after_seconds INTEGER NOT NULL DEFAULT 3600,
          valid_until_epoch INTEGER NOT NULL DEFAULT 0,
          status TEXT NOT NULL,
          displayed_epoch INTEGER NOT NULL DEFAULT 0,
          dither_algorithm TEXT NOT NULL DEFAULT '',
          displayed_image_url TEXT NOT NULL DEFAULT '',
          displayed_image_sha256 TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE device_config_plans (
          id INTEGER PRIMARY KEY,
          device_id TEXT NOT NULL,
          created_epoch INTEGER NOT NULL,
          note TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE photo_deliveries (
          id INTEGER PRIMARY KEY,
          photo_asset_id INTEGER NOT NULL,
          render_variant_id INTEGER NOT NULL,
          override_id INTEGER NOT NULL,
          device_id TEXT NOT NULL,
          duration_minutes INTEGER NOT NULL DEFAULT 1440,
          note TEXT NOT NULL DEFAULT '',
          requested_epoch INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE device_log_upload_requests (
          id INTEGER PRIMARY KEY,
          device_id TEXT NOT NULL,
          created_epoch INTEGER NOT NULL,
          reason TEXT NOT NULL DEFAULT '',
          status TEXT NOT NULL,
          completed_epoch INTEGER NOT NULL DEFAULT 0,
          uploaded_line_count INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE firmware_artifacts (
          id INTEGER PRIMARY KEY,
          version TEXT NOT NULL,
          asset_sha256 TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE firmware_rollouts (
          id INTEGER PRIMARY KEY,
          device_id TEXT NOT NULL,
          artifact_id INTEGER NOT NULL,
          min_battery_percent INTEGER NOT NULL,
          requires_vbus INTEGER NOT NULL,
          note TEXT NOT NULL DEFAULT '',
          enabled INTEGER NOT NULL,
          created_epoch INTEGER NOT NULL
        );
        CREATE TABLE device_debug_stages (
          id INTEGER PRIMARY KEY,
          device_id TEXT NOT NULL,
          stage TEXT NOT NULL,
          stage_epoch INTEGER NOT NULL
        );
        """
    )

  def tearDown(self) -> None:
    self.conn.close()

  def test_timeline_merges_and_orders_multiple_sources(self) -> None:
    self.conn.execute(
        """
        INSERT INTO publish_history (
          id, device_id, issued_epoch, source, status, displayed_epoch, dither_algorithm
        ) VALUES (1, 'pf-demo', 100, 'daily', 'displayed', 120, 'sierra')
        """
    )
    self.conn.execute(
        "INSERT INTO device_config_plans VALUES (2, 'pf-demo', 110, 'new wifi')"
    )
    self.conn.execute(
        "INSERT INTO photo_deliveries VALUES (5, 9, 10, 11, 'pf-demo', 1440, 'family', 115)"
    )
    self.conn.execute(
        "INSERT INTO firmware_artifacts (id, version) VALUES (3, '0.2.0')"
    )
    self.conn.execute(
        "INSERT INTO firmware_rollouts VALUES (4, 'pf-demo', 3, 50, 1, 'test ota', 1, 130)"
    )
    self.conn.commit()

    events = load_device_timeline(self.conn, "pf-demo", limit=20)

    self.assertEqual(events[0]["kind"], "ota_rollout_created")
    self.assertEqual(events[1]["kind"], "photo_displayed")
    self.assertEqual(events[2]["kind"], "photo_delivery_requested")
    self.assertEqual(events[3]["kind"], "config_published")
    self.assertEqual(events[4]["kind"], "photo_sent")

  def test_timeline_before_epoch_filters_newer_events(self) -> None:
    self.conn.execute(
        """
        INSERT INTO publish_history (
          id, device_id, issued_epoch, source, status, displayed_epoch, dither_algorithm
        ) VALUES (1, 'pf-demo', 100, 'daily', 'sent', 0, 'sierra')
        """
    )
    self.conn.execute(
        "INSERT INTO device_debug_stages VALUES (2, 'pf-demo', 'after_fetch', 200)"
    )
    self.conn.commit()

    events = load_device_timeline(self.conn, "pf-demo", limit=20, before_epoch=150)

    self.assertEqual(len(events), 1)
    self.assertEqual(events[0]["kind"], "photo_sent")

  def test_dashboard_removes_token_query_from_image_reference(self) -> None:
    self.conn.execute(
        """
        INSERT INTO publish_history (
          id, device_id, issued_epoch, source, image_url, poll_after_seconds,
          valid_until_epoch, status, displayed_epoch, dither_algorithm
        ) VALUES (
          1, 'pf-demo', 100, 'daily',
          'https://example.test/api/v1/assets/photo.bmp?device_id=pf-demo&token=secret',
          3600, 7200, 'sent', 0, 'sierra'
        )
        """
    )
    self.conn.commit()

    dashboard = build_admin_dashboard(
        self.conn,
        devices=[
            {
                "device_id": "pf-demo",
                "last_seen_epoch": 110,
                "next_wakeup_epoch": 5000,
                "poll_interval_seconds": 3600,
                "battery_percent": 80,
                "battery_mv": 4080,
                "vbus_good": 0,
                "charging": 0,
            }
        ],
        requested_device_id="pf-demo",
        now_epoch=200,
        event_limit=10,
        service={"app_version": "test"},
    )

    self.assertEqual(dashboard["current_delivery"]["image_reference"], "/api/v1/assets/photo.bmp")
    self.assertNotIn("secret", str(dashboard))

  def test_dashboard_event_compaction_groups_repeated_photo_sends(self) -> None:
    events = [
        make_device_event(
            event_id=f"publish:{index}:sent",
            device_id="pf-demo",
            epoch=100 - index,
            kind="photo_sent",
            severity="info",
            title="已下发日常照片",
            detail="当前状态 sent。",
            source="publish_history",
            metadata={"delivery_status": "sent", "image_source": "daily"},
        )
        for index in range(3)
    ]

    compacted = compact_device_events(events, 10)

    self.assertEqual(len(compacted), 1)
    self.assertEqual(compacted[0]["metadata"]["group_count"], 3)
    self.assertIn("× 3", compacted[0]["title"])


if __name__ == "__main__":
  unittest.main()
