#!/usr/bin/env python3
"""
Night-shift timer for ez-sdr autonomous work session.
Start time is stored permanently in .claude/night_shift_start (not /tmp).
"""
import os, sys, time

START_FILE = os.path.join(os.path.dirname(os.path.abspath(__file__)), ".claude", "night_shift_start")
TARGET_SECS = 8 * 3600   # 8 hours

def _save(t):
    os.makedirs(os.path.dirname(START_FILE), exist_ok=True)
    with open(START_FILE, "w") as f:
        f.write(str(int(t)))

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
        print(f"Timer started at {time.ctime(t)}")
    else:
        print(f"Timer already running since {time.ctime(t)}")
    end = t + TARGET_SECS
    print(f"  Target end: {time.ctime(end)}")
    elapsed = time.time() - t
    remaining = max(0, TARGET_SECS - elapsed)
    print(f"  Elapsed: {elapsed/3600:.2f}h, Remaining: {remaining/3600:.2f}h")

def check():
    start = _load()
    if start is None:
        print("ERROR: Timer not started. Run: python3 timer.py init")
        return False
    elapsed = time.time() - start
    hours = elapsed / 3600.0
    remaining = max(0, TARGET_SECS - elapsed)
    if elapsed >= TARGET_SECS:
        print(f"DONE: {hours:.2f}h elapsed — 8-hour shift complete. Time to wrap up!")
        return False
    else:
        print(f"OK: {hours:.2f}h elapsed — {remaining/3600:.2f}h remaining")
        return True

def status():
    start = _load()
    if start is None:
        print("No active timer.")
        return
    elapsed = time.time() - start
    remaining = max(0, TARGET_SECS - elapsed)
    end_time = start + TARGET_SECS
    print(f"Start:     {time.ctime(start)}")
    print(f"End:       {time.ctime(end_time)}")
    print(f"Elapsed:   {elapsed/3600:.2f}h ({elapsed/60:.0f}m)")
    print(f"Remaining: {remaining/3600:.2f}h ({remaining/60:.0f}m)")
    pct = min(100, elapsed / TARGET_SECS * 100)
    bar = '#' * int(pct/5) + '-' * (20 - int(pct/5))
    print(f"Progress:  [{bar}] {pct:.0f}%")

if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else "status"
    if cmd == "init":
        init()
    elif cmd == "check":
        if not check():
            sys.exit(1)
    elif cmd == "status":
        status()
    elif cmd == "reset":
        if os.path.exists(START_FILE):
            os.remove(START_FILE)
            print("Timer reset.")
        else:
            print("No timer to reset.")
    else:
        print(f"Usage: timer.py [init|check|status|reset]")
