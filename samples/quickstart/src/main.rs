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
    {
        let mut ctx = plc_context_mut!();
        ctx.data[0].var1[0] = 5;
    }
    init_plc!();
    tempmon_spawn();
    run_plc!();
}
