#!/usr/bin/env python3
import sys
import time
import urllib.request
end = time.monotonic() + 240
while True:
    try:
        with urllib.request.urlopen(sys.argv[1], timeout=3) as response:
            if response.status == 200:
                break
    except OSError:
        pass
    if time.monotonic() >= end:
        raise SystemExit('Dependency timed out: ' + sys.argv[1])
    time.sleep(2)
