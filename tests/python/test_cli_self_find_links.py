from __future__ import annotations

import json
from pathlib import Path

from mohaus._cli import _wheelhouse_from_direct_url_text


def test_wheelhouse_from_direct_url_text_accepts_local_wheel(tmp_path: Path) -> None:
  wheel = tmp_path / "mohaus-0.1.0-cp311-abi3-macosx_14_0_arm64.whl"
  wheel.write_bytes(b"")

  text = json.dumps({"url": wheel.as_uri(), "archive_info": {}})

  assert _wheelhouse_from_direct_url_text(text) == str(tmp_path)


def test_wheelhouse_from_direct_url_text_rejects_editable_source(tmp_path: Path) -> None:
  text = json.dumps({"url": tmp_path.as_uri(), "dir_info": {"editable": True}})

  assert _wheelhouse_from_direct_url_text(text) is None
