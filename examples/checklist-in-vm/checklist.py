#!/usr/bin/env python3
"""checklist: a small checklist that saves to a JSON file.

Examples:
  python3 checklist.py list
  python3 checklist.py add Buy milk
  python3 checklist.py done 1
  python3 checklist.py undone 1
  python3 checklist.py remove 1
  python3 checklist.py clear            # remove done items
  python3 checklist.py clear --all      # remove everything
  python3 checklist.py --file todo.json add Call mom
"""
import argparse
import json
import os
import sys
import tempfile

DEFAULT_FILE = os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "checklist.json"
)


def load(path):
    """Read the checklist file; a missing file means an empty checklist."""
    if not os.path.exists(path):
        return {"next_id": 1, "items": []}
    try:
        with open(path, "r", encoding="utf-8") as f:
            data = json.load(f)
    except (json.JSONDecodeError, OSError) as e:
        sys.exit(f"error: could not read {path}: {e}")
    if not isinstance(data, dict) or not isinstance(data.get("items"), list):
        sys.exit(f"error: {path} is not a valid checklist file")
    ids = [i["id"] for i in data["items"] if isinstance(i.get("id"), int)]
    stored = data.get("next_id")
    if not isinstance(stored, int) or stored <= max(ids, default=0):
        data["next_id"] = max(ids, default=0) + 1
    return data


def save(path, data):
    """Write atomically: temp file in the same directory, then replace."""
    directory = os.path.dirname(os.path.abspath(path)) or "."
    fd, tmp = tempfile.mkstemp(dir=directory, prefix=".checklist-", suffix=".tmp")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=2, ensure_ascii=False)
            f.write("\n")
        os.replace(tmp, path)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def find_item(data, item_id):
    for item in data["items"]:
        if item.get("id") == item_id:
            return item
    return None


def cmd_list(args):
    data = load(args.file)
    if not data["items"]:
        print("Checklist is empty. Add something with: checklist.py add <text>")
        return 0
    done = sum(1 for i in data["items"] if i.get("done"))
    for item in data["items"]:
        mark = "x" if item.get("done") else " "
        print(f"[{mark}] {item['id']}: {item['text']}")
    print(f"\n{done}/{len(data['items'])} done")
    return 0


def cmd_add(args):
    data = load(args.file)
    text = " ".join(args.text)
    item = {"id": data["next_id"], "text": text, "done": False}
    data["items"].append(item)
    data["next_id"] += 1
    save(args.file, data)
    print(f"Added {item['id']}: {item['text']}")
    return 0


def cmd_done(args, mark_done):
    data = load(args.file)
    item = find_item(data, args.id)
    if item is None:
        print(f"error: no item with id {args.id}", file=sys.stderr)
        return 1
    item["done"] = mark_done
    save(args.file, data)
    print(f"{'Done' if mark_done else 'Undone'} {item['id']}: {item['text']}")
    return 0


def cmd_remove(args):
    data = load(args.file)
    item = find_item(data, args.id)
    if item is None:
        print(f"error: no item with id {args.id}", file=sys.stderr)
        return 1
    data["items"].remove(item)
    save(args.file, data)
    print(f"Removed {item['id']}: {item['text']}")
    return 0


def cmd_clear(args):
    data = load(args.file)
    if args.all:
        removed = len(data["items"])
        data["items"] = []
    else:
        before = len(data["items"])
        data["items"] = [i for i in data["items"] if not i.get("done")]
        removed = before - len(data["items"])
    save(args.file, data)
    print(f"Cleared {removed} item(s)")
    return 0


def main(argv=None):
    p = argparse.ArgumentParser(
        prog="checklist",
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    p.add_argument(
        "--file",
        default=DEFAULT_FILE,
        help=f"checklist file (default: {DEFAULT_FILE})",
    )
    sub = p.add_subparsers(dest="command")

    sub.add_parser("list", help="show the checklist")

    sp = sub.add_parser("add", help="add an item")
    sp.add_argument("text", nargs="+", help="item text")

    sp = sub.add_parser("done", help="mark an item done")
    sp.add_argument("id", type=int, help="item id")

    sp = sub.add_parser("undone", help="mark an item not done")
    sp.add_argument("id", type=int, help="item id")

    sp = sub.add_parser("remove", help="remove an item")
    sp.add_argument("id", type=int, help="item id")

    sp = sub.add_parser("clear", help="remove done items (or all with --all)")
    sp.add_argument("--all", action="store_true", help="remove every item")

    args = p.parse_args(argv)
    if args.command is None:
        args.command = "list"

    handlers = {
        "list": lambda a: cmd_list(a),
        "add": lambda a: cmd_add(a),
        "done": lambda a: cmd_done(a, True),
        "undone": lambda a: cmd_done(a, False),
        "remove": lambda a: cmd_remove(a),
        "clear": lambda a: cmd_clear(a),
    }
    return handlers[args.command](args)


if __name__ == "__main__":
    sys.exit(main())
