#!/usr/bin/env python3

import random
import subprocess
import sys
from time import sleep

# Change these vals as needed.
port = "/dev/ttyACM1"
cs_count = 31
vid = 0xC0
wait_range = range(5_000, 10_000 + 1, 1_000)
dip_range = range(227_000, 229_000 + 1, 500)

test_count = 4000

for _ in range(test_count):
    wait = random.choice(wait_range)
    dip = random.choice(dip_range)

    result = subprocess.run(
        [
            "../glitcher-pc/target/release/glitcher",
            "--port",
            port,
            "attack",
            "--chip-select-count",
            str(cs_count),
            "--vid",
            str(vid),
            "--wait-duration-ns",
            str(wait),
            "--dip-duration-ns",
            str(dip),
            "--spi-byte-count",
            "4096",
        ],
        check=False,
        capture_output=True,
        text=True,
    )

    if "target was not running" in result.stdout + result.stderr:
        subprocess.run(
            [
                "../glitcher-pc/target/release/glitcher",
                "--port",
                port,
                "press-power-button",
                "--duration-ms",
                "100",
            ],
            check=False,
        )
        sleep(0.5)
        continue

    status = "success" if result.returncode == 0 else "broken"
    print(f"({cs_count}, {vid}, {wait}, {dip}) => {status}", flush=True)

    if result.returncode == 0:
        print(result.stdout, end="")
        print(result.stderr, end="", file=sys.stderr)
        break
