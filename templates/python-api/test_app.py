"""The template's own test: the server starts and answers its health route. What the API should actually
do is decided by the job, so add those assertions here as you implement the routes."""
import app


def test_health_answers():
    status, body = app.dispatch("GET", "/api/health", None)
    assert status == 200 and body.get("ok") is True


def test_unknown_route_is_404():
    assert app.dispatch("GET", "/nope", None)[0] == 404
