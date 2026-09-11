#!/usr/bin/env python3
"""{{name}}: describe what it does here."""
import argparse
import sys


def main(argv=None):
    p = argparse.ArgumentParser(prog="{{name}}", description=__doc__)
    p.add_argument("items", nargs="*", help="inputs")
    p.add_argument("--json", action="store_true", help="machine-readable output")
    args = p.parse_args(argv)
    result = {"items": args.items, "count": len(args.items)}
    if args.json:
        import json
        print(json.dumps(result))
    else:
        print(f"{result['count']} item(s): {' '.join(args.items)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
