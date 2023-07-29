use rplc::prelude::*;
use rppal::gpio::Gpio;
use std::time::Duration;

mod plc;

const GPIO_IN1: u8 = 1;
const GPIO_IN2: u8 = 2;

const GPIO_OUT1: u8 = 3;
const GPIO_OUT2: u8 = 4;

#[plc_program(loop = "500ms")]
fn pinroute() {
    let mut ctx = plc_context_mut!();
    ctx.out1 = ctx.in1;
    ctx.out2 = ctx.in2;
}

fn gpio_input_spawn() {
    let pin_in1 = Gpio::new().unwrap().get(GPIO_IN1).unwrap().into_input();
    let pin_in2 = Gpio::new().unwrap().get(GPIO_IN2).unwrap().into_input();
    rplc::tasks::spawn_input_loop(
        "gpio",
        Duration::from_millis(500),
        Duration::default(),
        move || {
            let in1 = pin_in1.is_high();
            let in2 = pin_in2.is_high();
            let mut ctx = plc_context_mut!();
            ctx.in1 = in1;
            ctx.in2 = in2;
        },
    );
}

fn gpio_output_spawn() {
    let mut pin_out1 = Gpio::new().unwrap().get(GPIO_OUT1).unwrap().into_output();
    let mut pin_out2 = Gpio::new().unwrap().get(GPIO_OUT2).unwrap().into_output();
    rplc::tasks::spawn_output_loop(
        "gpio",
        Duration::from_millis(500),
        Duration::default(),
        move || {
            let (out1, out2) = {
                let ctx = plc_context!();
                (ctx.out1, ctx.out2)
            };
            pin_out1.write(out1.into());
            pin_out2.write(out2.into());
        },
    );
}

fn main() {
    init_plc!();
    gpio_input_spawn();
    gpio_output_spawn();
    pinroute_spawn();
    run_plc!();
}
