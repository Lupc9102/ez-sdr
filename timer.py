#!/usr/bin/env python3
"""Timer script: tracks elapsed time, ensures 2h-2.5h work session.
Persists start time in /tmp/ezsdr_timer_start."""
import os, sys, time

START_FILE = "/tmp/ezsdr_timer_start"
TARGET_MIN = 300
MAX_MIN = 330

def _save(t):
    with open(START_FILE, "w") as f:
        f.write(str(t))

def _load():
    try:
        with open(START_FILE) as f:
            return float(f.read().strip())
    except (FileNotFoundError, ValueError):
        return None

def init():
    t = _load()
    if t is None:
        t = time.time()
        _save(t)
    start = t
    print(f"Timer started at {time.ctime(start)}")
    print(f"Need {TARGET_MIN}min min, {MAX_MIN}min max")
    print(f"  Target window: {time.ctime(start + TARGET_MIN*60)} — {time.ctime(start + MAX_MIN*60)}")
    print(f"Run: python3 timer.py check  — after each task")

def check():
    start = _load()
    if start is None:
        print("Timer not started. Run: python3 timer.py init")
        return True
    elapsed = time.time() - start
    mins = elapsed / 60.0
    if mins < TARGET_MIN:
        remaining = TARGET_MIN - mins
        print(f"⏱  {mins:.0f}m elapsed — {remaining:.0f}m remaining (need {TARGET_MIN}min min)")
        return True
    elif mins >= MAX_MIN:
        remaining = MAX_MIN - mins
        print(f"⏱  {mins:.0f}m elapsed — OVER {MAX_MIN}min limit ({remaining:.0f}m over), wrapping up.")
        return False
    else:
        remaining = MAX_MIN - mins
        print(f"⏱  {mins:.0f}m elapsed — in the green zone ({remaining:.0f}m left of {MAX_MIN}min cap)")
        return True

if __name__ == "__main__":
    if len(sys.argv) > 1:
        if sys.argv[1] == "init":
            init()
        elif sys.argv[1] == "check":
            if not check():
                sys.exit(1)
        elif sys.argv[1] == "reset":
            if os.path.exists(START_FILE):
                os.remove(START_FILE)
                print("Timer reset.")
            else:
                print("No timer to reset.")
    else:
        init()
