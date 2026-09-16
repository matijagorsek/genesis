# {{name}}

A JSON API on the Python standard library: routes in `ROUTES`, data in `items.json`, CORS on so a
web front end can call it. Tests call the routes directly.

    python3 app.py --port 5500        # serve
    python3 -m pytest -q              # test
    curl -X POST -d '{"text":"milk"}' http://127.0.0.1:5500/api/items
