#!/usr/bin/env python3
"""{{name}}: describe the job here."""
from pathlib import Path


def main():
    here = Path(__file__).resolve().parent
    print(f"{{name}} running in {here}")


if __name__ == "__main__":
    main()
