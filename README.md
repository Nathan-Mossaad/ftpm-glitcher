# Docs

Mostly missing, refer to `--help` of `glitcher` for now.

# Building

> [!NOTE]
> This project is only intended for use on Linux.

Install rustup & cargo:

[https://rust-lang.org/tools/install/](https://rust-lang.org/tools/install/)

## Pico firmware

Simple setup:

```bash
# Install pico specific target
rustup target add thumbv6m-none-eabi
# compile the firmware
cd glitcher-controller
cargo build -r
# Generating the uf2
cargo install elf2uf2-rs
elf2uf2-rs target/thumbv6m-none-eabi/release/glitcher-controller target/thumbv6m-none-eabi/release/glitcher-controller.uf2
# Now you can enter bootloader mode on the pico (by holding bootsel while plugging it in) and copy the uf2 file to it.
```

With debugging support:

You can use any [probe-rs](https://probe.rs/docs/getting-started/probe-setup/) compatible debugger, e.g. a PicoProbe (a second pico): [download](https://github.com/raspberrypi/debugprobe/releases) Wiring instructions: [wiring](https://mcuoneclipse.com/2022/09/17/picoprobe-using-the-raspberry-pi-pico-as-debug-probe/)

```bash
# Install pico specific target
rustup target add thumbv6m-none-eabi
# compile the firmware
cd glitcher-controller
# This will build, flash and then attach to the debug output of the pico
cargo run -r
```

## Cli tool

Direct compile and run

```bash
# compile the cli tool
cd glitcher-cli
cargo run -r -- --help
# Check versions on different serial port
cargo run -r -- --port /dev/ttyACM1 check-version
```

Alternatively you may wish to add `glitcher` to your path and use e.g. `glitcher generate-completions zsh` to generate completions for your shell.

## Helper scripts

Managed via [uv](https://docs.astral.sh/uv/getting-started/installation/)

> [!NOTE]
> The main project needs to already run

```bash
# Direcly modify helpers/determine-params.py and helpers/flash-pico.py to your needs

# For param determination
uv run helpers/determine-params.py
# Additionally get & print spi-tap data
uv run helpers/extract-data.py
```
