"""Tests for the API: the routes are called directly, no server needed. Run: python3 -m pytest -q"""
import os, tempfile
import app


def setup_function():
    app.DATA = os.path.join(tempfile.mkdtemp(), "items.json")


def test_health():
    assert app.dispatch("GET", "/api/health", None) == (200, {"ok": True})


def test_add_list_delete():
    assert app.dispatch("GET", "/api/items", None) == (200, [])
    status, item = app.dispatch("POST", "/api/items", {"text": "milk"})
    assert status == 201 and item["id"] == 1
    assert app.dispatch("GET", "/api/items", None)[1] == [item]
    assert app.dispatch("DELETE", "/api/items/1", None) == (200, {"deleted": 1})
    assert app.dispatch("DELETE", "/api/items/1", None)[0] == 404


def test_validation_and_unknown_route():
    assert app.dispatch("POST", "/api/items", {"text": "  "})[0] == 400
    assert app.dispatch("GET", "/nope", None)[0] == 404
