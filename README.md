<h2>
  rPLC - PLC programming for Linux in Rust
  <a href="https://crates.io/crates/rplc"><img alt="crates.io page" src="https://img.shields.io/crates/v/rplc.svg"></img></a>
  <a href="https://docs.rs/rplc"><img alt="docs.rs page" src="https://docs.rs/rplc/badge.svg"></img></a>
</h2>

THIS IS A LEGACY REPOSITORY. CONSIDER SWITCHING TO [RoboPLC](https://github.com/eva-ics/roboplc) INSTEAD.

rPLC project allows to write PLC programs for Linux systems in Rust using
classical PLC programming approach.

rPLC supports Modbus and OPC-UA input/output protocols out-of-the-box and can
be easily extended with custom I/O as well.

rPLC is a part of [EVA ICS](https://www.eva-ics.com) open-source industrial
automation eco-system.

## A quick example

```rust,ignore
use rplc::prelude::*;

mod plc;

#[plc_program(loop = "200ms")]
fn tempmon() {
    let mut ctx = plc_context_mut!();
    if ctx.temperature > 30.0 {
        ctx.fan = true;
    } else if ctx.temperature < 25.0 {
        ctx.fan = false;
    }
}

fn main() {
    init_plc!();
    tempmon_spawn();
    run_plc!();
}
```

## Technical documentation

Available at <https://info.bma.ai/en/actual/rplc/index.html>
